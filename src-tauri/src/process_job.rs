//! Shared Windows Job Object wrapper enforcing KILL_ON_JOB_CLOSE so spawned
//! child processes never outlive their owner, even on abnormal exit paths.
//!
//! Used by both the Tailscale funnel sidecar lifecycle
//! (`connection::sidecar`) and the bounded command runner (`execution::runner`)
//! so every child tree dies with its owner.

use std::os::windows::io::AsRawHandle;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};

/// Owns a job object handle whose close kills every assigned process.
///
/// Not `Clone`: duplicating the handle would create a second owner and
/// weaken the kill-on-drop guarantee.
pub(crate) struct JobHandle {
    handle: HANDLE,
}

impl JobHandle {
    /// Creates an unnamed job object configured with KILL_ON_JOB_CLOSE.
    pub fn create() -> Result<Self, String> {
        // SAFETY: no attributes and no name requested; the returned handle is
        // either valid or the call returns Err.
        let handle = unsafe { CreateJobObjectW(None, None) }
            .map_err(|e| format!("CreateJobObjectW failed: {e}"))?;

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `handle` is valid from the successful CreateJobObjectW call
        // above; `limits` is a fully initialized struct of exactly the class
        // named by JobObjectExtendedLimitInformation, passed as read-only.
        unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|e| format!("SetInformationJobObject failed: {e}"))?;

        Ok(Self { handle })
    }

    /// Adds a spawned child process to the job.
    pub fn assign(&self, child: &std::process::Child) -> Result<(), String> {
        // SAFETY: `handle` is valid for this JobHandle's lifetime; the raw
        // process handle from the child is valid while `child` is alive
        // (it is not exited/killed here).
        unsafe { AssignProcessToJobObject(self.handle, HANDLE(child.as_raw_handle() as *mut _)) }
            .map_err(|e| format!("AssignProcessToJobObject failed: {e}"))
    }

    /// Adds an already-spawned process to the job by PID. This is used by
    /// transports that own their child handle internally (for example rmcp's
    /// TokioChildProcess).
    pub(crate) fn assign_pid(&self, pid: u32) -> Result<(), String> {
        // SAFETY: OpenProcess is called with the minimal rights needed to
        // assign/terminate/query the child. The returned handle is closed on
        // every path below.
        let process = unsafe {
            OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            )
        }
        .map_err(|e| format!("OpenProcess({pid}) failed: {e}"))?;
        let result = unsafe { AssignProcessToJobObject(self.handle, process) }
            .map_err(|e| format!("AssignProcessToJobObject({pid}) failed: {e}"));
        let _ = unsafe { CloseHandle(process) };
        result
    }

    fn close(&mut self) {
        if !self.handle.is_invalid() {
            // SAFETY: `handle` was obtained from CreateJobObjectW and closed
            // exactly once here; `close` runs at most once per instance
            // (Drop takes &mut self and invalidates afterwards).
            let ok = unsafe { CloseHandle(self.handle) };
            debug_assert!(ok.is_ok(), "CloseHandle failed");
            self.handle = HANDLE(std::ptr::null_mut());
        }
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        self.close();
    }
}

// SAFETY: HANDLE is an owning, non-thread-affine kernel object handle; moving
// the struct between threads transfers sole ownership and never enables
// concurrent use of the same handle value.
unsafe impl Send for JobHandle {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Best-effort liveness probe for a PID: a dead PID fails to open with
    /// query-limited access.
    fn process_exists(pid: u32) -> bool {
        // SAFETY: pid comes from a live Child earlier in the test; opening
        // with query-limited access only and closing immediately after.
        let opened = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
        match opened {
            Ok(h) => {
                // SAFETY: h is valid from the successful OpenProcess above.
                let _ = unsafe { CloseHandle(h) };
                true
            }
            // A dead PID fails to open (invalid parameter / not found).
            Err(_) => false,
        }
    }

    #[test]
    fn dropping_job_kills_assigned_process() {
        let job = JobHandle::create().expect("create job");

        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sleeper");
        let pid = child.id();
        assert!(
            process_exists(pid),
            "sleeper should be running before assign"
        );

        job.assign(&child).expect("assign sleeper to job");

        // Drop the job; KILL_ON_JOB_CLOSE must terminate the sleeper even
        // though its 60-second sleep would otherwise keep it alive.
        drop(job);
        // Wait for termination, then release our own process handle: while
        // any handle stays open, OpenProcess(pid) keeps succeeding even for
        // a dead process, so it must not be held during the liveness poll.
        let _ = child.kill(); // no-op if the job already killed it
        let _ = child.wait();
        drop(child);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if !process_exists(pid) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("sleeper pid {pid} still alive 5s after job drop");
    }

    #[test]
    fn dropping_job_kills_pid_assigned_process() {
        let job = JobHandle::create().expect("create job");
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sleeper");
        let pid = child.id();
        job.assign_pid(pid).expect("assign sleeper by pid");
        drop(job);
        let _ = child.wait();
        drop(child);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if !process_exists(pid) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("sleeper pid {pid} still alive 5s after PID job assignment");
    }
}

//! Lifecycle management for the `tunnel-client` sidecar process.
//!
//! The sidecar bridges the local MCP server (`http://127.0.0.1:{port}/mcp`)
//! to a remote control plane. We own the child process: spawn on configure /
//! startup, kill on stop / exit, and broadcast status transitions to the UI
//! via the `"tunnel-status"` event.
//!
//! Status honesty note: we emit `Connected` right after a successful spawn
//! without verifying the control-plane handshake. Real health polling (the
//! sidecar exposes `--health.listen-addr` / `--health.url-file`) is future
//! work; until then `Connected` means "process running", nothing more.
//!
//! Known limitation (MVP): teardown only runs on the normal exit path
//! (`RunEvent::ExitRequested`). If the app hard-crashes or is force-killed,
//! the sidecar is orphaned and keeps bridging. A Windows Job Object (kill-on-
//! close) or equivalent OS-level process binding is the future mitigation.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

/// Sidecar file name as produced by Tauri's `externalBin` resolution
/// (bundle target triple appended to `binaries/tunnel-client`).
const SIDECAR_FILE: &str = "tunnel-client-x86_64-pc-windows-msvc.exe";

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelStatus {
    #[default]
    Stopped,
    Connecting,
    Connected,
    Error(String),
}

#[derive(Default)]
pub struct TunnelHandle {
    pub child: Mutex<Option<Child>>,
    pub status: Mutex<TunnelStatus>,
}

/// Locates the sidecar executable: bundled builds place `externalBin`
/// artifacts next to the main executable; in dev (`cargo run`) fall back to
/// the source-tree `src-tauri/binaries/` checkout.
fn sidecar_path() -> PathBuf {
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let candidate = exe_dir.join(SIDECAR_FILE);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(SIDECAR_FILE)
}

/// Builds the `tunnel-client run` argument list. The API key itself is passed
/// via the `TUNNEL_CLIENT_API_KEY` environment variable (see `spawn`); the
/// flag only carries an `env:` reference so the secret never hits argv.
fn build_args(tunnel_id: &str, port: u16) -> Vec<String> {
    vec![
        "run".into(),
        format!("--control-plane.tunnel-id={tunnel_id}"),
        "--control-plane.api-key=env:TUNNEL_CLIENT_API_KEY".into(),
        format!("--mcp.server-url=url=http://127.0.0.1:{port}/mcp"),
    ]
}

fn set_status(app: &AppHandle, status: TunnelStatus) {
    let handle = app.state::<TunnelHandle>();
    *handle.status.lock().unwrap() = status.clone();
    let _ = app.emit("tunnel-status", &status);
}

/// Spawns the sidecar and stores it in [`TunnelHandle`]. Emits
/// `Connecting` before launch and `Connected` (or `Error`) after.
pub fn spawn(app: &AppHandle, tunnel_id: &str, api_key: &str, port: u16) -> Result<(), String> {
    set_status(app, TunnelStatus::Connecting);
    let child = Command::new(sidecar_path())
        .args(build_args(tunnel_id, port))
        .env("TUNNEL_CLIENT_API_KEY", api_key)
        .stdin(Stdio::null())
        // Piped-but-never-drained streams deadlock once the pipe buffer
        // fills; the sidecar logs are not consumed, so discard them.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            let msg = format!("failed to spawn tunnel-client: {e}");
            set_status(app, TunnelStatus::Error(msg.clone()));
            msg
        })?;
    let handle = app.state::<TunnelHandle>();
    *handle.child.lock().unwrap() = Some(child);
    // See module docs: no health verification yet.
    set_status(app, TunnelStatus::Connected);
    Ok(())
}

/// Kills the sidecar if running (idempotent) and marks the status `Stopped`.
pub fn stop(app: &AppHandle) {
    app.state::<TunnelHandle>().kill();
    set_status(app, TunnelStatus::Stopped);
}

impl TunnelHandle {
    /// Terminates the tracked child, if any. Safe to call repeatedly.
    pub fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_target_local_mcp_server_with_env_key_reference() {
        let args = build_args("t-123", 8123);
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--control-plane.tunnel-id=t-123".to_string()));
        assert!(args.contains(&"--control-plane.api-key=env:TUNNEL_CLIENT_API_KEY".to_string()));
        assert!(
            args.contains(&"--mcp.server-url=url=http://127.0.0.1:8123/mcp".to_string()),
            "args were {args:?}"
        );
    }

    #[test]
    fn status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&TunnelStatus::Stopped).unwrap(),
            "\"stopped\""
        );
        assert_eq!(
            serde_json::to_string(&TunnelStatus::Error("boom".into())).unwrap(),
            "{\"error\":\"boom\"}"
        );
    }

    /// Windows-only: the long-running stub is a PowerShell sleeper and the
    /// liveness probe uses `tasklist`.
    #[cfg(windows)]
    #[test]
    fn kill_terminates_tracked_child() {
        let child = Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let handle = TunnelHandle {
            child: Mutex::new(Some(child)),
            status: Mutex::new(TunnelStatus::Connecting),
        };
        assert!(is_alive(pid));
        handle.kill();
        // kill() must be idempotent and safe on an emptied slot.
        handle.kill();
        assert!(!is_alive(pid));
    }

    #[cfg(windows)]
    fn is_alive(pid: u32) -> bool {
        // waitpid-free probe: give the OS a beat to reap after wait().
        std::thread::sleep(std::time::Duration::from_millis(200));
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

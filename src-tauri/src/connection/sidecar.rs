//! Tailscale funnel sidecar process: spawn, NDJSON event parsing, secret
//! redaction, and kill-tree teardown via a KILL_ON_JOB_CLOSE job object.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::sync::mpsc;

use crate::process_job::JobHandle;

/// Target filename of the Go funnel sidecar, mirroring the tunnel-client
/// dev/bundled fallback convention in `tunnel.rs`.
const SIDECAR_FILE: &str = "tailscale-funnel-sidecar-x86_64-pc-windows-msvc.exe";

/// Cap on retained stderr diagnostics lines.
const DIAGNOSTICS_CAP: usize = 200;
/// Marker substituted for every scrubbed secret in diagnostics.
const REDACTED: &str = "[REDACTED]";
/// Login-URL prefix; anything from here to the next whitespace is a secret
/// device-auth link and gets scrubbed.
const LOGIN_URL_PREFIX: &str = "https://login.tailscale.com/a/";

#[derive(Debug)]
pub(crate) struct SidecarSpawnError {
    /// Stable machine-readable code, e.g. `SIDECAR_NOT_FOUND`.
    #[allow(dead_code)] // read by Task 10's ConnectionError conversion
    pub code: String,
    /// Human-safe message; never contains the capability token.
    #[allow(dead_code)] // read by Task 10's ConnectionError conversion
    pub message: String,
}

// TODO(Task 10): convert SidecarSpawnError into ConnectionError once that
// type exists; codes SIDECAR_NOT_FOUND / SIDECAR_SPAWN_FAILED map 1:1.

/// Configuration for spawning the funnel sidecar. Deliberately not `Debug`:
/// it carries the capability token.
pub struct SidecarConfig {
    pub local_port: u16,
    pub state_dir: PathBuf,
    pub hostname: String,
    pub capability_token: String,
}

/// One NDJSON stdout event from the sidecar. Unknown/malformed lines never
/// reach this enum — `parse_event` returns `None` for those instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarEvent {
    Starting,
    AuthRequired { url: String },
    Authenticated,
    FunnelListening { fqdn: String },
    Error { code: String, message: String },
    Stopped,
}

/// Parses one NDJSON line. Returns `None` for malformed JSON or unknown event
/// types so the reader thread can route them to diagnostics without panicking.
///
/// Defense in depth, enforced here at the parse boundary:
/// - `auth_required` URLs not starting with the Tailscale login prefix are
///   dropped (routed to diagnostics by the caller like any malformed line);
/// - error messages pass through [`redact`] so the capability token can never
///   reach UI-visible state via a sidecar echo.
pub fn parse_event(line: &str, token: &str) -> Option<SidecarEvent> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    match value.get("type")?.as_str()? {
        "starting" => Some(SidecarEvent::Starting),
        "auth_required" => {
            let url = value.get("url")?.as_str()?.to_string();
            if url.starts_with(LOGIN_URL_PREFIX) {
                Some(SidecarEvent::AuthRequired { url })
            } else {
                None
            }
        }
        "authenticated" => Some(SidecarEvent::Authenticated),
        "funnel_listening" => Some(SidecarEvent::FunnelListening {
            fqdn: value.get("fqdn")?.as_str()?.to_string(),
        }),
        "error" => Some(SidecarEvent::Error {
            code: value
                .get("code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message: redact(
                value
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
                token,
            ),
        }),
        "stopped" => Some(SidecarEvent::Stopped),
        _ => None,
    }
}

/// Scrubs the capability token and any login device-auth URL from raw
/// sidecar output before it is stored or shown anywhere.
fn redact(text: &str, token: &str) -> String {
    let mut out = if token.is_empty() {
        text.to_string()
    } else {
        text.replace(token, REDACTED)
    };
    // Truncate each login URL (prefix through next whitespace) to REDACTED.
    while let Some(pos) = out.find(LOGIN_URL_PREFIX) {
        let rest = &out[pos + LOGIN_URL_PREFIX.len()..];
        let end = rest
            .find(char::is_whitespace)
            .map(|i| pos + LOGIN_URL_PREFIX.len() + i)
            .unwrap_or(out.len());
        out.replace_range(pos..end, REDACTED);
    }
    out
}

/// Resolves the sidecar binary path: bundled next to the current executable,
/// falling back to `<manifest>/binaries` for dev runs (same shape as
/// `tunnel.rs::sidecar_path`).
fn sidecar_path() -> Result<PathBuf, SidecarSpawnError> {
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let candidate = exe_dir.join(SIDECAR_FILE);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(SIDECAR_FILE);
    if fallback.is_file() {
        return Ok(fallback);
    }
    Err(SidecarSpawnError {
        code: "SIDECAR_NOT_FOUND".into(),
        message: format!("funnel sidecar binary not found ({SIDECAR_FILE})"),
    })
}

/// Bounded ring of redacted sidecar output lines kept for user-facing
/// diagnostics. Shared between the stdout/stderr reader threads via `Arc`.
struct DiagnosticsRing {
    lines: std::sync::Mutex<VecDeque<String>>,
    cap: usize,
}

impl DiagnosticsRing {
    fn new(cap: usize) -> Self {
        Self {
            lines: std::sync::Mutex::new(VecDeque::with_capacity(cap)),
            cap,
        }
    }

    fn push(&self, line: String) {
        let mut q = self.lines.lock().unwrap();
        if q.len() == self.cap {
            q.pop_front();
        }
        q.push_back(line);
    }

    fn snapshot(&self) -> Vec<String> {
        self.lines.lock().unwrap().iter().cloned().collect()
    }
}

/// A running sidecar child, its job object, and the diagnostics buffer.
///
/// The event receiver is returned alongside from [`spawn_sidecar`]; events on
/// it are best-effort — dropping it does not affect the child.
// `stop` is consumed by Task 10's manager; until then the lint fires because
// SidecarSpawnError is crate-private while the method is pub.
#[allow(private_interfaces)]
pub struct SidecarProcess {
    child: Option<std::process::Child>,
    job: Option<JobHandle>,
    diagnostics: std::sync::Arc<DiagnosticsRing>,
    stopped: bool,
}

impl SidecarProcess {
    /// Kills the child and tears down the job (killing the whole process
    /// tree). Idempotent.
    // SidecarSpawnError is crate-private until Task 10 introduces
    // ConnectionError; the lint fires on these pub signatures meanwhile.
    #[allow(private_interfaces)]
    pub async fn stop(&mut self) -> Result<(), SidecarSpawnError> {
        self.kill_child()?;
        // Dropping the job handle is what kills remaining tree members.
        drop(self.job.take());
        Ok(())
    }

    fn kill_child(&mut self) -> Result<(), SidecarSpawnError> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        if let Some(mut child) = self.child.take() {
            // Job close usually reaps the child first; ignore NotFound races.
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }

    /// Redacted recent stderr lines (most recent last), capped at 200.
    pub fn recent_diagnostics(&self) -> Vec<String> {
        self.diagnostics.snapshot()
    }
}

impl Drop for SidecarProcess {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.kill_child();
        }
        // JobHandle drops after this body, closing the handle and killing
        // anything still assigned to the job.
    }
}

/// Spawns the funnel sidecar wired to the given local target, returning the
/// managed process plus an unbounded receiver for parsed stdout events.
// SidecarSpawnError is crate-private until Task 10; see SidecarProcess::stop.
#[allow(private_interfaces)]
pub fn spawn_sidecar(
    cfg: SidecarConfig,
) -> Result<(SidecarProcess, mpsc::UnboundedReceiver<SidecarEvent>), SidecarSpawnError> {
    let path = sidecar_path()?;
    let mut cmd = std::process::Command::new(path);
    cmd.args([
        "--target",
        &format!("http://127.0.0.1:{}", cfg.local_port),
        "--state-dir",
        &cfg.state_dir.to_string_lossy(),
        "--hostname",
        &cfg.hostname,
    ]);
    cmd.env("MCP_CAPABILITY_TOKEN", &cfg.capability_token);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|_| SidecarSpawnError {
        code: "SIDECAR_SPAWN_FAILED".into(),
        message: "failed to spawn sidecar".into(), // generic: no OS error text
    })?;

    let job = match JobHandle::create() {
        Ok(job) => {
            // If assignment fails we still run with direct-child kill only;
            // tree-kill is defense in depth, not a spawn precondition.
            if let Err(e) = job.assign(&child) {
                eprintln!("job-object assign unavailable: {e}");
                None
            } else {
                Some(job)
            }
        }
        Err(e) => {
            eprintln!("job-object create unavailable: {e}");
            None
        }
    };

    let stdout = child.stdout.take().expect("stdout piped just above");
    let stderr = child.stderr.take().expect("stderr piped just above");

    let (tx, rx) = mpsc::unbounded_channel::<SidecarEvent>();
    let diagnostics = std::sync::Arc::new(DiagnosticsRing::new(DIAGNOSTICS_CAP));

    // Stdout reader: parse each line into an event; malformed/unknown lines
    // go to the ring instead. Never panics on sidecar output.
    let tx_lines = tx.clone();
    let diag_stdout = diagnostics.clone();
    let token = cfg.capability_token.clone();
    std::thread::Builder::new()
        .name("sidecar-stdout".into())
        .spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                match parse_event(&line, &token) {
                    Some(event) => {
                        if tx_lines.send(event).is_err() {
                            break; // receiver dropped; stop reading
                        }
                    }
                    None => diag_stdout.push(redact(&line, &token)),
                }
            }
        })
        .map_err(|e| {
            let _ = child.kill();
            SidecarSpawnError {
                code: "SIDECAR_SPAWN_FAILED".into(),
                message: format!("failed to start sidecar reader thread ({e})"),
            }
        })?;

    // Stderr reader: everything lands in the shared ring, redacted.
    let diag_stderr = diagnostics.clone();
    let token = cfg.capability_token.clone();
    std::thread::Builder::new()
        .name("sidecar-stderr".into())
        .spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                diag_stderr.push(redact(&line, &token));
            }
        })
        .map_err(|e| {
            let _ = child.kill();
            SidecarSpawnError {
                code: "SIDECAR_SPAWN_FAILED".into(),
                message: format!("failed to start sidecar stderr thread ({e})"),
            }
        })?;

    Ok((
        SidecarProcess {
            child: Some(child),
            job,
            diagnostics,
            stopped: false,
        },
        rx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_auth_required_event_with_url() {
        let line = r#"{"type":"auth_required","url":"https://login.tailscale.com/a/123"}"#;
        assert_eq!(
            parse_event(line, "test-token"),
            Some(SidecarEvent::AuthRequired {
                url: "https://login.tailscale.com/a/123".into()
            })
        );
    }

    #[test]
    fn auth_required_non_login_url_is_dropped() {
        for hostile in [
            r#"{"type":"auth_required","url":"https://evil.example/a/1"}"#,
            r#"{"type":"auth_required","url":"http://login.tailscale.com/a/1"}"#,
            "not json",
        ] {
            assert_eq!(
                parse_event(hostile, "test-token"),
                None,
                "line should be dropped: {hostile}"
            );
        }
    }

    #[test]
    fn error_event_message_is_redacted_of_token() {
        // Simulates the reader-thread path: a sidecar error echoing the
        // capability token must never carry it into UI-visible state.
        let token = "super-secret-token";
        let line =
            format!(r#"{{"type":"error","code":"X","message":"failed with {token} in env"}}"#);
        let event = parse_event(&line, "super-secret-token").unwrap();
        match event {
            SidecarEvent::Error { message, .. } => {
                assert!(!message.contains(token));
            }
            other => panic!("expected Error event, got {other:?}"),
        }
    }

    #[test]
    fn parses_funnel_listening_event_with_fqdn() {
        assert_eq!(
            parse_event(
                r#"{"type":"funnel_listening","fqdn":"foo.tailnet.ts.net"}"#,
                "test-token"
            ),
            Some(SidecarEvent::FunnelListening {
                fqdn: "foo.tailnet.ts.net".into()
            })
        );
    }

    #[test]
    fn unknown_event_type_returns_none_without_panicking() {
        assert_eq!(parse_event(r#"{"type":"weird","x":1}"#, "test-token"), None);
        // Missing "type" entirely is equally tolerated.
        assert_eq!(parse_event(r#"{"x":1}"#, "test-token"), None);
    }

    #[test]
    fn garbage_line_returns_none() {
        assert_eq!(parse_event("not json", "test-token"), None);
        assert_eq!(parse_event("", "test-token"), None);
    }

    #[test]
    fn redaction_scrubs_token_and_login_url() {
        let token = "super-secret-capability-token";
        let input =
            format!("auth failed for {token} at https://login.tailscale.com/a/abc123 trailing");
        let out = redact(&input, token);
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains(token));
        assert!(!out.contains("abc123"));
    }
}

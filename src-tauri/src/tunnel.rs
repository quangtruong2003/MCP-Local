//! Low-level adapter around OpenAI's bundled `tunnel-client` binary.
//!
//! This module intentionally does not decide whether the app is Ready. It
//! owns CLI argument construction, tunnel CRUD lookups and the runtime child
//! process. `connection.rs` owns orchestration and readiness semantics.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;

use tauri::{AppHandle, Manager};

const SIDECAR_FILE: &str = "tunnel-client-x86_64-pc-windows-msvc.exe";
const TUNNEL_DESCRIPTION: &str = "Routes ChatGPT to Local Workspace MCP Manager";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TunnelRecord {
    #[serde(alias = "tunnel_id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub organization_ids: Vec<String>,
    #[serde(default)]
    pub workspace_ids: Vec<String>,
}

#[derive(Default)]
pub struct TunnelHandle {
    child: Mutex<Option<Child>>,
}

pub fn sidecar_path() -> PathBuf {
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

fn build_runtime_args(tunnel_id: &str, port: u16, health_url_file: &Path) -> Vec<String> {
    vec![
        "run".into(),
        format!("--control-plane.tunnel-id={tunnel_id}"),
        "--control-plane.api-key=env:CONTROL_PLANE_API_KEY".into(),
        format!("--mcp.server-url=url=http://127.0.0.1:{port}/mcp"),
        "--health.listen-addr=127.0.0.1:0".into(),
        format!("--health.url-file={}", health_url_file.display()),
    ]
}

fn build_create_args(
    name: &str,
    organization_id: Option<&str>,
    workspace_id: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "admin".into(),
        "--json".into(),
        "tunnels".into(),
        "create".into(),
        format!("--name={name}"),
        format!("--description={TUNNEL_DESCRIPTION}"),
    ];
    if let Some(id) = non_blank(organization_id) {
        args.push(format!("--organization-id={id}"));
    }
    if let Some(id) = non_blank(workspace_id) {
        args.push(format!("--workspace-id={id}"));
    }
    args
}

fn build_get_args(tunnel_id: &str) -> Vec<String> {
    vec![
        "admin".into(),
        "--json".into(),
        "tunnels".into(),
        "get".into(),
        tunnel_id.into(),
    ]
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn parse_tunnel_record(stdout: &str) -> Result<TunnelRecord, String> {
    serde_json::from_str(stdout.trim()).map_err(|e| format!("invalid tunnel-client JSON: {e}"))
}

fn output_error(action: &str, output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if !stdout.is_empty() { stdout } else { stderr };
    if detail.is_empty() {
        format!("tunnel-client {action} failed with {}", output.status)
    } else {
        format!("tunnel-client {action} failed: {detail}")
    }
}

pub fn create_tunnel(
    admin_key: &str,
    name: &str,
    organization_id: Option<&str>,
    workspace_id: Option<&str>,
) -> Result<TunnelRecord, String> {
    if non_blank(organization_id).is_none() && non_blank(workspace_id).is_none() {
        return Err("an Organization ID or Workspace ID is required to create a tunnel".into());
    }
    let output = Command::new(sidecar_path())
        .args(build_create_args(name, organization_id, workspace_id))
        .env("OPENAI_ADMIN_KEY", admin_key)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to launch tunnel-client admin create: {e}"))?;
    if !output.status.success() {
        return Err(output_error("admin create", &output));
    }
    parse_tunnel_record(&String::from_utf8_lossy(&output.stdout))
}

pub fn get_tunnel(runtime_key: &str, tunnel_id: &str) -> Result<TunnelRecord, String> {
    let output = Command::new(sidecar_path())
        .args(build_get_args(tunnel_id))
        .env("CONTROL_PLANE_API_KEY", runtime_key)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to launch tunnel-client admin get: {e}"))?;
    if !output.status.success() {
        return Err(output_error("admin get", &output));
    }
    parse_tunnel_record(&String::from_utf8_lossy(&output.stdout))
}

pub fn spawn_runtime(
    app: &AppHandle,
    tunnel_id: &str,
    runtime_key: &str,
    port: u16,
    health_url_file: &Path,
) -> Result<(), String> {
    let handle = app.state::<TunnelHandle>();
    handle.kill();
    if let Some(parent) = health_url_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(health_url_file);
    let child = Command::new(sidecar_path())
        .args(build_runtime_args(tunnel_id, port, health_url_file))
        .env("CONTROL_PLANE_API_KEY", runtime_key)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn tunnel-client runtime: {e}"))?;
    *handle.child.lock().unwrap() = Some(child);
    Ok(())
}

pub fn stop(app: &AppHandle) {
    app.state::<TunnelHandle>().kill();
}

impl TunnelHandle {
    pub fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn is_running(&self) -> bool {
        let mut guard = self.child.lock().unwrap();
        let Some(child) = guard.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => {
                guard.take();
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_args_target_local_mcp_and_emit_health_url_without_secret() {
        let health_file = std::path::Path::new(r"C:\\temp\\health.url");
        let args = build_runtime_args("tunnel_123", 8123, health_file);
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--control-plane.tunnel-id=tunnel_123".to_string()));
        assert!(args.contains(&"--control-plane.api-key=env:CONTROL_PLANE_API_KEY".to_string()));
        assert!(args.contains(&"--health.listen-addr=127.0.0.1:0".to_string()));
        assert!(args.iter().any(|a| a.starts_with("--health.url-file=")));
        assert!(
            args.contains(&"--mcp.server-url=url=http://127.0.0.1:8123/mcp".to_string()),
            "args were {args:?}"
        );
        assert!(!args.join(" ").contains("runtime-secret"));
    }

    #[test]
    fn create_args_include_scope_but_not_admin_secret() {
        let args = build_create_args(
            "My MCP",
            Some("org_123"),
            Some("ws_123"),
        );
        assert_eq!(args[0], "admin");
        assert_eq!(args[1], "--json");
        assert_eq!(args[2], "tunnels");
        assert_eq!(args[3], "create");
        assert!(args.contains(&"--name=My MCP".to_string()));
        assert!(args.contains(&"--organization-id=org_123".to_string()));
        assert!(args.contains(&"--workspace-id=ws_123".to_string()));
        assert!(!args.join(" ").contains("admin-secret"));
    }

    #[test]
    fn tunnel_record_parses_json_create_output() {
        let record = parse_tunnel_record(
            r#"{"id":"tunnel_123","name":"My MCP","description":"Local Workspace MCP Manager","organization_ids":["org_123"],"workspace_ids":["ws_123"]}"#,
        )
        .unwrap();
        assert_eq!(record.id, "tunnel_123");
        assert_eq!(record.name, "My MCP");
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

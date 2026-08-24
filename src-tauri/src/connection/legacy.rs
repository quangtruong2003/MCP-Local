//! High-level connection lifecycle for the desktop app.
//!
//! The MCP server itself is started with the app. This manager orchestrates
//! the remote Secure MCP Tunnel identity, the tunnel-client runtime process,
//! and readiness reporting exposed to the React UI.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::secret_store::SecretStore;
use crate::{settings, tunnel};

const EXISTING_TUNNEL_READY_TIMEOUT: Duration = Duration::from_secs(30);
const NEW_TUNNEL_READY_TIMEOUT: Duration = Duration::from_secs(70);
const PROBE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Stopped,
    StartingServer,
    ValidatingTunnel,
    ProvisioningTunnel,
    Connecting,
    CheckingHealth,
    Ready,
    Stopping,
    NeedsSetup,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConnectionSnapshot {
    pub state: ConnectionState,
    pub tunnel_id: Option<String>,
    pub local_mcp_url: String,
    pub health_url: Option<String>,
    pub started_at: Option<String>,
    pub ready_at: Option<String>,
    pub message: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
}

impl ConnectionSnapshot {
    pub fn new(port: u16) -> Self {
        Self {
            state: ConnectionState::Stopped,
            tunnel_id: None,
            local_mcp_url: format!("http://127.0.0.1:{port}/mcp"),
            health_url: None,
            started_at: None,
            ready_at: None,
            message: Some("MCP is stopped".into()),
            error_code: None,
            error_detail: None,
        }
    }
}

pub struct ConnectionManager {
    snapshot: Mutex<ConnectionSnapshot>,
    op_lock: tokio::sync::Mutex<()>,
}

impl ConnectionManager {
    pub fn new(port: u16) -> Self {
        Self {
            snapshot: Mutex::new(ConnectionSnapshot::new(port)),
            op_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> ConnectionSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    pub async fn start(
        &self,
        app: &AppHandle,
        app_dir: &Path,
        port: u16,
    ) -> Result<ConnectionSnapshot, String> {
        let _guard = self.op_lock.lock().await;
        if matches!(
            self.snapshot().state,
            ConnectionState::Ready
                | ConnectionState::StartingServer
                | ConnectionState::ValidatingTunnel
                | ConnectionState::ProvisioningTunnel
                | ConnectionState::Connecting
                | ConnectionState::CheckingHealth
        ) {
            return Ok(self.snapshot());
        }
        self.start_locked(app, app_dir, port, false).await
    }

    pub async fn regenerate(
        &self,
        app: &AppHandle,
        app_dir: &Path,
        port: u16,
    ) -> Result<ConnectionSnapshot, String> {
        let _guard = self.op_lock.lock().await;
        // Preserve the currently saved tunnel until replacement creation has
        // actually succeeded. `start_locked(force_new=true)` persists the new
        // ID only after `admin tunnels create` returns successfully.
        tunnel::stop(app);
        self.start_locked(app, app_dir, port, true).await
    }

    pub async fn stop(
        &self,
        app: &AppHandle,
        app_dir: &Path,
    ) -> Result<ConnectionSnapshot, String> {
        let _guard = self.op_lock.lock().await;
        if self.snapshot().state == ConnectionState::Stopped {
            return Ok(self.snapshot());
        }
        self.transition(
            app,
            ConnectionState::Stopping,
            Some("Stopping secure tunnel…"),
            None,
            None,
        );
        tunnel::stop(app);
        let _ = std::fs::remove_file(health_url_file(app_dir));
        {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.state = ConnectionState::Stopped;
            snapshot.health_url = None;
            snapshot.started_at = None;
            snapshot.ready_at = None;
            snapshot.message = Some("MCP is stopped".into());
            snapshot.error_code = None;
            snapshot.error_detail = None;
        }
        self.emit(app);
        Ok(self.snapshot())
    }

    async fn start_locked(
        &self,
        app: &AppHandle,
        app_dir: &Path,
        port: u16,
        force_new: bool,
    ) -> Result<ConnectionSnapshot, String> {
        self.reset_for_start(app);
        let mut loaded = settings::load(app_dir)
            .map_err(|e| self.fail_message(app, "SETTINGS_LOAD_FAILED", &e))?;
        let store = SecretStore::system();
        let runtime_key = match store
            .load_runtime_key()
            .map_err(|e| self.fail_message(app, "SECRET_STORE_FAILED", &e))?
        {
            Some(key) => key,
            None => {
                return self.needs_setup(
                    app,
                    "RUNTIME_KEY_REQUIRED",
                    "Add a Runtime API key in Settings before starting.",
                )
            }
        };

        let newly_created = force_new || loaded.settings.tunnel_id.trim().is_empty();
        let tunnel_id = if newly_created {
            loaded.settings.validate_scope().map_err(|message| {
                self.set_failure(
                    app,
                    ConnectionState::NeedsSetup,
                    "TUNNEL_SCOPE_REQUIRED",
                    &message,
                    None,
                );
                message
            })?;
            let admin_key = match store
                .load_admin_key()
                .map_err(|e| self.fail_message(app, "SECRET_STORE_FAILED", &e))?
            {
                Some(key) => key,
                None => {
                    return self.needs_setup(
                        app,
                        "ADMIN_KEY_REQUIRED",
                        "Add an Admin API key to create the first Secure MCP Tunnel.",
                    )
                }
            };
            self.transition(
                app,
                ConnectionState::ProvisioningTunnel,
                Some("Creating a reusable Secure MCP Tunnel…"),
                None,
                None,
            );
            let name = loaded.settings.tunnel_name.clone();
            let org = loaded.settings.organization_id.clone();
            let workspace = loaded.settings.workspace_id.clone();
            let created = tokio::task::spawn_blocking(move || {
                tunnel::create_tunnel(&admin_key, &name, org.as_deref(), workspace.as_deref())
            })
            .await
            .map_err(|e| format!("tunnel create task failed: {e}"))?
            .map_err(|e| self.fail_message(app, "TUNNEL_CREATE_FAILED", &e))?;
            loaded.settings.tunnel_id = created.id.clone();
            settings::save(app_dir, &loaded.settings)
                .map_err(|e| self.fail_message(app, "SETTINGS_SAVE_FAILED", &e))?;
            created.id
        } else {
            let id = loaded.settings.tunnel_id.trim().to_string();
            self.transition(
                app,
                ConnectionState::ValidatingTunnel,
                Some("Validating saved tunnel…"),
                None,
                None,
            );
            let key = runtime_key.clone();
            let lookup_id = id.clone();
            tokio::task::spawn_blocking(move || tunnel::get_tunnel(&key, &lookup_id))
                .await
                .map_err(|e| format!("tunnel lookup task failed: {e}"))?
                .map_err(|e| self.fail_message(app, "TUNNEL_LOOKUP_FAILED", &e))?;
            id
        };

        {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.tunnel_id = Some(tunnel_id.clone());
        }
        self.transition(
            app,
            ConnectionState::Connecting,
            Some("Starting secure tunnel runtime…"),
            None,
            None,
        );
        let health_file = health_url_file(app_dir);
        tunnel::spawn_runtime(app, &tunnel_id, &runtime_key, port, &health_file)
            .map_err(|e| self.fail_message(app, "TUNNEL_RUNTIME_FAILED", &e))?;
        self.transition(
            app,
            ConnectionState::CheckingHealth,
            Some(if newly_created {
                "Tunnel created. Waiting for OpenAI provisioning and readiness…"
            } else {
                "Checking tunnel readiness…"
            }),
            None,
            None,
        );

        let timeout = if newly_created {
            NEW_TUNNEL_READY_TIMEOUT
        } else {
            EXISTING_TUNNEL_READY_TIMEOUT
        };
        let health_url = self
            .wait_until_ready(app, &health_file, timeout)
            .await
            .map_err(|e| self.fail_message(app, "TUNNEL_NOT_READY", &e))?;

        {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.state = ConnectionState::Ready;
            snapshot.health_url = Some(health_url);
            snapshot.ready_at = Some(now_iso());
            snapshot.message = Some("Secure MCP Tunnel is ready".into());
            snapshot.error_code = None;
            snapshot.error_detail = None;
        }
        self.emit(app);
        Ok(self.snapshot())
    }

    fn reset_for_start(&self, app: &AppHandle) {
        {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.state = ConnectionState::StartingServer;
            snapshot.health_url = None;
            snapshot.started_at = Some(now_iso());
            snapshot.ready_at = None;
            snapshot.message = Some("Preparing local MCP server…".into());
            snapshot.error_code = None;
            snapshot.error_detail = None;
        }
        self.emit(app);
    }

    async fn wait_until_ready(
        &self,
        app: &AppHandle,
        health_file: &Path,
        timeout: Duration,
    ) -> Result<String, String> {
        let started = tokio::time::Instant::now();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;
        let mut known_health_url: Option<String> = None;

        while started.elapsed() < timeout {
            if !app.state::<tunnel::TunnelHandle>().is_running() {
                return Err("tunnel-client exited before becoming ready".into());
            }
            if known_health_url.is_none() {
                if let Ok(raw) = std::fs::read_to_string(health_file) {
                    let url = raw.trim().trim_end_matches('/').to_string();
                    if url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")
                    {
                        known_health_url = Some(url);
                    }
                }
            }
            if let Some(base) = known_health_url.as_deref() {
                // /healthz proves liveness only. Ready is deliberately gated
                // exclusively by /readyz returning a successful status.
                let _ = client.get(format!("{base}/healthz")).send().await;
                if let Ok(response) = client.get(format!("{base}/readyz")).send().await {
                    if response.status().is_success() {
                        return Ok(base.to_string());
                    }
                }
            }
            tokio::time::sleep(PROBE_INTERVAL).await;
        }
        Err(format!(
            "tunnel-client did not report ready within {} seconds",
            timeout.as_secs()
        ))
    }

    fn needs_setup(
        &self,
        app: &AppHandle,
        code: &str,
        message: &str,
    ) -> Result<ConnectionSnapshot, String> {
        self.set_failure(app, ConnectionState::NeedsSetup, code, message, None);
        Err(message.into())
    }

    fn fail_message(&self, app: &AppHandle, code: &str, detail: &str) -> String {
        let message = friendly_error(code);
        self.set_failure(app, ConnectionState::Error, code, message, Some(detail));
        message.into()
    }

    fn set_failure(
        &self,
        app: &AppHandle,
        state: ConnectionState,
        code: &str,
        message: &str,
        detail: Option<&str>,
    ) {
        tunnel::stop(app);
        {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.state = state;
            snapshot.health_url = None;
            snapshot.message = Some(message.into());
            snapshot.error_code = Some(code.into());
            snapshot.error_detail = detail.map(Into::into);
        }
        self.emit(app);
    }

    fn transition(
        &self,
        app: &AppHandle,
        state: ConnectionState,
        message: Option<&str>,
        error_code: Option<&str>,
        detail: Option<&str>,
    ) {
        {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.state = state;
            snapshot.message = message.map(Into::into);
            snapshot.error_code = error_code.map(Into::into);
            snapshot.error_detail = detail.map(Into::into);
        }
        self.emit(app);
    }

    fn emit(&self, app: &AppHandle) {
        let _ = app.emit("connection-status", self.snapshot());
    }
}

fn friendly_error(code: &str) -> &'static str {
    match code {
        "SETTINGS_LOAD_FAILED" => "Connection settings could not be loaded.",
        "SETTINGS_SAVE_FAILED" => "The new tunnel was created but its connection settings could not be saved.",
        "SECRET_STORE_FAILED" => "Protected credentials could not be accessed on this Windows account.",
        "TUNNEL_CREATE_FAILED" => "Could not create the Secure MCP Tunnel. Check Admin key and scope settings.",
        "TUNNEL_LOOKUP_FAILED" => "The saved tunnel could not be validated. Check the Runtime key or regenerate the tunnel.",
        "TUNNEL_RUNTIME_FAILED" => "The secure tunnel runtime could not start.",
        "TUNNEL_NOT_READY" => "The secure tunnel did not become ready.",
        _ => "The secure tunnel could not start.",
    }
}

fn health_url_file(app_dir: &Path) -> PathBuf {
    app_dir.join("tunnel-health.url")
}

fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_uses_loopback_mcp_url() {
        assert_eq!(
            ConnectionSnapshot::new(9999).local_mcp_url,
            "http://127.0.0.1:9999/mcp"
        );
    }

    #[test]
    fn readiness_error_copy_is_actionable() {
        assert!(friendly_error("TUNNEL_NOT_READY").contains("did not become ready"));
    }
}

//! Idempotent WSL/Lightpanda runtime provisioner.
//!
//! Drives the provisioning state machine from the design spec:
//! uninitialized -> checking_host -> (wsl_install_required -> installing_wsl ->
//! reboot_required) -> distro_install_required -> installing_distro ->
//! lightpanda_install_required -> installing_lightpanda -> verifying_runtime ->
//! ready, with repair_required / unsupported_host side states.
//!
//! All downloads are pinned by SHA-256 from the immutable runtime manifest,
//! use `.partial` files with atomic rename, and are verified before import or
//! install. Only `ready` may launch browser sessions.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lc_core::error::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::wsl::{
    choose_distro_name, generate_install_id, validate_install_id, verify_ownership,
    DistroNameDecision, ManagedDistroIdentity, OwnershipMarker, WslCommandRunner, WslHostAdapter,
    WslUser, LIGHTPANDA_BIN, PREFERRED_DISTRO_NAME,
};

/// Name of the small atomic state file persisted under the runtime root.
pub const RUNTIME_STATE_FILE: &str = "runtime-state.json";
/// Install root for Lightpanda binaries inside the managed distro.
pub const LIGHTPANDA_INSTALL_ROOT: &str = "/opt/local-workspaces/lightpanda";

/// Required Lightpanda native MCP tools for v0.2.0. Missing tools are a
/// version-mismatch failure, never a partial success.
pub const REQUIRED_NATIVE_TOOLS: &[&str] = &[
    "goto",
    "getUrl",
    "getCookies",
    "consoleLogs",
    "tree",
    "markdown",
    "html",
    "nodeDetails",
    "findElement",
    "interactiveElements",
    "links",
    "extract",
    "structuredData",
    "search",
    "click",
    "hover",
    "fill",
    "selectOption",
    "setChecked",
    "press",
    "scroll",
    "detectForms",
    "evaluate",
    "waitForSelector",
    "waitForState",
    "waitForScript",
];

/// Provisioning states, exactly as named in the design spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProvisionState {
    Uninitialized,
    CheckingHost,
    WslInstallRequired,
    InstallingWsl,
    RebootRequired,
    DistroInstallRequired,
    InstallingDistro,
    LightpandaInstallRequired,
    InstallingLightpanda,
    VerifyingRuntime,
    Ready,
    RepairRequired,
    UnsupportedHost,
}

/// Typed immutable runtime manifest (bundled Tauri resource).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RuntimeManifest {
    pub schema_version: u32,
    pub runtime_family: String,
    pub lightpanda_version: String,
    pub lightpanda_arch: String,
    pub lightpanda_asset_url: String,
    pub lightpanda_sha256: String,
    pub distro_image_version: String,
    pub distro_arch: String,
    pub distro_image_url: String,
    pub distro_image_sha256: String,
}

fn bad_manifest(what: &str) -> CoreError {
    CoreError::new(
        ErrorCode::BrowserRuntimeProvisionFailed,
        format!("runtime manifest is invalid: {what}"),
    )
}

impl RuntimeManifest {
    /// Parses and validates the manifest, rejecting placeholders, floating
    /// versions, and non-pinned URLs.
    pub fn from_json_str(json: &str) -> Result<Self, CoreError> {
        let manifest: RuntimeManifest = serde_json::from_str(json).map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserRuntimeProvisionFailed,
                format!("runtime manifest is not valid json: {e}"),
            )
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version != 1 {
            return Err(bad_manifest("unsupported schema_version"));
        }
        if self.runtime_family != "lightpanda-wsl" {
            return Err(bad_manifest("runtime_family must be lightpanda-wsl"));
        }
        let fields = [
            &self.runtime_family,
            &self.lightpanda_version,
            &self.lightpanda_arch,
            &self.lightpanda_asset_url,
            &self.lightpanda_sha256,
            &self.distro_image_version,
            &self.distro_arch,
            &self.distro_image_url,
            &self.distro_image_sha256,
        ];
        for field in fields {
            let lower = field.to_lowercase();
            if field.contains('<') || field.contains('>') || lower.contains("placeholder") {
                return Err(bad_manifest("field contains a placeholder"));
            }
        }
        validate_pinned_version(&self.lightpanda_version)?;
        if self.lightpanda_arch != "x86_64-linux" {
            return Err(bad_manifest("lightpanda_arch must be x86_64-linux"));
        }
        if self.distro_arch != "amd64" {
            return Err(bad_manifest("distro_arch must be amd64"));
        }
        if !is_sha256_hex(&self.lightpanda_sha256) {
            return Err(bad_manifest(
                "lightpanda_sha256 must be 64 lowercase hex chars",
            ));
        }
        if !is_sha256_hex(&self.distro_image_sha256) {
            return Err(bad_manifest(
                "distro_image_sha256 must be 64 lowercase hex chars",
            ));
        }
        validate_release_asset_url(
            &self.lightpanda_asset_url,
            &format!("/releases/download/{}/", self.lightpanda_version),
        )?;
        validate_release_asset_url(
            &self.distro_image_url,
            &format!("/releases/download/runtime-{}/", self.distro_image_version),
        )?;
        if self.distro_image_version.is_empty()
            || self
                .distro_image_version
                .chars()
                .any(|c| !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' && c != '.')
        {
            return Err(bad_manifest(
                "distro_image_version must be lowercase alphanumeric",
            ));
        }
        Ok(())
    }

    /// Lightpanda install directory inside the distro for this manifest.
    pub fn lightpanda_install_dir(&self) -> String {
        format!("{LIGHTPANDA_INSTALL_ROOT}/{}", self.lightpanda_version)
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Immutable release asset URL: https on github.com with a pinned
/// `/releases/download/<tag>/` path and no mutable segments.
fn validate_release_asset_url(url: &str, required_tag_segment: &str) -> Result<(), CoreError> {
    let invalid = || {
        CoreError::new(
            ErrorCode::BrowserRuntimeProvisionFailed,
            format!("runtime manifest url is not a pinned immutable release asset: {url}"),
        )
    };
    let parsed = url::Url::parse(url).map_err(|_| invalid())?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("github.com") {
        return Err(invalid());
    }
    let path = parsed.path();
    if !path.contains("/releases/download/") || !path.contains(required_tag_segment) {
        return Err(invalid());
    }
    if path
        .split('/')
        .any(|segment| matches!(segment, "latest" | "nightly" | "rolling" | "edge"))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Small atomic persisted runtime state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeStateFile {
    pub state: RuntimeProvisionState,
    pub install_id: String,
    pub distro: Option<ManagedDistroIdentity>,
    pub distro_image_version: Option<String>,
    pub lightpanda_version: Option<String>,
    /// Sanitized reason code only (never raw output or host paths).
    pub reason: Option<String>,
}

/// Downloads pinned runtime artifacts over HTTPS with `.partial` + atomic
/// rename after SHA-256 verification.
#[async_trait]
pub trait RuntimeDownloader: Send + Sync {
    async fn download_verified(
        &self,
        url: &str,
        dest: &Path,
        expected_sha256: &str,
    ) -> Result<(), CoreError>;
}

/// Production HTTPS downloader (reqwest, bounded redirects/timeouts).
pub struct HttpsRuntimeDownloader {
    client: reqwest::Client,
}

impl HttpsRuntimeDownloader {
    pub fn new() -> Result<Self, CoreError> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(900))
            .build()
            .map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserRuntimeDownloadFailed,
                    format!("failed to initialize download client: {e}"),
                )
            })?;
        Ok(Self { client })
    }
}

impl Default for HttpsRuntimeDownloader {
    fn default() -> Self {
        Self::new().expect("reqwest client builds")
    }
}

#[async_trait]
impl RuntimeDownloader for HttpsRuntimeDownloader {
    async fn download_verified(
        &self,
        url: &str,
        dest: &Path,
        expected_sha256: &str,
    ) -> Result<(), CoreError> {
        let parsed = url::Url::parse(url).map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserRuntimeDownloadFailed,
                format!("artifact url is invalid: {e}"),
            )
        })?;
        if parsed.scheme() != "https" {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeDownloadFailed,
                "artifact url must use https",
            ));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserRuntimeDownloadFailed,
                    format!("failed to create staging directory: {e}"),
                )
            })?;
        }
        let file_name = dest
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let partial = dest.with_file_name(format!("{file_name}.partial"));
        let _ = std::fs::remove_file(&partial);

        let response = self.client.get(url).send().await.map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserRuntimeDownloadFailed,
                format!("artifact download failed: {e}"),
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeDownloadFailed,
                format!("artifact download failed with http status {status}"),
            ));
        }
        let bytes = response.bytes().await.map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserRuntimeDownloadFailed,
                format!("artifact download failed: {e}"),
            )
        })?;

        // Verify before anything is written beside the .partial file.
        let digest = sha256_hex(&bytes);
        if digest != expected_sha256 {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeIntegrityFailed,
                "downloaded artifact sha256 does not match the pinned manifest digest",
            ));
        }
        std::fs::write(&partial, &bytes).map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserRuntimeDownloadFailed,
                format!("failed to stage artifact: {e}"),
            )
        })?;
        std::fs::rename(&partial, dest).map_err(|e| {
            let _ = std::fs::remove_file(&partial);
            CoreError::new(
                ErrorCode::BrowserRuntimeDownloadFailed,
                format!("failed to finalize artifact: {e}"),
            )
        })?;
        Ok(())
    }
}

/// MCP smoke verification: launch a temporary Lightpanda MCP child, perform
/// initialize + tools/list, verify the required native tool set, terminate.
#[async_trait]
pub trait McpSmokeVerifier: Send + Sync {
    async fn smoke_verify(&self, distro_name: &str) -> Result<(), CoreError>;
}

/// Production smoke verifier speaking raw JSON-RPC over the child stdio.
pub struct RealMcpSmokeVerifier {
    wsl_exe: PathBuf,
}

impl RealMcpSmokeVerifier {
    pub fn new(wsl_exe: PathBuf) -> Self {
        Self { wsl_exe }
    }
}

#[async_trait]
impl McpSmokeVerifier for RealMcpSmokeVerifier {
    async fn smoke_verify(&self, distro_name: &str) -> Result<(), CoreError> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let std_cmd = super::wsl::build_lightpanda_command(&self.wsl_exe, distro_name, &[])?;
        let mut child = tokio::process::Command::from(std_cmd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserWslUnavailable,
                    format!("failed to spawn lightpanda smoke verifier: {e}"),
                )
            })?;

        let mut stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");

        let requests = [
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": { "name": "local-workspaces-runtime-smoke", "version": "1.0" }
                }
            }),
            serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        ];
        for request in &requests {
            let mut line = serde_json::to_string(request).map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserProtocolError,
                    format!("failed to encode smoke request: {e}"),
                )
            })?;
            line.push('\n');
            stdin.write_all(line.as_bytes()).await.map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserWslUnavailable,
                    format!("lightpanda smoke verifier stdin failed: {e}"),
                )
            })?;
        }
        // A flush failure means the child pipe is gone; the read loop below
        // surfaces that as a fail-closed error.
        let _ = stdin.flush().await;

        let mut lines = BufReader::new(stdout).lines();
        let outcome = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(line) = lines.next_line().await.map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserWslUnavailable,
                    format!("lightpanda smoke verifier stdout failed: {e}"),
                )
            })? {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if value.get("id") == Some(&serde_json::json!(2)) {
                    return Ok(value);
                }
            }
            Err(CoreError::new(
                ErrorCode::BrowserWslUnavailable,
                "lightpanda smoke verifier exited before answering tools/list",
            ))
        })
        .await;

        let _ = child.start_kill();
        let _ = child.wait().await;

        let response = match outcome {
            Ok(Ok(value)) => value,
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                return Err(CoreError::new(
                    ErrorCode::BrowserWslUnavailable,
                    "lightpanda smoke verification timed out",
                ))
            }
        };
        if response.get("error").is_some() {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeVersionMismatch,
                "lightpanda tools/list failed during smoke verification",
            ));
        }
        let tools: Vec<String> = response
            .pointer("/result/tools")
            .and_then(|v| v.as_array())
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.get("name").and_then(|n| n.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let missing = REQUIRED_NATIVE_TOOLS
            .iter()
            .filter(|required| !tools.iter().any(|tool| tool == *required))
            .count();
        if missing > 0 {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeVersionMismatch,
                format!("lightpanda runtime is missing {missing} required native tools"),
            ));
        }
        Ok(())
    }
}

/// Serialized, idempotent provisioner for the managed WSL2 Lightpanda runtime.
pub struct BrowserRuntimeManager {
    adapter: Arc<WslHostAdapter<Arc<dyn WslCommandRunner>>>,
    downloader: Arc<dyn RuntimeDownloader>,
    smoke_verifier: Arc<dyn McpSmokeVerifier>,
    manifest: RuntimeManifest,
    /// Managed distro install root (LocalAppData on Windows).
    wsl_dir: PathBuf,
    /// Download staging directory for pinned artifacts.
    staging_dir: PathBuf,
    /// Process-wide serialization of provisioning runs.
    provision_lock: Mutex<()>,
    current: Mutex<RuntimeStateFile>,
}

/// Fixed shell fragment that re-asserts the runtime user's directory layout
/// after import (rootfs already ships it; idempotent by design).
const RUNTIME_DIRS_FRAGMENT: &str = "mkdir -p /var/lib/local-workspaces/browser/profile /var/lib/local-workspaces/browser/cookies /var/lib/local-workspaces/browser/cache /var/lib/local-workspaces/browser/artifacts /opt/local-workspaces/lightpanda && chown -R localworkspaces:localworkspaces /var/lib/local-workspaces /opt/local-workspaces/lightpanda";

impl BrowserRuntimeManager {
    pub fn new(
        runner: Arc<dyn WslCommandRunner>,
        downloader: Arc<dyn RuntimeDownloader>,
        smoke_verifier: Arc<dyn McpSmokeVerifier>,
        manifest: RuntimeManifest,
        browser_root: PathBuf,
    ) -> Arc<Self> {
        let wsl_dir = browser_root.join("wsl");
        let staging_dir = browser_root.join("staging");
        let current = Self::load_or_init_state(&wsl_dir);
        Arc::new(Self {
            adapter: Arc::new(WslHostAdapter::new(runner)),
            downloader,
            smoke_verifier,
            manifest,
            wsl_dir,
            staging_dir,
            provision_lock: Mutex::new(()),
            current: Mutex::new(current),
        })
    }

    /// Production wiring: real WSL runner, HTTPS downloader, MCP smoke
    /// verifier, bundled manifest, LocalAppData paths.
    #[cfg(windows)]
    pub fn production(manifest_json: &str) -> Result<Arc<Self>, CoreError> {
        use super::wsl::RealWslCommandRunner;
        let manifest = RuntimeManifest::from_json_str(manifest_json)?;
        let browser_root = default_runtime_root()?;
        Ok(Self::new(
            Arc::new(RealWslCommandRunner),
            Arc::new(HttpsRuntimeDownloader::new()?),
            Arc::new(RealMcpSmokeVerifier::new(PathBuf::from("wsl.exe"))),
            manifest,
            browser_root,
        ))
    }

    #[cfg(not(windows))]
    pub fn production(_manifest_json: &str) -> Result<Arc<Self>, CoreError> {
        Err(CoreError::new(
            ErrorCode::BrowserWslUnavailable,
            "the lightpanda runtime requires a Windows host",
        ))
    }

    pub fn current_state(&self) -> RuntimeProvisionState {
        self.current
            .try_lock()
            .map(|guard| guard.state)
            .unwrap_or(RuntimeProvisionState::Uninitialized)
    }

    pub fn distro_identity(&self) -> Option<ManagedDistroIdentity> {
        self.current.try_lock().ok()?.distro.clone()
    }

    pub fn manifest(&self) -> &RuntimeManifest {
        &self.manifest
    }

    /// Ensures the runtime is provisioned and verified. Lazy: only called by
    /// browser session startup / explicit setup commands, never by opening
    /// Settings. Returns `Ready` or a fail-closed error.
    pub async fn ensure_runtime(&self) -> Result<RuntimeProvisionState, CoreError> {
        self.ensure_inner(false).await
    }

    /// Repair: revalidate everything, replace only app-owned runtime files,
    /// rebuild the owned distro only when it cannot be repaired in place.
    pub async fn repair_runtime(&self) -> Result<RuntimeProvisionState, CoreError> {
        self.ensure_inner(true).await
    }

    async fn ensure_inner(&self, force_rebuild: bool) -> Result<RuntimeProvisionState, CoreError> {
        // Single flight: provisioning is serialized process-wide.
        let _serialization = self.provision_lock.lock().await;

        self.set_state(RuntimeProvisionState::CheckingHost, None)
            .await;
        let mut host = self.adapter.host_status().await?;
        if !host.wsl_installed {
            self.set_state(RuntimeProvisionState::WslInstallRequired, None)
                .await;
            self.set_state(RuntimeProvisionState::InstallingWsl, None)
                .await;
            self.adapter.launch_elevated_install()?;
            host = self.adapter.host_status().await?;
            if !host.wsl_installed {
                self.set_state(
                    RuntimeProvisionState::RebootRequired,
                    Some("wsl_installed_reboot_required"),
                )
                .await;
                return Err(CoreError::new(
                    ErrorCode::BrowserWslRebootRequired,
                    "WSL was installed; reboot Windows and retry",
                ));
            }
        }

        let install_id = self.current.lock().await.install_id.clone();
        let preferred_exists = host
            .distros
            .iter()
            .any(|distro| distro.name == PREFERRED_DISTRO_NAME);
        let preferred_marker = if preferred_exists {
            self.adapter.read_marker(PREFERRED_DISTRO_NAME).await?
        } else {
            None
        };
        let decision = choose_distro_name(&host.distros, preferred_marker.as_ref(), &install_id)?;

        let distro_name = match decision {
            DistroNameDecision::ReusePreferred => {
                let is_wsl2 = host
                    .distros
                    .iter()
                    .find(|distro| distro.name == PREFERRED_DISTRO_NAME)
                    .is_some_and(|distro| distro.version == 2);
                if !is_wsl2 {
                    self.set_state(
                        RuntimeProvisionState::UnsupportedHost,
                        Some("managed_distro_is_not_wsl2"),
                    )
                    .await;
                    return Err(CoreError::new(
                        ErrorCode::BrowserRuntimeProvisionFailed,
                        "the managed distro is not a WSL2 distro",
                    ));
                }
                let identity = self
                    .resolve_identity(&install_id, PREFERRED_DISTRO_NAME)
                    .await;
                let registered = self.wsl_dir.join(&identity.distro_name);
                verify_ownership(&identity, preferred_marker.as_ref(), &registered)?;
                let recorded_image = self.current.lock().await.distro_image_version.clone();
                if recorded_image
                    .as_deref()
                    .is_some_and(|version| version != self.manifest.distro_image_version)
                {
                    // Pinned image changed: rebuild the owned distro in place.
                    self.adapter
                        .unregister_owned(&identity, preferred_marker.as_ref(), &registered)
                        .await?;
                    self.provision_distro(PREFERRED_DISTRO_NAME, &install_id)
                        .await?
                } else {
                    identity.distro_name.clone()
                }
            }
            DistroNameDecision::CreatePreferred => {
                self.provision_distro(PREFERRED_DISTRO_NAME, &install_id)
                    .await?
            }
            DistroNameDecision::CreateSuffixed(name) => {
                self.provision_distro(&name, &install_id).await?
            }
        };

        self.ensure_lightpanda(&distro_name, force_rebuild).await?;

        self.set_state(RuntimeProvisionState::VerifyingRuntime, None)
            .await;
        if let Err(error) = self.smoke_verifier.smoke_verify(&distro_name).await {
            self.set_state(
                RuntimeProvisionState::VerifyingRuntime,
                Some("smoke_verification_failed"),
            )
            .await;
            return Err(error);
        }

        let mut current = self.current.lock().await;
        current.state = RuntimeProvisionState::Ready;
        current.lightpanda_version = Some(self.manifest.lightpanda_version.clone());
        current.reason = None;
        Self::persist_state(&self.wsl_dir, &current);
        Ok(RuntimeProvisionState::Ready)
    }

    /// Imports and hardens the owned distro: rootfs download (pinned SHA-256),
    /// `wsl --import`, isolation wsl.conf, runtime dirs, ownership marker,
    /// then a terminate so the next boot picks up the new configuration.
    async fn provision_distro(
        &self,
        distro_name: &str,
        install_id: &str,
    ) -> Result<String, CoreError> {
        self.set_state(RuntimeProvisionState::DistroInstallRequired, None)
            .await;
        self.discard_partial_downloads();
        self.set_state(RuntimeProvisionState::InstallingDistro, None)
            .await;

        let rootfs_name = url_file_name(&self.manifest.distro_image_url)?;
        let rootfs_dest = self.staging_dir.join(&rootfs_name);
        self.downloader
            .download_verified(
                &self.manifest.distro_image_url,
                &rootfs_dest,
                &self.manifest.distro_image_sha256,
            )
            .await?;

        std::fs::create_dir_all(&self.wsl_dir).map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserRuntimeProvisionFailed,
                format!("failed to create managed distro root: {e}"),
            )
        })?;
        let install_dir = self.wsl_dir.join(distro_name);
        self.adapter
            .import_distro(distro_name, &install_dir, &rootfs_dest)
            .await?;
        self.adapter.write_wsl_conf(distro_name).await?;
        self.exec_sh(
            distro_name,
            RUNTIME_DIRS_FRAGMENT,
            None,
            "create runtime user directories",
        )
        .await?;
        let marker = OwnershipMarker::new(install_id.to_string());
        self.adapter.write_marker(distro_name, &marker).await?;

        let identity = ManagedDistroIdentity {
            distro_name: distro_name.to_string(),
            install_id: install_id.to_string(),
            install_dir: install_dir.clone(),
        };
        {
            let mut current = self.current.lock().await;
            current.distro = Some(identity.clone());
            current.distro_image_version = Some(self.manifest.distro_image_version.clone());
            Self::persist_state(&self.wsl_dir, &current);
        }
        self.adapter
            .terminate_owned(&identity, Some(&marker), &install_dir)
            .await?;
        Ok(distro_name.to_string())
    }

    /// Verifies the in-distro Lightpanda binary against the pinned digest and
    /// (re)installs it when missing, mismatched, or forced. A mismatched
    /// binary is removed before anything else and is never executed.
    async fn ensure_lightpanda(&self, distro_name: &str, force: bool) -> Result<(), CoreError> {
        let version_dir = self.manifest.lightpanda_install_dir();
        let bin_path = format!("{version_dir}/lightpanda");
        let current_link = format!("{LIGHTPANDA_INSTALL_ROOT}/current");

        let pre = self
            .adapter
            .exec_fixed(distro_name, WslUser::Root, &["sha256sum", &bin_path], None)
            .await?;
        let pre_digest = if pre.exit_code == 0 {
            pre.decoded_stdout()
                .split_whitespace()
                .next()
                .map(str::to_string)
        } else {
            None
        };
        let digest_ok = pre_digest.as_deref() == Some(self.manifest.lightpanda_sha256.as_str());
        if pre.exit_code == 0 && !digest_ok {
            self.exec_sh(
                distro_name,
                &format!("rm -f {bin_path}"),
                None,
                "remove mismatched lightpanda binary",
            )
            .await?;
        }

        if force || !digest_ok {
            self.set_state(RuntimeProvisionState::LightpandaInstallRequired, None)
                .await;
            self.discard_partial_downloads();
            self.set_state(RuntimeProvisionState::InstallingLightpanda, None)
                .await;
            let asset_name = url_file_name(&self.manifest.lightpanda_asset_url)?;
            let asset_dest = self.staging_dir.join(&asset_name);
            self.downloader
                .download_verified(
                    &self.manifest.lightpanda_asset_url,
                    &asset_dest,
                    &self.manifest.lightpanda_sha256,
                )
                .await?;
            let bytes = std::fs::read(&asset_dest).map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserRuntimeDownloadFailed,
                    format!("failed to read staged artifact: {e}"),
                )
            })?;
            let install_fragment =
                format!("mkdir -p {version_dir} && cat > {bin_path} && chmod 0755 {bin_path}");
            self.exec_sh(
                distro_name,
                &install_fragment,
                Some(bytes),
                "install lightpanda binary",
            )
            .await?;

            let post = self
                .adapter
                .exec_fixed(distro_name, WslUser::Root, &["sha256sum", &bin_path], None)
                .await?;
            let post_digest = if post.exit_code == 0 {
                post.decoded_stdout()
                    .split_whitespace()
                    .next()
                    .map(str::to_string)
            } else {
                None
            };
            if post_digest.as_deref() != Some(self.manifest.lightpanda_sha256.as_str()) {
                let _ = self
                    .exec_sh(
                        distro_name,
                        &format!("rm -f {bin_path}"),
                        None,
                        "remove mismatched lightpanda binary",
                    )
                    .await;
                return Err(CoreError::new(
                    ErrorCode::BrowserRuntimeIntegrityFailed,
                    "installed lightpanda binary does not match the pinned manifest digest",
                ));
            }
            self.exec_sh(
                distro_name,
                &format!("ln -sfn {version_dir} {current_link}"),
                None,
                "switch lightpanda symlink",
            )
            .await?;
        }

        // Version gate: run as the unprivileged runtime user through the
        // `current` symlink exactly like production sessions will.
        let mut out = self
            .adapter
            .exec_fixed(
                distro_name,
                WslUser::Runtime,
                &[LIGHTPANDA_BIN, "version"],
                None,
            )
            .await?;
        if out.exit_code != 0 {
            // One in-place repair attempt: repoint the symlink, recheck.
            self.exec_sh(
                distro_name,
                &format!("ln -sfn {version_dir} {current_link}"),
                None,
                "repoint lightpanda symlink",
            )
            .await?;
            out = self
                .adapter
                .exec_fixed(
                    distro_name,
                    WslUser::Runtime,
                    &[LIGHTPANDA_BIN, "version"],
                    None,
                )
                .await?;
        }
        if out.exit_code != 0 {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeVersionMismatch,
                "lightpanda binary did not start during runtime verification",
            ));
        }
        let reported = out
            .decoded_stdout()
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        if reported != self.manifest.lightpanda_version {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeVersionMismatch,
                "lightpanda binary reported an unexpected version",
            ));
        }
        validate_pinned_version(&reported)?;
        Ok(())
    }

    /// Runs a fixed shell fragment as root inside the distro. Errors are
    /// sanitized: raw stdout/stderr never leaves this module.
    async fn exec_sh(
        &self,
        distro_name: &str,
        fragment: &str,
        stdin: Option<Vec<u8>>,
        context: &str,
    ) -> Result<(), CoreError> {
        let output = self
            .adapter
            .exec_fixed(distro_name, WslUser::Root, &["sh", "-c", fragment], stdin)
            .await?;
        if output.exit_code == 0 {
            Ok(())
        } else {
            Err(CoreError::new(
                ErrorCode::BrowserRuntimeProvisionFailed,
                format!("{context} failed with exit code {}", output.exit_code),
            ))
        }
    }

    /// Returns the persisted identity for the distro, reconstructing (and
    /// persisting) it when the marker proves ownership but the state file has
    /// no record.
    async fn resolve_identity(&self, install_id: &str, distro_name: &str) -> ManagedDistroIdentity {
        let mut current = self.current.lock().await;
        let matches = current.distro.as_ref().is_some_and(|identity| {
            identity.distro_name == distro_name && identity.install_id == install_id
        });
        if !matches {
            current.distro = Some(ManagedDistroIdentity {
                distro_name: distro_name.to_string(),
                install_id: install_id.to_string(),
                install_dir: self.wsl_dir.join(distro_name),
            });
            Self::persist_state(&self.wsl_dir, &current);
        }
        current.distro.clone().expect("identity just set")
    }

    /// Removes stale `.partial` downloads from staging (interrupted runs).
    fn discard_partial_downloads(&self) {
        let Ok(entries) = std::fs::read_dir(&self.staging_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "partial") {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    async fn set_state(&self, state: RuntimeProvisionState, reason: Option<&str>) {
        let mut current = self.current.lock().await;
        current.state = state;
        current.reason = reason.map(str::to_string);
        Self::persist_state(&self.wsl_dir, &current);
    }

    fn load_or_init_state(wsl_dir: &Path) -> RuntimeStateFile {
        let path = wsl_dir.join(RUNTIME_STATE_FILE);
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(state) = serde_json::from_slice::<RuntimeStateFile>(&bytes) {
                if validate_install_id(&state.install_id).is_ok() {
                    return state;
                }
            }
        }
        RuntimeStateFile {
            state: RuntimeProvisionState::Uninitialized,
            install_id: generate_install_id(),
            distro: None,
            distro_image_version: None,
            lightpanda_version: None,
            reason: None,
        }
    }

    fn persist_state(wsl_dir: &Path, state: &RuntimeStateFile) {
        if std::fs::create_dir_all(wsl_dir).is_err() {
            return;
        }
        let path = wsl_dir.join(RUNTIME_STATE_FILE);
        let tmp = wsl_dir.join(format!("{RUNTIME_STATE_FILE}.tmp"));
        let Ok(json) = serde_json::to_vec_pretty(state) else {
            return;
        };
        if std::fs::write(&tmp, &json).is_err() {
            return;
        }
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Default runtime root on Windows: `%LOCALAPPDATA%\Local Workspace MCP
/// Manager\browser` (never Roaming AppData).
pub fn default_runtime_root() -> Result<PathBuf, CoreError> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        CoreError::new(
            ErrorCode::BrowserWslUnavailable,
            "LOCALAPPDATA is not set; cannot resolve the runtime root",
        )
    })?;
    Ok(PathBuf::from(base)
        .join("Local Workspace MCP Manager")
        .join("browser"))
}

fn validate_pinned_version(version: &str) -> Result<(), CoreError> {
    let valid = !version.is_empty()
        && version.chars().all(|c| c.is_ascii_digit() || c == '.')
        && !version.starts_with('.')
        && !version.ends_with('.')
        && !version.contains("..")
        && version.split('.').count() >= 2;
    if valid {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::BrowserRuntimeProvisionFailed,
            format!("version is not a pinned release version: {version}"),
        ))
    }
}

/// Last path segment of an artifact URL, used for staging file names.
fn url_file_name(url: &str) -> Result<String, CoreError> {
    url.rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserRuntimeDownloadFailed,
                "artifact url has no file name segment",
            )
        })
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::wsl::{WslCommand, WslCommandOutput, MARKER_PATH};
    use std::sync::atomic::{AtomicU32, Ordering};

    const TEST_LP_SHA: &str = "895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a";
    const TEST_ROOTFS_SHA: &str =
        "3ba9db8ab987834c2f0e60de5d72c628f30d8d019e148d68066165698202b7c6";

    fn test_manifest() -> RuntimeManifest {
        RuntimeManifest {
            schema_version: 1,
            runtime_family: "lightpanda-wsl".to_string(),
            lightpanda_version: "0.3.7".to_string(),
            lightpanda_arch: "x86_64-linux".to_string(),
            lightpanda_asset_url: "https://github.com/lightpanda-io/browser/releases/download/0.3.7/lightpanda-x86_64-linux".to_string(),
            lightpanda_sha256: TEST_LP_SHA.to_string(),
            distro_image_version: "lwsl-ubuntu-noble-minbase-v1".to_string(),
            distro_arch: "amd64".to_string(),
            distro_image_url: "https://github.com/quangtruong2003/MCP-Local/releases/download/runtime-lwsl-ubuntu-noble-minbase-v1/lightpanda-rootfs-ubuntu-noble-minbase-amd64.tar.gz".to_string(),
            distro_image_sha256: TEST_ROOTFS_SHA.to_string(),
        }
    }

    /// Mutable world the scripted WSL runner reacts to.
    #[derive(Default)]
    struct FakeWorld {
        wsl_installed: bool,
        distros: Vec<String>,
        marker: Option<OwnershipMarker>,
        lightpanda_installed: bool,
        lightpanda_digest: String,
        symlink_switched: bool,
    }

    struct ScriptedRunner {
        world: Arc<Mutex<FakeWorld>>,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        elevated: Arc<AtomicU32>,
        /// Optional one-shot failure injected into `--list --verbose`.
        list_failure: AtomicU32,
    }

    type ScriptedHandles = (
        Arc<ScriptedRunner>,
        Arc<Mutex<Vec<Vec<String>>>>,
        Arc<Mutex<FakeWorld>>,
    );

    impl ScriptedRunner {
        fn new(world: FakeWorld) -> ScriptedHandles {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let world = Arc::new(Mutex::new(world));
            (
                Arc::new(Self {
                    world: world.clone(),
                    calls: calls.clone(),
                    elevated: Arc::new(AtomicU32::new(0)),
                    list_failure: AtomicU32::new(0),
                }),
                calls,
                world,
            )
        }
    }

    fn out(exit_code: i32, stdout: &str, stderr: &str) -> WslCommandOutput {
        WslCommandOutput {
            exit_code,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[async_trait]
    impl WslCommandRunner for ScriptedRunner {
        async fn run(&self, request: WslCommand) -> Result<WslCommandOutput, CoreError> {
            self.calls.lock().await.push(request.args.clone());
            let argv: Vec<&str> = request.args.iter().map(String::as_str).collect();
            let args = argv.as_slice();
            let mut world = self.world.lock().await;
            match args {
                ["--status"] => {
                    if world.wsl_installed {
                        Ok(out(
                            0,
                            "Default Distribution: Ubuntu\nWindows Subsystem for Linux version: 2.5.9.0\n",
                            "",
                        ))
                    } else {
                        Ok(out(1, "", "Windows Subsystem for Linux is not installed."))
                    }
                }
                ["--list", "--verbose"] => {
                    if self.list_failure.load(Ordering::SeqCst) > 0 {
                        self.list_failure.fetch_sub(1, Ordering::SeqCst);
                        return Ok(out(1, "", "transient wsl failure"));
                    }
                    if !world.wsl_installed {
                        return Ok(out(1, "", "Windows Subsystem for Linux is not installed."));
                    }
                    let mut text = String::from("  NAME      STATE     VERSION\n");
                    for name in &world.distros {
                        text.push_str(&format!("  {name}    Stopped   2\n"));
                    }
                    Ok(out(0, &text, ""))
                }
                ["--import", name, _dir, _tar, "--version", "2"] => {
                    world.distros.push(name.to_string());
                    Ok(out(0, "", ""))
                }
                ["--terminate", _name] => Ok(out(0, "", "")),
                ["--unregister", name] => {
                    world.distros.retain(|d| d != name);
                    world.marker = None;
                    world.lightpanda_installed = false;
                    Ok(out(0, "", ""))
                }
                ["-d", _name, "-u", "root", "--", "cat", MARKER_PATH] => match &world.marker {
                    Some(marker) => Ok(out(0, &serde_json::to_string(marker).unwrap(), "")),
                    None => Ok(out(
                        1,
                        "",
                        &format!("cat: {MARKER_PATH}: No such file or directory"),
                    )),
                },
                ["-d", _name, "-u", "root", "--", "sh", "-c", fragment] => {
                    if fragment.contains("local-workspaces-runtime.json") {
                        let marker: OwnershipMarker =
                            serde_json::from_slice(request.stdin.as_deref().unwrap_or_default())
                                .map_err(|_| {
                                    CoreError::new(
                                        ErrorCode::BrowserRuntimeProvisionFailed,
                                        "bad marker stdin",
                                    )
                                })?;
                        world.marker = Some(marker);
                        Ok(out(0, "", ""))
                    } else if fragment.contains("wsl.conf")
                        || fragment.contains("mkdir -p /var/lib/local-workspaces")
                    {
                        Ok(out(0, "", ""))
                    } else if fragment.contains("cat > /opt/local-workspaces/lightpanda/") {
                        // A fresh install writes the pinned bytes. A re-install
                        // overwrites whatever was on disk. A first install with a
                        // pre-seeded (wrong) digest keeps it, simulating corrupt
                        // bytes landing on disk.
                        if world.lightpanda_installed || world.lightpanda_digest.is_empty() {
                            world.lightpanda_digest = TEST_LP_SHA.to_string();
                        }
                        world.lightpanda_installed = true;
                        Ok(out(0, "", ""))
                    } else if fragment.starts_with("ln -sfn") {
                        world.symlink_switched = true;
                        Ok(out(0, "", ""))
                    } else if fragment.starts_with("rm -f ") {
                        world.lightpanda_installed = false;
                        world.lightpanda_digest.clear();
                        Ok(out(0, "", ""))
                    } else {
                        Ok(out(1, "", "unexpected fragment"))
                    }
                }
                ["-d", _name, "-u", "root", "--", "sha256sum", path] => {
                    if world.lightpanda_installed {
                        Ok(out(
                            0,
                            &format!("{}  {}\n", world.lightpanda_digest, path),
                            "",
                        ))
                    } else {
                        Ok(out(1, "", "sha256sum: No such file or directory"))
                    }
                }
                ["-d", _name, "-u", "localworkspaces", "--", LIGHTPANDA_BIN, "version"] => {
                    if world.lightpanda_installed && world.symlink_switched {
                        Ok(out(0, "0.3.7\n", ""))
                    } else {
                        Ok(out(127, "", "not found"))
                    }
                }
                _ => Ok(out(1, "", "unexpected command")),
            }
        }

        fn launch_elevated_install(&self) -> Result<(), CoreError> {
            self.elevated.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FakeDownloader {
        mode: DownloaderMode,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    #[derive(Clone)]
    enum DownloaderMode {
        /// Writes bytes whose SHA-256 matches the expected digest.
        Ok,
        /// Simulates a digest mismatch.
        IntegrityFailure,
        /// Simulates a network failure.
        NetworkFailure,
    }

    type DownloaderHandles = (Arc<FakeDownloader>, Arc<Mutex<Vec<(String, String)>>>);

    impl FakeDownloader {
        fn new(mode: DownloaderMode) -> DownloaderHandles {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (
                Arc::new(Self {
                    mode,
                    calls: calls.clone(),
                }),
                calls,
            )
        }
    }

    #[async_trait]
    impl RuntimeDownloader for FakeDownloader {
        async fn download_verified(
            &self,
            url: &str,
            dest: &Path,
            expected_sha256: &str,
        ) -> Result<(), CoreError> {
            self.calls
                .lock()
                .await
                .push((url.to_string(), expected_sha256.to_string()));
            match &self.mode {
                DownloaderMode::NetworkFailure => Err(CoreError::new(
                    ErrorCode::BrowserRuntimeDownloadFailed,
                    "download failed",
                )),
                DownloaderMode::IntegrityFailure => Err(CoreError::new(
                    ErrorCode::BrowserRuntimeIntegrityFailed,
                    "sha256 mismatch",
                )),
                DownloaderMode::Ok => {
                    // Mimic the real downloader: no stale .partial may survive.
                    let partial = dest.with_file_name(format!(
                        "{}.partial",
                        dest.file_name().unwrap().to_string_lossy()
                    ));
                    if partial.exists() {
                        return Err(CoreError::new(
                            ErrorCode::BrowserRuntimeDownloadFailed,
                            "stale partial file present",
                        ));
                    }
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    std::fs::write(dest, b"fake-artifact").ok();
                    Ok(())
                }
            }
        }
    }

    struct FakeSmoke {
        ok: bool,
        calls: Arc<AtomicU32>,
    }

    #[async_trait]
    impl McpSmokeVerifier for FakeSmoke {
        async fn smoke_verify(&self, _distro_name: &str) -> Result<(), CoreError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.ok {
                Ok(())
            } else {
                Err(CoreError::new(
                    ErrorCode::BrowserRuntimeVersionMismatch,
                    "missing required native tools",
                ))
            }
        }
    }

    fn build_manager(
        tmp: &Path,
        runner: Arc<ScriptedRunner>,
        downloader: Arc<dyn RuntimeDownloader>,
        smoke_ok: bool,
    ) -> (Arc<BrowserRuntimeManager>, Arc<AtomicU32>) {
        let smoke_calls = Arc::new(AtomicU32::new(0));
        let manager = BrowserRuntimeManager::new(
            runner.clone(),
            downloader,
            Arc::new(FakeSmoke {
                ok: smoke_ok,
                calls: smoke_calls.clone(),
            }),
            test_manifest(),
            tmp.to_path_buf(),
        );
        (manager, smoke_calls)
    }

    fn healthy_world() -> FakeWorld {
        FakeWorld {
            wsl_installed: true,
            distros: vec![PREFERRED_DISTRO_NAME.to_string()],
            marker: None, // filled per test with the right install id
            lightpanda_installed: true,
            lightpanda_digest: TEST_LP_SHA.to_string(),
            symlink_switched: true,
        }
    }

    fn calls_contain(calls: &[Vec<String>], needle: &str) -> bool {
        calls
            .iter()
            .any(|argv| argv.iter().any(|a| a.contains(needle)))
    }

    // --- manifest validation ---------------------------------------------------

    #[test]
    fn manifest_validation_accepts_pinned_manifest() {
        assert!(test_manifest().validate().is_ok());
    }

    #[test]
    fn manifest_validation_rejects_placeholder_sha() {
        let mut m = test_manifest();
        m.distro_image_sha256 = "<64 lowercase hex>".to_string();
        assert!(m.validate().is_err());
    }

    #[test]
    fn manifest_validation_rejects_nightly_and_mutable_urls() {
        let mut m = test_manifest();
        m.lightpanda_version = "nightly".to_string();
        assert!(m.validate().is_err());

        let mut m = test_manifest();
        m.distro_image_url = "https://example.com/latest/rootfs.tar.gz".to_string();
        assert!(m.validate().is_err());
    }

    #[test]
    fn manifest_from_json_str_parses_bundled_resource() {
        let json = serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "runtime_family": "lightpanda-wsl",
            "lightpanda_version": "0.3.7",
            "lightpanda_arch": "x86_64-linux",
            "lightpanda_asset_url": "https://github.com/lightpanda-io/browser/releases/download/0.3.7/lightpanda-x86_64-linux",
            "lightpanda_sha256": TEST_LP_SHA,
            "distro_image_version": "lwsl-ubuntu-noble-minbase-v1",
            "distro_arch": "amd64",
            "distro_image_url": "https://github.com/quangtruong2003/MCP-Local/releases/download/runtime-lwsl-ubuntu-noble-minbase-v1/lightpanda-rootfs-ubuntu-noble-minbase-amd64.tar.gz",
            "distro_image_sha256": TEST_ROOTFS_SHA
        }))
        .unwrap();
        let manifest = RuntimeManifest::from_json_str(&json).unwrap();
        assert_eq!(manifest.lightpanda_version, "0.3.7");
    }

    // --- state machine -----------------------------------------------------------

    #[tokio::test]
    async fn healthy_reuse_reaches_ready_without_provisioning() {
        let tmp = tempfile::tempdir().unwrap();
        let world = healthy_world();
        let (runner, calls, world_ref) = ScriptedRunner::new(world);
        let (downloader, dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);

        // Seed the marker with the manager's install id (owned by this install).
        let install_id = manager.install_id_for_test().await;
        world_ref.lock().await.marker = Some(OwnershipMarker::new(install_id.clone()));
        // Seed persisted identity so reuse can verify the install location.
        manager.seed_identity_for_test().await;

        let state = manager.ensure_runtime().await.unwrap();
        assert_eq!(state, RuntimeProvisionState::Ready);
        assert_eq!(manager.current_state(), RuntimeProvisionState::Ready);
        assert_eq!(smoke_calls.load(Ordering::SeqCst), 1);

        let calls = calls.lock().await.clone();
        assert!(!calls_contain(&calls, "--import"));
        assert!(!calls_contain(&calls, "--unregister"));
        assert!(dl_calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn absent_distro_provisions_full_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let world = FakeWorld {
            wsl_installed: true,
            ..Default::default()
        };
        let (runner, calls, world_ref) = ScriptedRunner::new(world);
        let (downloader, dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);

        let state = manager.ensure_runtime().await.unwrap();
        assert_eq!(state, RuntimeProvisionState::Ready);

        // Rootfs + lightpanda downloaded with pinned digests.
        let downloads = dl_calls.lock().await.clone();
        assert_eq!(downloads.len(), 2);
        assert!(downloads[0].0.contains("lightpanda-rootfs"));
        assert_eq!(downloads[0].1, TEST_ROOTFS_SHA);
        assert!(downloads[1].0.contains("lightpanda-x86_64-linux"));
        assert_eq!(downloads[1].1, TEST_LP_SHA);

        let calls = calls.lock().await.clone();
        assert!(calls_contain(&calls, "--import"));
        assert!(calls_contain(&calls, "--version"));
        assert!(calls_contain(&calls, "wsl.conf"));
        assert!(calls_contain(&calls, "local-workspaces-runtime.json"));
        assert!(calls_contain(&calls, "--terminate"));
        assert!(calls_contain(&calls, "ln -sfn"));
        assert!(calls_contain(&calls, "sha256sum"));

        // Owned distro identity persisted with marker install id.
        let identity = manager.distro_identity().unwrap();
        assert_eq!(identity.distro_name, PREFERRED_DISTRO_NAME);
        let marker = world_ref.lock().await.marker.clone().unwrap();
        assert_eq!(marker.install_id, identity.install_id);
        assert_eq!(smoke_calls.load(Ordering::SeqCst), 1);

        // State file persisted atomically as ready.
        let state_file = tmp.path().join("wsl").join(RUNTIME_STATE_FILE);
        assert!(state_file.is_file());
    }

    #[tokio::test]
    async fn wsl_absent_triggers_elevated_install_and_reboot_state() {
        let tmp = tempfile::tempdir().unwrap();
        let world = FakeWorld::default(); // WSL not installed, stays absent
        let (runner, _calls, _world_ref) = ScriptedRunner::new(world);
        let (downloader, _dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);

        let err = manager.ensure_runtime().await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslRebootRequired);
        assert_eq!(runner.elevated.load(Ordering::SeqCst), 1);
        assert_eq!(
            manager.current_state(),
            RuntimeProvisionState::RebootRequired
        );
    }

    #[tokio::test]
    async fn reboot_resume_completes_provisioning_with_same_install_id() {
        let tmp = tempfile::tempdir().unwrap();
        let world = FakeWorld::default();
        let (runner, _calls, _world_ref) = ScriptedRunner::new(world);
        let (downloader, _dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);

        // First attempt: WSL absent -> reboot required.
        let err = manager.ensure_runtime().await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslRebootRequired);
        let install_id = manager.install_id_for_test().await;

        // "Reboot" happened: WSL is now healthy, distro still absent.
        runner.world.lock().await.wsl_installed = true;
        let state = manager.ensure_runtime().await.unwrap();
        assert_eq!(state, RuntimeProvisionState::Ready);
        // Install id survived the reboot round-trip.
        assert_eq!(manager.install_id_for_test().await, install_id);
        assert_eq!(runner.elevated.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rootfs_download_network_failure_surfaces_without_import() {
        let tmp = tempfile::tempdir().unwrap();
        let world = FakeWorld {
            wsl_installed: true,
            ..Default::default()
        };
        let (runner, calls, _world_ref) = ScriptedRunner::new(world);
        let (downloader, _dl_calls) = FakeDownloader::new(DownloaderMode::NetworkFailure);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);

        let err = manager.ensure_runtime().await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeDownloadFailed);
        let calls = calls.lock().await.clone();
        assert!(!calls_contain(&calls, "--import"));
        assert_ne!(manager.current_state(), RuntimeProvisionState::Ready);
    }

    #[tokio::test]
    async fn rootfs_checksum_failure_blocks_import() {
        let tmp = tempfile::tempdir().unwrap();
        let world = FakeWorld {
            wsl_installed: true,
            ..Default::default()
        };
        let (runner, calls, _world_ref) = ScriptedRunner::new(world);
        let (downloader, _dl_calls) = FakeDownloader::new(DownloaderMode::IntegrityFailure);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);

        let err = manager.ensure_runtime().await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeIntegrityFailed);
        let calls = calls.lock().await.clone();
        assert!(!calls_contain(&calls, "--import"));
        assert_ne!(manager.current_state(), RuntimeProvisionState::Ready);
    }

    #[tokio::test]
    async fn lightpanda_digest_mismatch_blocks_symlink_switch() {
        let tmp = tempfile::tempdir().unwrap();
        let world = FakeWorld {
            wsl_installed: true,
            ..Default::default()
        };
        let (runner, calls, world_ref) = ScriptedRunner::new(world);
        let (downloader, _dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);
        // The in-distro digest will not match the pinned manifest digest.
        world_ref.lock().await.lightpanda_digest = "deadbeef".repeat(8);

        let err = manager.ensure_runtime().await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeIntegrityFailed);
        let calls = calls.lock().await.clone();
        assert!(!calls_contain(&calls, "ln -sfn"));
        // The mismatched binary is deleted, never executed further.
        assert!(calls_contain(&calls, "rm -f"));
    }

    #[tokio::test]
    async fn interrupted_partial_download_is_discarded() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(
            staging.join("lightpanda-rootfs-lwsl-ubuntu-noble-minbase-v1.tar.gz.partial"),
            b"interrupted-bytes",
        )
        .unwrap();

        let world = FakeWorld {
            wsl_installed: true,
            ..Default::default()
        };
        let (runner, _calls, _world_ref) = ScriptedRunner::new(world);
        let (downloader, _dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);

        let state = manager.ensure_runtime().await.unwrap();
        assert_eq!(state, RuntimeProvisionState::Ready);
        let leftovers: Vec<_> = std::fs::read_dir(&staging)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "partial"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[tokio::test]
    async fn concurrent_ensure_runtime_is_serialized_single_flight() {
        let tmp = tempfile::tempdir().unwrap();
        let world = FakeWorld {
            wsl_installed: true,
            ..Default::default()
        };
        let (runner, _calls, _world_ref) = ScriptedRunner::new(world);
        let (downloader, dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);

        let m1 = manager.clone();
        let m2 = manager.clone();
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { m1.ensure_runtime().await }),
            tokio::spawn(async move { m2.ensure_runtime().await })
        );
        r1.unwrap().unwrap();
        r2.unwrap().unwrap();
        // Single flight: the rootfs is downloaded exactly once.
        let downloads = dl_calls.lock().await.clone();
        let rootfs_downloads = downloads
            .iter()
            .filter(|(url, _)| url.contains("lightpanda-rootfs"))
            .count();
        assert_eq!(rootfs_downloads, 1);
        assert_eq!(manager.current_state(), RuntimeProvisionState::Ready);
    }

    #[tokio::test]
    async fn transient_wsl_failure_surfaces_without_destructive_actions() {
        let tmp = tempfile::tempdir().unwrap();
        let world = healthy_world();
        let (runner, calls, world_ref) = ScriptedRunner::new(world);
        let (downloader, _dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);
        let install_id = manager.install_id_for_test().await;
        world_ref.lock().await.marker = Some(OwnershipMarker::new(install_id));
        manager.seed_identity_for_test().await;

        // One transient failure while listing distros.
        runner.list_failure.store(1, Ordering::SeqCst);
        let err = manager.ensure_runtime().await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslUnavailable);

        let calls = calls.lock().await.clone();
        assert!(!calls_contain(&calls, "--unregister"));
        assert!(!calls_contain(&calls, "--terminate"));
        assert!(!calls_contain(&calls, "--import"));

        // Next attempt recovers without rebuilding.
        let state = manager.ensure_runtime().await.unwrap();
        assert_eq!(state, RuntimeProvisionState::Ready);
    }

    #[tokio::test]
    async fn smoke_verification_failure_blocks_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let world = FakeWorld {
            wsl_installed: true,
            ..Default::default()
        };
        let (runner, _calls, _world_ref) = ScriptedRunner::new(world);
        let (downloader, _dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, false);

        let err = manager.ensure_runtime().await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeVersionMismatch);
        assert_ne!(manager.current_state(), RuntimeProvisionState::Ready);
    }

    #[tokio::test]
    async fn repair_rebuilds_lightpanda_when_digest_mismatched() {
        let tmp = tempfile::tempdir().unwrap();
        let world = healthy_world();
        let (runner, calls, world_ref) = ScriptedRunner::new(world);
        let (downloader, dl_calls) = FakeDownloader::new(DownloaderMode::Ok);
        let (manager, _smoke_calls) = build_manager(tmp.path(), runner.clone(), downloader, true);
        let install_id = manager.install_id_for_test().await;
        {
            let mut w = world_ref.lock().await;
            w.marker = Some(OwnershipMarker::new(install_id));
            // Corrupt the installed binary digest.
            w.lightpanda_digest = "0".repeat(64);
        }
        manager.seed_identity_for_test().await;

        let state = manager.repair_runtime().await.unwrap();
        assert_eq!(state, RuntimeProvisionState::Ready);
        // Repair re-downloaded lightpanda and re-switched the symlink.
        let downloads = dl_calls.lock().await.clone();
        assert!(downloads
            .iter()
            .any(|(url, _)| url.contains("lightpanda-x86_64-linux")));
        let calls = calls.lock().await.clone();
        assert!(calls_contain(&calls, "ln -sfn"));
        // Repair must not rebuild the owned distro when in-place repair works.
        assert!(!calls_contain(&calls, "--unregister"));
        assert!(!calls_contain(&calls, "--import"));
        // Digest restored to the pinned value.
        assert_eq!(world_ref.lock().await.lightpanda_digest, TEST_LP_SHA);
    }

    impl BrowserRuntimeManager {
        async fn install_id_for_test(&self) -> String {
            self.current.lock().await.install_id.clone()
        }

        async fn seed_identity_for_test(&self) {
            let mut current = self.current.lock().await;
            let identity = ManagedDistroIdentity {
                distro_name: PREFERRED_DISTRO_NAME.to_string(),
                install_id: current.install_id.clone(),
                install_dir: self.wsl_dir.join(PREFERRED_DISTRO_NAME),
            };
            current.distro = Some(identity);
        }
    }
}

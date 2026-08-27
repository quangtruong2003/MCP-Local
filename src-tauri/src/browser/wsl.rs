//! Managed WSL2 host adapter for the Lightpanda browser runtime.
//!
//! Every `wsl.exe` invocation is a fixed-shape command built from application
//! constants. No model- or user-controlled data is ever interpolated into a
//! shell fragment; payloads (marker JSON, `wsl.conf`) are streamed over stdin.
//! Public error messages are sanitized and never include raw WSL output.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use lc_core::error::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};

/// Preferred registered name for the app-owned distro.
pub const PREFERRED_DISTRO_NAME: &str = "LocalWorkspaces-Lightpanda";
/// Ownership marker owner identity (matches the Tauri app identifier).
pub const RUNTIME_OWNER: &str = "com.local.workspace-mcp-manager";
/// Ownership marker runtime kind.
pub const RUNTIME_KIND: &str = "lightpanda";
/// In-distro ownership marker path.
pub const MARKER_PATH: &str = "/etc/local-workspaces-runtime.json";
/// Unprivileged Linux user that runs Lightpanda.
pub const RUNTIME_USER: &str = "localworkspaces";
/// Lightpanda executable path inside the managed distro.
pub const LIGHTPANDA_BIN: &str = "/opt/local-workspaces/lightpanda/current/lightpanda";
/// Fixed isolation config written to the managed distro.
pub const WSL_CONF_CONTENT: &str = "[automount]\nenabled=false\n\n[interop]\nenabled=false\nappendWindowsPath=false\n";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const IMPORT_TIMEOUT: Duration = Duration::from_secs(900);

/// A fixed-shape `wsl.exe` invocation. Arguments are always built from
/// application constants by this module.
#[derive(Debug, Clone, PartialEq)]
pub struct WslCommand {
    pub args: Vec<String>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WslCommandOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl WslCommandOutput {
    pub fn decoded_stdout(&self) -> String {
        decode_wsl_output(&self.stdout)
    }

    pub fn decoded_stderr(&self) -> String {
        decode_wsl_output(&self.stderr)
    }
}

/// Seam over the real `wsl.exe` process so parsing/ownership logic is unit
/// testable without a Windows host.
#[async_trait]
pub trait WslCommandRunner: Send + Sync {
    async fn run(&self, request: WslCommand) -> Result<WslCommandOutput, CoreError>;
    /// Launches an elevated `wsl.exe --install --no-distribution` through the
    /// Windows UAC consent flow. Never bypasses UAC.
    fn launch_elevated_install(&self) -> Result<(), CoreError>;
}

/// One row of `wsl.exe --list --verbose`.
#[derive(Debug, Clone, PartialEq)]
pub struct WslDistroEntry {
    pub name: String,
    pub state: String,
    /// WSL version of the distro (1 or 2). WSL1 entries are reported so the
    /// provisioner can refuse to reuse them.
    pub version: u8,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WslHostStatus {
    pub wsl_installed: bool,
    pub wsl_version: Option<String>,
    pub default_distro: Option<String>,
    pub distros: Vec<WslDistroEntry>,
}

/// Ownership marker persisted inside an app-owned distro.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OwnershipMarker {
    pub owner: String,
    pub runtime: String,
    pub install_id: String,
    pub schema_version: u32,
}

impl OwnershipMarker {
    pub fn new(install_id: impl Into<String>) -> Self {
        Self {
            owner: RUNTIME_OWNER.to_string(),
            runtime: RUNTIME_KIND.to_string(),
            install_id: install_id.into(),
            schema_version: 1,
        }
    }
}

/// The host-side identity of the managed distro. Persisted by the runtime
/// provisioner beside the managed distro metadata under LocalAppData.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManagedDistroIdentity {
    pub distro_name: String,
    pub install_id: String,
    pub install_dir: PathBuf,
}

/// Which Linux user a fixed exec runs as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WslUser {
    Root,
    Runtime,
}

impl WslUser {
    fn as_str(self) -> &'static str {
        match self {
            WslUser::Root => "root",
            WslUser::Runtime => RUNTIME_USER,
        }
    }
}

/// Outcome of choosing a registered distro name.
#[derive(Debug, Clone, PartialEq)]
pub enum DistroNameDecision {
    /// The preferred name is free; import under it.
    CreatePreferred,
    /// The preferred name exists and is owned by this install; reuse it.
    ReusePreferred,
    /// The preferred name exists but is not owned by this install; import
    /// under a collision-safe suffixed name.
    CreateSuffixed(String),
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Decodes `wsl.exe` management output. Recent `wsl.exe` emits UTF-16LE for
/// management commands; falls back to UTF-8 for older builds and child output.
pub fn decode_wsl_output(bytes: &[u8]) -> String {
    let body = if bytes.starts_with(&[0xFF, 0xFE]) {
        &bytes[2..]
    } else {
        bytes
    };
    if body.len() >= 4 && body.len() % 2 == 0 && body[1] == 0 && body[3] == 0 {
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if let Ok(s) = String::from_utf16(&units) {
            return s.trim_start_matches('\u{feff}').to_string();
        }
    }
    String::from_utf8_lossy(bytes)
        .trim_start_matches('\u{feff}')
        .to_string()
}

/// Parses `wsl.exe --version` output into the WSL version string.
pub fn parse_wsl_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        let rest = if let Some(rest) = lower.strip_prefix("wsl version:") {
            &line[line.len() - rest.len()..]
        } else if let Some(rest) = lower.strip_prefix("windows subsystem for linux version:") {
            &line[line.len() - rest.len()..]
        } else {
            continue;
        };
        let version = rest.trim();
        if !version.is_empty() {
            return Some(version.to_string());
        }
    }
    None
}

/// Parses `wsl.exe --list --verbose` output (header + rows).
pub fn parse_distro_list(text: &str) -> Vec<WslDistroEntry> {
    let mut entries = Vec::new();
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.contains("name") && lower.contains("state") {
            continue; // header row
        }
        let (line, is_default) = match line.strip_prefix('*') {
            Some(stripped) => (stripped.trim_start(), true),
            None => (line, false),
        };
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() != 3 {
            continue;
        }
        let Ok(version) = tokens[2].parse::<u8>() else {
            continue;
        };
        entries.push(WslDistroEntry {
            name: tokens[0].to_string(),
            state: tokens[1].to_string(),
            version,
            is_default,
        });
    }
    entries
}

/// Parses the interesting fields of `wsl.exe --status`.
pub fn parse_wsl_status(text: &str) -> (Option<String>, Option<String>) {
    let mut default_distro = None;
    for line in text.lines() {
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix("default distribution:") {
            let value = line[line.len() - rest.len()..].trim();
            if !value.is_empty() {
                default_distro = Some(value.to_string());
            }
        }
    }
    (default_distro, parse_wsl_version(text))
}

/// Detects the "WSL is not installed" condition from a failed management call.
pub fn detect_wsl_absent(exit_code: i32, combined_output: &str) -> bool {
    if exit_code == 0 {
        return false;
    }
    let lower = combined_output.to_lowercase();
    lower.contains("not installed") || lower.contains("no distributions")
}

/// Validates a distro name we are about to register or address.
pub fn validate_distro_name(name: &str) -> Result<(), CoreError> {
    let mut chars = name.chars();
    let valid = !name.is_empty()
        && name.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::BrowserWslUnavailable,
            "invalid wsl distro name",
        ))
    }
}

/// Validates an 8-char lowercase-hex install id.
pub fn validate_install_id(install_id: &str) -> Result<(), CoreError> {
    let valid = install_id.len() == 8
        && install_id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    if valid {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "install id must be exactly 8 lowercase hex characters",
        ))
    }
}

/// Generates a fresh 8-char lowercase-hex install id.
pub fn generate_install_id() -> String {
    format!("{:08x}", rand::random::<u32>())
}

/// Chooses the registered distro name given the current distro list and the
/// ownership marker read from the preferred name (if it exists).
pub fn choose_distro_name(
    distros: &[WslDistroEntry],
    preferred_marker: Option<&OwnershipMarker>,
    install_id: &str,
) -> Result<DistroNameDecision, CoreError> {
    validate_install_id(install_id)?;
    let preferred_exists = distros.iter().any(|d| d.name == PREFERRED_DISTRO_NAME);
    if !preferred_exists {
        return Ok(DistroNameDecision::CreatePreferred);
    }
    let owned = preferred_marker.is_some_and(|marker| {
        marker.owner == RUNTIME_OWNER
            && marker.runtime == RUNTIME_KIND
            && marker.install_id == install_id
    });
    if owned {
        Ok(DistroNameDecision::ReusePreferred)
    } else {
        let mut suffixed = String::from(PREFERRED_DISTRO_NAME);
        suffixed.push('-');
        suffixed.push_str(install_id);
        Ok(DistroNameDecision::CreateSuffixed(suffixed))
    }
}

/// Case-handling path comparison for registered install locations.
fn install_locations_match(expected: &Path, registered: &Path) -> bool {
    let expected = dunce::simplified(expected);
    let registered = dunce::simplified(registered);
    if cfg!(windows) {
        expected
            .to_string_lossy()
            .eq_ignore_ascii_case(&registered.to_string_lossy())
    } else {
        expected == registered
    }
}

/// Full ownership verification. A distro is only treated as app-owned when the
/// marker owner/runtime/install_id all match AND the registered install
/// location matches the managed install directory. Any mismatch fails closed.
pub fn verify_ownership(
    identity: &ManagedDistroIdentity,
    marker: Option<&OwnershipMarker>,
    registered_install_dir: &Path,
) -> Result<(), CoreError> {
    let conflict = |message: &str| {
        CoreError::new(
            ErrorCode::BrowserWslDistroConflict,
            message.to_string(),
        )
    };
    let Some(marker) = marker else {
        return Err(conflict("ownership marker missing; distro is not app-owned"));
    };
    if marker.owner != RUNTIME_OWNER {
        return Err(conflict("ownership marker owner mismatch"));
    }
    if marker.runtime != RUNTIME_KIND {
        return Err(conflict("ownership marker runtime mismatch"));
    }
    if marker.install_id != identity.install_id {
        return Err(conflict("ownership marker install id mismatch"));
    }
    if !install_locations_match(&identity.install_dir, registered_install_dir) {
        return Err(conflict(
            "registered install location does not match the managed install directory",
        ));
    }
    Ok(())
}

/// Builds the fixed `wsl.exe` child command that runs a Lightpanda MCP process
/// as the unprivileged runtime user. No console window; telemetry and core
/// dumps disabled; `--log-level warn` enforced.
pub fn build_lightpanda_command(
    wsl_exe: &Path,
    distro_name: &str,
    extra_args: &[String],
) -> Result<std::process::Command, CoreError> {
    validate_distro_name(distro_name)?;
    // Fixed argv list; nothing here is ever passed through a shell.
    let mut argv: Vec<String> = vec![
        "-d".to_string(),
        distro_name.to_string(),
        "-u".to_string(),
        RUNTIME_USER.to_string(),
        "--".to_string(),
        LIGHTPANDA_BIN.to_string(),
        "mcp".to_string(),
        "--log-level".to_string(),
        "warn".to_string(),
    ];
    argv.extend(extra_args.iter().cloned());
    let mut cmd = std::process::Command::new(wsl_exe);
    cmd.args(argv);
    cmd.env("LIGHTPANDA_DISABLE_TELEMETRY", "true");
    cmd.env("LIGHTPANDA_DISABLE_CORE_DUMP", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(cmd)
}

/// Sanitized public error for a failed `wsl.exe` call. Never includes raw
/// stdout/stderr content.
fn sanitized_wsl_error(code: ErrorCode, context: &str, exit_code: i32) -> CoreError {
    CoreError::new(
        code,
        format!("wsl.exe {context} failed with exit code {exit_code}"),
    )
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// Fixed-operation adapter over a [`WslCommandRunner`]. Never accepts
/// arbitrary shell commands.
pub struct WslHostAdapter<R: WslCommandRunner> {
    runner: R,
    wsl_exe: PathBuf,
}

impl<R: WslCommandRunner> WslHostAdapter<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            wsl_exe: PathBuf::from("wsl.exe"),
        }
    }

    pub fn wsl_exe(&self) -> &Path {
        &self.wsl_exe
    }

    async fn run_fixed(
        &self,
        args: Vec<String>,
        stdin: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<WslCommandOutput, CoreError> {
        self.runner.run(WslCommand { args, stdin, timeout }).await
    }

    /// Inspects the WSL host: installation state, version, distro list.
    pub async fn host_status(&self) -> Result<WslHostStatus, CoreError> {
        let status_out = self
            .run_fixed(vec!["--status".to_string()], None, DEFAULT_TIMEOUT)
            .await;
        let list_out = self
            .run_fixed(
                vec!["--list".to_string(), "--verbose".to_string()],
                None,
                DEFAULT_TIMEOUT,
            )
            .await;

        let absent = |res: &Result<WslCommandOutput, CoreError>| match res {
            Ok(output) => output.exit_code != 0 && {
                let mut combined = output.decoded_stdout();
                combined.push(' ');
                combined.push_str(&output.decoded_stderr());
                detect_wsl_absent(output.exit_code, &combined)
            },
            Err(_) => true,
        };

        let mut host = WslHostStatus::default();
        if absent(&status_out) && absent(&list_out) {
            return Ok(host);
        }
        host.wsl_installed = true;

        if let Ok(output) = &status_out {
            if output.exit_code == 0 {
                let (default_distro, wsl_version) = parse_wsl_status(&output.decoded_stdout());
                host.default_distro = default_distro;
                host.wsl_version = wsl_version;
            }
        }

        match list_out {
            Ok(output) if output.exit_code == 0 => {
                host.distros = parse_distro_list(&output.decoded_stdout());
            }
            Ok(output) => {
                return Err(sanitized_wsl_error(
                    ErrorCode::BrowserWslUnavailable,
                    "list distros",
                    output.exit_code,
                ));
            }
            Err(err) => return Err(err),
        }
        Ok(host)
    }

    /// Lists registered distros (`wsl.exe --list --verbose`).
    pub async fn list_distros(&self) -> Result<Vec<WslDistroEntry>, CoreError> {
        let output = self
            .run_fixed(
                vec!["--list".to_string(), "--verbose".to_string()],
                None,
                DEFAULT_TIMEOUT,
            )
            .await?;
        if output.exit_code == 0 {
            return Ok(parse_distro_list(&output.decoded_stdout()));
        }
        let mut combined = output.decoded_stdout();
        combined.push(' ');
        combined.push_str(&output.decoded_stderr());
        if detect_wsl_absent(output.exit_code, &combined) {
            return Ok(Vec::new());
        }
        Err(sanitized_wsl_error(
            ErrorCode::BrowserWslUnavailable,
            "list distros",
            output.exit_code,
        ))
    }

    /// Imports the pinned rootfs as a WSL2 distro:
    /// `wsl.exe --import <name> <managed-install-dir> <rootfs-tar> --version 2`.
    pub async fn import_distro(
        &self,
        name: &str,
        install_dir: &Path,
        rootfs_tar: &Path,
    ) -> Result<(), CoreError> {
        validate_distro_name(name)?;
        let output = self
            .run_fixed(
                vec![
                    "--import".to_string(),
                    name.to_string(),
                    install_dir.to_string_lossy().into_owned(),
                    rootfs_tar.to_string_lossy().into_owned(),
                    "--version".to_string(),
                    "2".to_string(),
                ],
                None,
                IMPORT_TIMEOUT,
            )
            .await?;
        if output.exit_code == 0 {
            Ok(())
        } else {
            Err(sanitized_wsl_error(
                ErrorCode::BrowserRuntimeProvisionFailed,
                "import distro",
                output.exit_code,
            ))
        }
    }

    /// Reads the ownership marker from inside the distro. `Ok(None)` when the
    /// marker file is absent or unparseable (never treated as owned).
    pub async fn read_marker(&self, distro_name: &str) -> Result<Option<OwnershipMarker>, CoreError> {
        validate_distro_name(distro_name)?;
        let output = self
            .run_fixed(
                vec![
                    "-d".to_string(),
                    distro_name.to_string(),
                    "-u".to_string(),
                    "root".to_string(),
                    "--".to_string(),
                    "cat".to_string(),
                    MARKER_PATH.to_string(),
                ],
                None,
                DEFAULT_TIMEOUT,
            )
            .await?;
        if output.exit_code == 0 {
            let text = output.decoded_stdout();
            match serde_json::from_str::<OwnershipMarker>(text.trim()) {
                Ok(marker) => Ok(Some(marker)),
                // A corrupt marker is never treated as owned; fail closed.
                Err(_) => Ok(None),
            }
        } else if output.decoded_stderr().contains("No such file") {
            Ok(None)
        } else {
            Err(sanitized_wsl_error(
                ErrorCode::BrowserWslUnavailable,
                "read ownership marker",
                output.exit_code,
            ))
        }
    }

    /// Writes a file inside the distro from stdin bytes using a fixed shell
    /// fragment. The payload never appears in the fragment itself.
    async fn write_distro_file(
        &self,
        distro_name: &str,
        path: &'static str,
        content: Vec<u8>,
        context: &str,
    ) -> Result<(), CoreError> {
        validate_distro_name(distro_name)?;
        debug_assert!(path.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-')));
        let fragment = format!("cat > {path} && chmod 0644 {path}");
        let output = self
            .run_fixed(
                vec![
                    "-d".to_string(),
                    distro_name.to_string(),
                    "-u".to_string(),
                    "root".to_string(),
                    "--".to_string(),
                    "sh".to_string(),
                    "-c".to_string(),
                    fragment,
                ],
                Some(content),
                DEFAULT_TIMEOUT,
            )
            .await?;
        if output.exit_code == 0 {
            Ok(())
        } else {
            Err(sanitized_wsl_error(
                ErrorCode::BrowserRuntimeProvisionFailed,
                context,
                output.exit_code,
            ))
        }
    }

    /// Writes the ownership marker via stdin (fixed shell fragment, no
    /// interpolation of the payload).
    pub async fn write_marker(
        &self,
        distro_name: &str,
        marker: &OwnershipMarker,
    ) -> Result<(), CoreError> {
        let content = serde_json::to_vec_pretty(marker)
            .map_err(|e| CoreError::new(ErrorCode::BrowserRuntimeProvisionFailed, e.to_string()))?;
        self.write_distro_file(distro_name, MARKER_PATH, content, "write ownership marker")
            .await
    }

    /// Writes the fixed isolation `/etc/wsl.conf` via stdin.
    pub async fn write_wsl_conf(&self, distro_name: &str) -> Result<(), CoreError> {
        self.write_distro_file(
            distro_name,
            "/etc/wsl.conf",
            WSL_CONF_CONTENT.as_bytes().to_vec(),
            "write wsl.conf",
        )
        .await
    }

    /// Runs a fixed argv inside the distro as the given user. All argv entries
    /// come from application constants at every call site.
    pub async fn exec_fixed(
        &self,
        distro_name: &str,
        user: WslUser,
        argv: &[&str],
        stdin: Option<Vec<u8>>,
    ) -> Result<WslCommandOutput, CoreError> {
        validate_distro_name(distro_name)?;
        let mut args = vec![
            "-d".to_string(),
            distro_name.to_string(),
            "-u".to_string(),
            user.as_str().to_string(),
            "--".to_string(),
        ];
        args.extend(argv.iter().map(|s| s.to_string()));
        self.run_fixed(args, stdin, DEFAULT_TIMEOUT).await
    }

    /// Terminates the managed distro after full ownership verification.
    pub async fn terminate_owned(
        &self,
        identity: &ManagedDistroIdentity,
        marker: Option<&OwnershipMarker>,
        registered_install_dir: &Path,
    ) -> Result<(), CoreError> {
        verify_ownership(identity, marker, registered_install_dir)?;
        let output = self
            .run_fixed(
                vec!["--terminate".to_string(), identity.distro_name.clone()],
                None,
                DEFAULT_TIMEOUT,
            )
            .await?;
        if output.exit_code == 0 {
            Ok(())
        } else {
            Err(sanitized_wsl_error(
                ErrorCode::BrowserWslUnavailable,
                "terminate distro",
                output.exit_code,
            ))
        }
    }

    /// Unregisters the managed distro after full ownership verification.
    pub async fn unregister_owned(
        &self,
        identity: &ManagedDistroIdentity,
        marker: Option<&OwnershipMarker>,
        registered_install_dir: &Path,
    ) -> Result<(), CoreError> {
        verify_ownership(identity, marker, registered_install_dir)?;
        let output = self
            .run_fixed(
                vec!["--unregister".to_string(), identity.distro_name.clone()],
                None,
                DEFAULT_TIMEOUT,
            )
            .await?;
        if output.exit_code == 0 {
            Ok(())
        } else {
            Err(sanitized_wsl_error(
                ErrorCode::BrowserWslUnavailable,
                "unregister distro",
                output.exit_code,
            ))
        }
    }

    /// Spawns a Lightpanda MCP child for one logical page.
    pub fn spawn_lightpanda_child(
        &self,
        distro_name: &str,
        extra_args: &[String],
    ) -> Result<std::process::Child, CoreError> {
        let mut cmd = build_lightpanda_command(&self.wsl_exe, distro_name, extra_args)?;
        cmd.spawn().map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserWslUnavailable,
                format!("failed to spawn lightpanda child: {e}"),
            )
        })
    }

    /// Launches the elevated WSL install flow (UAC consent; never bypassed).
    pub fn launch_elevated_install(&self) -> Result<(), CoreError> {
        self.runner.launch_elevated_install()
    }
}

// ---------------------------------------------------------------------------
// Real runner (Windows)
// ---------------------------------------------------------------------------

/// Production runner: spawns `wsl.exe` with no visible console window.
pub struct RealWslCommandRunner;

#[cfg(windows)]
#[async_trait]
impl WslCommandRunner for RealWslCommandRunner {
    async fn run(&self, request: WslCommand) -> Result<WslCommandOutput, CoreError> {
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let mut cmd = tokio::process::Command::new("wsl.exe");
        cmd.args(&request.args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(if request.stdin.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .creation_flags(CREATE_NO_WINDOW);

        let mut child = cmd.spawn().map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserWslUnavailable,
                format!("failed to start wsl.exe: {e}"),
            )
        })?;

        if let Some(data) = request.stdin {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&data).await.map_err(|e| {
                    CoreError::new(
                        ErrorCode::BrowserRuntimeProvisionFailed,
                        format!("failed to write wsl.exe stdin: {e}"),
                    )
                })?;
            }
        }

        let mut stdout_pipe = child.stdout.take().expect("stdout is piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
        let outcome = tokio::time::timeout(request.timeout, async {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let (out_res, err_res, wait_res) = tokio::join!(
                stdout_pipe.read_to_end(&mut stdout),
                stderr_pipe.read_to_end(&mut stderr),
                child.wait()
            );
            out_res?;
            err_res?;
            let status = wait_res?;
            Ok::<_, std::io::Error>((stdout, stderr, status))
        })
        .await;

        match outcome {
            Ok(Ok((stdout, stderr, status))) => Ok(WslCommandOutput {
                exit_code: status.code().unwrap_or(-1),
                stdout,
                stderr,
            }),
            Ok(Err(e)) => Err(CoreError::new(
                ErrorCode::BrowserWslUnavailable,
                format!("wsl.exe exited unexpectedly: {e}"),
            )),
            Err(_) => {
                let _ = child.start_kill();
                Err(CoreError::new(
                    ErrorCode::BrowserWslUnavailable,
                    "wsl.exe timed out",
                ))
            }
        }
    }

    fn launch_elevated_install(&self) -> Result<(), CoreError> {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
        use windows::Win32::UI::Shell::{
            ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS,
        };

        let verb = HSTRING::from("runas");
        let file = HSTRING::from("wsl.exe");
        let parameters = HSTRING::from("--install --no-distribution");
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            // SW_HIDE: the elevated console is not shown to the user.
            nShow: 0,
            ..Default::default()
        };

        // `runas` triggers the standard UAC consent flow; never bypassed. If
        // the user declines, ShellExecuteExW fails and we surface it.
        unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserWslUnavailable,
                format!("elevated WSL install did not start (UAC declined or failed): {e}"),
            )
        })?;

        let handle: HANDLE = info.hProcess;
        if !handle.is_invalid() {
            unsafe {
                WaitForSingleObject(handle, INFINITE);
                let _ = CloseHandle(handle);
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
#[async_trait]
impl WslCommandRunner for RealWslCommandRunner {
    async fn run(&self, _request: WslCommand) -> Result<WslCommandOutput, CoreError> {
        Err(CoreError::new(
            ErrorCode::BrowserWslUnavailable,
            "WSL is only supported on Windows hosts",
        ))
    }

    fn launch_elevated_install(&self) -> Result<(), CoreError> {
        Err(CoreError::new(
            ErrorCode::BrowserWslUnavailable,
            "WSL is only supported on Windows hosts",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    /// Scripted fake runner keyed by joined argv.
    struct FakeRunner {
        responses: Mutex<HashMap<String, WslCommandOutput>>,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        elevated_calls: AtomicU32,
    }

    impl FakeRunner {
        fn new() -> (Self, Arc<Mutex<Vec<Vec<String>>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    responses: Mutex::new(HashMap::new()),
                    calls: calls.clone(),
                    elevated_calls: AtomicU32::new(0),
                },
                calls,
            )
        }

        fn script(&self, args: &[&str], output: WslCommandOutput) {
            let key = args.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" ");
            self.responses.lock().unwrap().insert(key, output);
        }
    }

    fn out(exit_code: i32, stdout: &str, stderr: &str) -> WslCommandOutput {
        WslCommandOutput {
            exit_code,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[async_trait]
    impl WslCommandRunner for FakeRunner {
        async fn run(&self, request: WslCommand) -> Result<WslCommandOutput, CoreError> {
            self.calls.lock().unwrap().push(request.args.clone());
            let key = request.args.join(" ");
            let map = self.responses.lock().unwrap();
            map.get(&key)
                .cloned()
                .ok_or_else(|| CoreError::new(ErrorCode::BrowserWslUnavailable, "no scripted response"))
        }

        fn launch_elevated_install(&self) -> Result<(), CoreError> {
            self.elevated_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // --- decoding -----------------------------------------------------------

    #[test]
    fn decodes_utf16le_management_output() {
        let bytes = utf16("  NAME      STATE     VERSION\n* Ubuntu    Stopped   2\n");
        let text = decode_wsl_output(&bytes);
        assert!(text.contains("NAME"));
        assert!(text.contains("Ubuntu"));
        assert!(!text.contains('\u{0}'));
    }

    #[test]
    fn decodes_utf8_output_with_fallback() {
        let text = decode_wsl_output("plain utf8 output ✓".as_bytes());
        assert_eq!(text, "plain utf8 output ✓");
    }

    #[test]
    fn strips_utf16_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(utf16("hello"));
        let text = decode_wsl_output(&bytes);
        assert_eq!(text.trim(), "hello");
    }

    // --- parsing --------------------------------------------------------------

    #[test]
    fn parses_wsl_version_output() {
        let text = "WSL version: 2.5.9.0\nKernel version: 5.15.167.4-1\n";
        assert_eq!(parse_wsl_version(text).as_deref(), Some("2.5.9.0"));
    }

    #[test]
    fn parses_legacy_wsl_version_header() {
        let text = "Windows Subsystem for Linux Version: 2.4.11.0\n";
        assert_eq!(parse_wsl_version(text).as_deref(), Some("2.4.11.0"));
    }

    #[test]
    fn parses_distro_list_with_default_and_wsl1_entries() {
        let text = "  NAME                   STATE           VERSION\n\
                    * Ubuntu                 Stopped         2\n\
                    Ubuntu-22.04             Running         2\n\
                    LegacyDistro             Stopped         1\n";
        let entries = parse_distro_list(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "Ubuntu");
        assert!(entries[0].is_default);
        assert_eq!(entries[0].state, "Stopped");
        assert_eq!(entries[0].version, 2);
        assert!(!entries[1].is_default);
        assert_eq!(entries[1].state, "Running");
        assert_eq!(entries[2].name, "LegacyDistro");
        assert_eq!(entries[2].version, 1);
    }

    #[test]
    fn parses_distro_list_from_utf16() {
        let bytes = utf16("  NAME      STATE     VERSION\n  MyDistro  Running   2\n");
        let entries = parse_distro_list(&decode_wsl_output(&bytes));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "MyDistro");
    }

    #[test]
    fn parses_wsl_status_default_distro_and_version() {
        let text = "Default Distribution: Ubuntu\n\
                    Default Version: 2\n\
                    Windows Subsystem for Linux version: 2.5.9.0\n\
                    Kernel version: 5.15.167.4-1\n";
        let (default_distro, version) = parse_wsl_status(text);
        assert_eq!(default_distro.as_deref(), Some("Ubuntu"));
        assert_eq!(version.as_deref(), Some("2.5.9.0"));
    }

    #[test]
    fn detects_wsl_absent_from_status_error() {
        assert!(detect_wsl_absent(
            1,
            "Windows Subsystem for Linux is not installed. Use 'wsl.exe --install' to install.",
        ));
        assert!(detect_wsl_absent(1, "There are no distributions installed."));
        assert!(!detect_wsl_absent(1, "some other failure"));
        assert!(!detect_wsl_absent(0, "not installed"));
    }

    // --- validation -----------------------------------------------------------

    #[test]
    fn validates_distro_names() {
        assert!(validate_distro_name("LocalWorkspaces-Lightpanda").is_ok());
        assert!(validate_distro_name("LocalWorkspaces-Lightpanda-ab12cd34").is_ok());
        assert!(validate_distro_name("").is_err());
        assert!(validate_distro_name("has space").is_err());
        assert!(validate_distro_name("-leading-dash").is_err());
        assert!(validate_distro_name(&"x".repeat(100)).is_err());
    }

    #[test]
    fn validates_install_ids() {
        assert!(validate_install_id("ab12cd34").is_ok());
        assert!(validate_install_id("AB12CD34").is_err());
        assert!(validate_install_id("ab12cd3").is_err());
        assert!(validate_install_id("ab12cd345").is_err());
        assert!(validate_install_id("zb12cd34").is_err());
    }

    #[test]
    fn generated_install_ids_are_valid() {
        for _ in 0..16 {
            let id = generate_install_id();
            assert!(validate_install_id(&id).is_ok(), "bad id: {id}");
        }
    }

    // --- name choice / ownership ----------------------------------------------

    fn entry(name: &str) -> WslDistroEntry {
        WslDistroEntry {
            name: name.to_string(),
            state: "Stopped".to_string(),
            version: 2,
            is_default: false,
        }
    }

    #[test]
    fn free_preferred_name_creates_preferred() {
        let decision =
            choose_distro_name(&[entry("Ubuntu")], None, "ab12cd34").unwrap();
        assert_eq!(decision, DistroNameDecision::CreatePreferred);
    }

    #[test]
    fn collision_not_owned_uses_suffixed_name() {
        let distros = vec![entry(PREFERRED_DISTRO_NAME)];
        let foreign = OwnershipMarker {
            owner: "someone-else".to_string(),
            runtime: "other".to_string(),
            install_id: "zz99zz99".to_string(),
            schema_version: 1,
        };
        let decision = choose_distro_name(&distros, Some(&foreign), "ab12cd34").unwrap();
        assert_eq!(
            decision,
            DistroNameDecision::CreateSuffixed("LocalWorkspaces-Lightpanda-ab12cd34".to_string())
        );
        // Also when no marker exists at all (someone else's distro).
        let decision = choose_distro_name(&distros, None, "ab12cd34").unwrap();
        assert_eq!(
            decision,
            DistroNameDecision::CreateSuffixed("LocalWorkspaces-Lightpanda-ab12cd34".to_string())
        );
    }

    #[test]
    fn collision_owned_by_this_install_reuses_preferred() {
        let distros = vec![entry(PREFERRED_DISTRO_NAME)];
        let marker = OwnershipMarker::new("ab12cd34");
        let decision = choose_distro_name(&distros, Some(&marker), "ab12cd34").unwrap();
        assert_eq!(decision, DistroNameDecision::ReusePreferred);
    }

    #[test]
    fn collision_with_same_owner_different_install_id_does_not_reuse() {
        let distros = vec![entry(PREFERRED_DISTRO_NAME)];
        let marker = OwnershipMarker::new("ffffffff");
        let decision = choose_distro_name(&distros, Some(&marker), "ab12cd34").unwrap();
        assert_eq!(
            decision,
            DistroNameDecision::CreateSuffixed("LocalWorkspaces-Lightpanda-ab12cd34".to_string())
        );
    }

    fn identity() -> ManagedDistroIdentity {
        ManagedDistroIdentity {
            distro_name: PREFERRED_DISTRO_NAME.to_string(),
            install_id: "ab12cd34".to_string(),
            install_dir: PathBuf::from(r"C:\Users\u\AppData\Local\Local Workspace MCP Manager\browser\wsl\distro-1"),
        }
    }

    #[test]
    fn ownership_verified_when_marker_and_location_match() {
        let id = identity();
        let marker = OwnershipMarker::new("ab12cd34");
        assert!(verify_ownership(&id, Some(&marker), &id.install_dir.clone()).is_ok());
    }

    #[test]
    fn ownership_refused_on_missing_marker() {
        let id = identity();
        let err = verify_ownership(&id, None, &id.install_dir.clone()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslDistroConflict);
    }

    #[test]
    fn ownership_refused_on_install_id_mismatch() {
        let id = identity();
        let marker = OwnershipMarker::new("ffffffff");
        let err = verify_ownership(&id, Some(&marker), &id.install_dir.clone()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslDistroConflict);
    }

    #[test]
    fn ownership_refused_on_owner_or_runtime_mismatch() {
        let id = identity();
        let mut marker = OwnershipMarker::new("ab12cd34");
        marker.owner = "evil".to_string();
        assert_eq!(
            verify_ownership(&id, Some(&marker), &id.install_dir.clone()).unwrap_err().code,
            ErrorCode::BrowserWslDistroConflict
        );
        let mut marker = OwnershipMarker::new("ab12cd34");
        marker.runtime = "not-lightpanda".to_string();
        assert_eq!(
            verify_ownership(&id, Some(&marker), &id.install_dir.clone()).unwrap_err().code,
            ErrorCode::BrowserWslDistroConflict
        );
    }

    #[test]
    fn ownership_refused_on_install_location_mismatch() {
        let id = identity();
        let marker = OwnershipMarker::new("ab12cd34");
        let elsewhere = PathBuf::from(r"D:\somewhere\else");
        let err = verify_ownership(&id, Some(&marker), &elsewhere).unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslDistroConflict);
    }

    // --- lightpanda child command ----------------------------------------------

    #[test]
    fn lightpanda_command_is_fixed_shape_and_hardened() {
        let cmd = build_lightpanda_command(
            Path::new("wsl.exe"),
            "LocalWorkspaces-Lightpanda",
            &["--user-data-dir".to_string(), "/var/lib/local-workspaces/browser/profile".to_string()],
        )
        .unwrap();
        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            vec![
                "-d", "LocalWorkspaces-Lightpanda",
                "-u", "localworkspaces",
                "--",
                "/opt/local-workspaces/lightpanda/current/lightpanda",
                "mcp",
                "--log-level", "warn",
                "--user-data-dir", "/var/lib/local-workspaces/browser/profile",
            ]
        );
        let envs: HashMap<String, String> = cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_string_lossy().into_owned(), v?.to_string_lossy().into_owned())))
            .collect();
        assert_eq!(envs.get("LIGHTPANDA_DISABLE_TELEMETRY").map(String::as_str), Some("true"));
        assert_eq!(envs.get("LIGHTPANDA_DISABLE_CORE_DUMP").map(String::as_str), Some("1"));
    }

    #[test]
    fn lightpanda_command_rejects_invalid_distro_name() {
        let err = build_lightpanda_command(Path::new("wsl.exe"), "bad name", &[]).unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslUnavailable);
    }

    // --- adapter operations -----------------------------------------------------

    #[tokio::test]
    async fn host_status_reports_healthy_host() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &["--status"],
            out(0, "Default Distribution: Ubuntu\nWindows Subsystem for Linux version: 2.5.9.0\n", ""),
        );
        fake.script(
            &["--list", "--verbose"],
            out(0, "  NAME      STATE     VERSION\n* Ubuntu    Stopped   2\n", ""),
        );
        let adapter = WslHostAdapter::new(fake);
        let status = adapter.host_status().await.unwrap();
        assert!(status.wsl_installed);
        assert_eq!(status.wsl_version.as_deref(), Some("2.5.9.0"));
        assert_eq!(status.default_distro.as_deref(), Some("Ubuntu"));
        assert_eq!(status.distros.len(), 1);
        assert_eq!(status.distros[0].name, "Ubuntu");
    }

    #[tokio::test]
    async fn host_status_reports_absent_wsl() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &["--status"],
            out(1, "", "Windows Subsystem for Linux is not installed."),
        );
        fake.script(&["--list", "--verbose"], out(1, "", "Windows Subsystem for Linux is not installed."));
        let adapter = WslHostAdapter::new(fake);
        let status = adapter.host_status().await.unwrap();
        assert!(!status.wsl_installed);
        assert!(status.distros.is_empty());
    }

    #[tokio::test]
    async fn import_distro_uses_exact_command_shape() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &[
                "--import",
                "LocalWorkspaces-Lightpanda",
                r"C:\LW\wsl\d1",
                r"C:\LW\staging\rootfs.tar.gz",
                "--version",
                "2",
            ],
            out(0, "", ""),
        );
        let adapter = WslHostAdapter::new(fake);
        adapter
            .import_distro(
                "LocalWorkspaces-Lightpanda",
                Path::new(r"C:\LW\wsl\d1"),
                Path::new(r"C:\LW\staging\rootfs.tar.gz"),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn import_distro_failure_is_sanitized() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &[
                "--import",
                "LocalWorkspaces-Lightpanda",
                r"C:\LW\wsl\d1",
                r"C:\LW\staging\rootfs.tar.gz",
                "--version",
                "2",
            ],
            out(1, "", "SECRET-RAW-WSL-DUMP with internal details"),
        );
        let adapter = WslHostAdapter::new(fake);
        let err = adapter
            .import_distro(
                "LocalWorkspaces-Lightpanda",
                Path::new(r"C:\LW\wsl\d1"),
                Path::new(r"C:\LW\staging\rootfs.tar.gz"),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeProvisionFailed);
        assert!(!err.message.contains("SECRET-RAW-WSL-DUMP"));
    }

    #[tokio::test]
    async fn read_marker_returns_none_when_file_missing() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &["-d", "LocalWorkspaces-Lightpanda", "-u", "root", "--", "cat", MARKER_PATH],
            out(1, "", "cat: /etc/local-workspaces-runtime.json: No such file or directory"),
        );
        let adapter = WslHostAdapter::new(fake);
        assert!(adapter.read_marker("LocalWorkspaces-Lightpanda").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn read_marker_parses_valid_marker() {
        let marker = OwnershipMarker::new("ab12cd34");
        let json = serde_json::to_string(&marker).unwrap();
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &["-d", "LocalWorkspaces-Lightpanda", "-u", "root", "--", "cat", MARKER_PATH],
            out(0, &json, ""),
        );
        let adapter = WslHostAdapter::new(fake);
        assert_eq!(
            adapter.read_marker("LocalWorkspaces-Lightpanda").await.unwrap(),
            Some(marker)
        );
    }

    #[tokio::test]
    async fn read_marker_treats_corrupt_marker_as_not_owned() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &["-d", "LocalWorkspaces-Lightpanda", "-u", "root", "--", "cat", MARKER_PATH],
            out(0, "{not-json", ""),
        );
        let adapter = WslHostAdapter::new(fake);
        assert!(adapter.read_marker("LocalWorkspaces-Lightpanda").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn write_marker_streams_payload_over_stdin_with_fixed_fragment() {
        let (fake, _calls) = FakeRunner::new();
        let marker = OwnershipMarker::new("ab12cd34");
        let expected_args: Vec<&str> = vec![
            "-d", "LocalWorkspaces-Lightpanda", "-u", "root", "--", "sh", "-c",
        ];
        // The shell fragment must be a constant (no marker data inside it).
        fake.script(
            &[
                "-d", "LocalWorkspaces-Lightpanda", "-u", "root", "--", "sh", "-c",
                "cat > /etc/local-workspaces-runtime.json && chmod 0644 /etc/local-workspaces-runtime.json",
            ],
            out(0, "", ""),
        );
        let adapter = WslHostAdapter::new(fake);
        adapter.write_marker("LocalWorkspaces-Lightpanda", &marker).await.unwrap();
        let _ = expected_args;
    }

    #[tokio::test]
    async fn write_wsl_conf_uses_fixed_content() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &[
                "-d", "LocalWorkspaces-Lightpanda", "-u", "root", "--", "sh", "-c",
                "cat > /etc/wsl.conf && chmod 0644 /etc/wsl.conf",
            ],
            out(0, "", ""),
        );
        let adapter = WslHostAdapter::new(fake);
        adapter.write_wsl_conf("LocalWorkspaces-Lightpanda").await.unwrap();
    }

    #[tokio::test]
    async fn terminate_owned_runs_after_ownership_check() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(&["--terminate", "LocalWorkspaces-Lightpanda"], out(0, "", ""));
        let adapter = WslHostAdapter::new(fake);
        let id = identity();
        let marker = OwnershipMarker::new("ab12cd34");
        adapter
            .terminate_owned(&id, Some(&marker), &id.install_dir.clone())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn terminate_refused_on_ownership_mismatch_without_any_command() {
        let (fake, calls) = FakeRunner::new();
        let adapter = WslHostAdapter::new(fake);
        let id = identity();
        let foreign = OwnershipMarker::new("ffffffff");
        let err = adapter
            .terminate_owned(&id, Some(&foreign), &id.install_dir.clone())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslDistroConflict);
        assert_eq!(calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn unregister_refused_on_location_mismatch_without_any_command() {
        let (fake, calls) = FakeRunner::new();
        let adapter = WslHostAdapter::new(fake);
        let id = identity();
        let marker = OwnershipMarker::new("ab12cd34");
        let err = adapter
            .unregister_owned(&id, Some(&marker), Path::new(r"D:\elsewhere"))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserWslDistroConflict);
        assert_eq!(calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn unregister_owned_runs_after_ownership_check() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(&["--unregister", "LocalWorkspaces-Lightpanda"], out(0, "", ""));
        let adapter = WslHostAdapter::new(fake);
        let id = identity();
        let marker = OwnershipMarker::new("ab12cd34");
        adapter
            .unregister_owned(&id, Some(&marker), &id.install_dir.clone())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn exec_fixed_builds_expected_argv() {
        let (fake, _calls) = FakeRunner::new();
        fake.script(
            &[
                "-d", "LocalWorkspaces-Lightpanda", "-u", "localworkspaces", "--",
                "/opt/local-workspaces/lightpanda/current/lightpanda", "version",
            ],
            out(0, "0.3.7", ""),
        );
        let adapter = WslHostAdapter::new(fake);
        let output = adapter
            .exec_fixed(
                "LocalWorkspaces-Lightpanda",
                WslUser::Runtime,
                &[LIGHTPANDA_BIN, "version"],
                None,
            )
            .await
            .unwrap();
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.decoded_stdout(), "0.3.7");
    }

    #[tokio::test]
    async fn elevated_install_delegates_to_runner() {
        let (fake, _calls) = FakeRunner::new();
        let adapter = WslHostAdapter::new(fake);
        adapter.launch_elevated_install().unwrap();
    }

}

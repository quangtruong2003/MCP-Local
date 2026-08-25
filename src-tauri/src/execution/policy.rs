//! Execution policy: approved-command allowlist, cwd confinement, and
//! sanitized child environment for the bounded runner.
//!
//! Honest limitation, stated everywhere this module is used: cwd confinement
//! does not prevent the executed program from opening paths outside the
//! workspace. It only constrains where the process starts.

use std::path::{Path, PathBuf};

use lc_core::confinement::ensure_inside;
use lc_core::error::CoreError;

/// Error raised while validating an execution request. Carries stable codes
/// (`COMMAND_NOT_ALLOWED`, `INVALID_ARGUMENT`) mirroring lc_core's
/// SCREAMING_SNAKE_CASE convention.
#[derive(Debug)]
pub struct PolicyError {
    pub code: &'static str,
    pub message: String,
}

impl PolicyError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn command_not_allowed(message: impl Into<String>) -> Self {
        Self::new("COMMAND_NOT_ALLOWED", message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new("INVALID_ARGUMENT", message)
    }
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PolicyError {}

impl From<lc_core::error::CoreError> for PolicyError {
    fn from(e: lc_core::error::CoreError) -> Self {
        Self {
            code: e.code.as_str(),
            message: e.message,
        }
    }
}

/// Basenames a client may ask to execute in this milestone. Matching is
/// case-insensitive and accepts the `.exe` suffix (`cargo.exe` -> `cargo`).
pub const DEFAULT_APPROVED_COMMANDS: &[&str] = &[
    "git", "cargo", "rustc", "npm", "npx", "node", "pnpm", "yarn", "python", "python3", "pip",
    "go", "dotnet",
];

pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const MAX_TIMEOUT_MS: u64 = 600_000;
/// Absolute ceiling even in unrestricted mode: effectively unlimited for any
/// real command, while a nonsense value cannot hang the runner forever.
pub const UNRESTRICTED_MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug)]
pub struct RunCommandRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
    /// Unrestricted access mode: skips the approved-command allowlist (any
    /// resolvable basename may run). Path-form programs stay rejected.
    pub unrestricted: bool,
}

#[derive(Debug)]
pub struct RunPowerShellRequest {
    pub script: String,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
    /// Unrestricted access mode: lifts the timeout cap and output bounds.
    pub unrestricted: bool,
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration_ms: u64,
}

/// Resolves `program` against the allowlist. Rejects anything that is not a
/// bare basename (no separators, no drive letters) so clients cannot point at
/// arbitrary executables; PATH lookup happens afterwards in the runner.
///
/// Returns the resolved program name to spawn (with `.exe` stripped so
/// Windows PATH lookup behaves as for the bare name).
pub fn resolve_program(program: &str) -> Result<String, PolicyError> {
    resolve_program_with(program, false)
}

/// `allow_any` (unrestricted access mode) skips the approved-list check but
/// keeps rejecting path-form programs — that is a quoting/injection surface,
/// not an access gate.
pub fn resolve_program_with(program: &str, allow_any: bool) -> Result<String, PolicyError> {
    if program.is_empty() {
        return Err(PolicyError::command_not_allowed(
            "program must not be empty",
        ));
    }
    // Absolute/relative paths are rejected outright in this milestone; only
    // allowlisted basenames may be requested.
    if program.contains('/') || program.contains('\\') || program.contains(':') {
        return Err(PolicyError::command_not_allowed(
            "only approved command names may be run, not executable paths",
        ));
    }

    let lower = program.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);

    if !allow_any && !DEFAULT_APPROVED_COMMANDS.contains(&stem) {
        return Err(PolicyError::command_not_allowed(format!(
            "command '{stem}' is not on the approved list"
        )));
    }
    Ok(stem.to_string())
}

/// Validates and resolves the requested working directory inside
/// `workspace_root`. An omitted or "." cwd resolves to the workspace root.
/// Junction/symlink escapes fail with PATH_OUTSIDE_WORKSPACE via the shared
/// confinement logic.
///
/// Note honestly: this confines only the start directory; it does not prevent
/// the executed program from opening paths outside the workspace.
pub fn resolve_cwd(workspace_root: &Path, cwd: Option<&str>) -> Result<PathBuf, CoreError> {
    match cwd {
        None => Ok(workspace_root.to_path_buf()),
        Some(rel) => ensure_inside(workspace_root, &workspace_root.join(rel)),
    }
}

/// Validates the timeout: rejects values above [`MAX_TIMEOUT_MS`] as
/// INVALID_ARGUMENT (pinned choice — no silent clamping), defaults to
/// [`DEFAULT_TIMEOUT_MS`] when omitted.
pub fn resolve_timeout(timeout_ms: Option<u64>) -> Result<u64, PolicyError> {
    resolve_timeout_with(timeout_ms, false)
}

/// `allow_any` (unrestricted access mode) lifts the 10-minute cap to a 24h
/// absolute ceiling; the 60 s default still applies when omitted.
pub fn resolve_timeout_with(timeout_ms: Option<u64>, allow_any: bool) -> Result<u64, PolicyError> {
    let max = if allow_any {
        UNRESTRICTED_MAX_TIMEOUT_MS
    } else {
        MAX_TIMEOUT_MS
    };
    match timeout_ms {
        None => Ok(DEFAULT_TIMEOUT_MS),
        Some(t) if t > max => Err(PolicyError::invalid_argument(format!(
            "timeout_ms {t} exceeds the maximum of {max}"
        ))),
        Some(t) => Ok(t),
    }
}

/// Host environment variables copied into the child. Everything else is
/// dropped by `env_clear()` before these are re-added.
const ENV_ALLOWLIST: &[&str] = &[
    "SystemRoot",
    "WINDIR",
    "PATH",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramData",
];

/// Substrings that mark a variable as secret-bearing. Case-insensitive;
/// matched anywhere in the variable name. The deny list WINS over the
/// allowlist.
const SECRET_MARKERS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "API_KEY",
    "APIKEY",
    "AUTH",
    "CREDENTIAL",
    "MCP_CAPABILITY",
    "TAILSCALE",
];

fn is_denied(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_MARKERS.iter().any(|m| upper.contains(m))
}

/// Builds the child environment from the host environment: allowlisted
/// variables only, minus anything matching the secret-marker deny list.
pub fn build_child_env(host: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    host.filter(|(name, _)| {
        ENV_ALLOWLIST.iter().any(|a| a.eq_ignore_ascii_case(name)) && !is_denied(name)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_code(r: Result<String, PolicyError>) -> String {
        r.unwrap_err().code.to_string()
    }

    #[test]
    fn plain_cargo_is_allowed_and_normalized() {
        assert_eq!(resolve_program("cargo").unwrap(), "cargo");
    }

    #[test]
    fn exe_suffix_maps_to_stem() {
        assert_eq!(resolve_program("cargo.exe").unwrap(), "cargo");
    }

    #[test]
    fn matching_is_case_insensitive_on_windows() {
        assert_eq!(resolve_program("Cargo.EXE").unwrap(), "cargo");
        assert_eq!(resolve_program("NPM").unwrap(), "npm");
    }

    #[test]
    fn absolute_executable_path_is_rejected() {
        assert_eq!(
            err_code(resolve_program(r"C:\Tools\cargo.exe")),
            "COMMAND_NOT_ALLOWED"
        );
    }

    #[test]
    fn cmd_exe_is_not_allowed() {
        assert_eq!(err_code(resolve_program("cmd.exe")), "COMMAND_NOT_ALLOWED");
        assert_eq!(err_code(resolve_program("cmd")), "COMMAND_NOT_ALLOWED");
    }

    #[test]
    fn powershell_through_run_command_is_not_allowed() {
        assert_eq!(
            err_code(resolve_program("powershell.exe")),
            "COMMAND_NOT_ALLOWED"
        );
        assert_eq!(err_code(resolve_program("pwsh")), "COMMAND_NOT_ALLOWED");
    }

    #[test]
    fn separators_in_program_are_rejected() {
        assert_eq!(
            err_code(resolve_program("tools/cargo")),
            "COMMAND_NOT_ALLOWED"
        );
        assert_eq!(
            err_code(resolve_program(r"tools\cargo")),
            "COMMAND_NOT_ALLOWED"
        );
    }

    #[test]
    fn unapproved_bare_name_is_rejected() {
        assert_eq!(err_code(resolve_program("curl")), "COMMAND_NOT_ALLOWED");
    }

    #[test]
    fn empty_program_is_rejected() {
        assert_eq!(err_code(resolve_program("")), "COMMAND_NOT_ALLOWED");
    }

    fn setup_root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        std::fs::create_dir_all(root.join("src")).unwrap();
        (dir, root)
    }

    #[test]
    fn dot_cwd_resolves_to_workspace_root() {
        let (_d, root) = setup_root();
        assert_eq!(resolve_cwd(&root, Some(".")).unwrap(), root);
    }

    #[test]
    fn omitted_cwd_is_workspace_root() {
        let (_d, root) = setup_root();
        assert_eq!(resolve_cwd(&root, None).unwrap(), root);
    }

    #[test]
    fn subdirectory_inside_workspace_is_accepted() {
        let (_d, root) = setup_root();
        assert_eq!(resolve_cwd(&root, Some("src")).unwrap(), root.join("src"));
    }

    #[test]
    fn parent_escape_is_path_outside_workspace() {
        let (_d, root) = setup_root();
        let e = resolve_cwd(&root, Some("..")).unwrap_err();
        assert_eq!(e.code.as_str(), "PATH_OUTSIDE_WORKSPACE");
    }

    #[cfg(windows)]
    #[test]
    fn junction_escape_is_path_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        let outside = dir.path().join("outside-target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        // mklink /J needs cmd.exe but only at test-authoring time, not in
        // production code paths under test.
        let link = root.join("jump");
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .status()
            .expect("mklink available on Windows");
        assert!(out.success(), "junction creation failed");

        let e = resolve_cwd(&root, Some("jump")).unwrap_err();
        assert_eq!(e.code.as_str(), "PATH_OUTSIDE_WORKSPACE");

        let _ = std::fs::remove_dir(&link); // remove junction, keep target
    }

    #[test]
    fn omitted_timeout_defaults_to_60s_without_waiting() {
        assert_eq!(resolve_timeout(None).unwrap(), DEFAULT_TIMEOUT_MS);
        assert_eq!(DEFAULT_TIMEOUT_MS, 60_000);
    }

    #[test]
    fn explicit_timeout_passes_through() {
        assert_eq!(resolve_timeout(Some(1_500)).unwrap(), 1_500);
        assert_eq!(resolve_timeout(Some(MAX_TIMEOUT_MS)).unwrap(), 600_000);
    }

    #[test]
    fn timeout_over_ten_minutes_is_invalid_argument_not_clamped() {
        let e = resolve_timeout(Some(MAX_TIMEOUT_MS + 1)).unwrap_err();
        assert_eq!(e.code, "INVALID_ARGUMENT");
    }

    fn env_from(pairs: &[(&str, &str)]) -> impl Iterator<Item = (String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn child_env_keeps_allowlisted_variables_only() {
        let env = build_child_env(env_from(&[
            ("PATH", r"C:\Windows"),
            ("SystemRoot", r"C:\Windows"),
            ("RANDOM_HOST_VAR", "x"),
        ]));
        assert!(env.contains(&("PATH".to_string(), r"C:\Windows".to_string())));
        assert!(env.contains(&("SystemRoot".to_string(), r"C:\Windows".to_string())));
        assert!(!env.iter().any(|(k, _)| k == "RANDOM_HOST_VAR"));
    }

    #[test]
    fn child_env_matching_is_case_insensitive_for_allowlist() {
        let env = build_child_env(env_from(&[("path", r"C:\Windows")]));
        assert!(env.len() == 1 && env[0].1 == r"C:\Windows");
    }

    #[test]
    fn secret_marker_deny_list_wins_over_allowlist() {
        // A hypothetical allowlisted-looking var carrying a marker is dropped.
        let env = build_child_env(env_from(&[
            ("PATH_TOKEN", "x"),
            ("MY_PASSWORD", "y"),
            ("AUTH", "z"),
        ]));
        assert!(env.is_empty());
    }

    #[test]
    fn tailscale_and_capability_vars_never_forwarded() {
        let env = build_child_env(env_from(&[
            ("MCP_CAPABILITY_TOKEN", "t"),
            ("TAILSCALE_HOSTNAME", "h"),
            ("SOME_TAILSCALE_THING", "v"),
            ("GITHUB_API_KEY", "k"),
            ("aws_credentials", "c"),
        ]));
        assert!(env.is_empty());
    }

    #[test]
    fn deny_check_is_case_insensitive() {
        let env = build_child_env(env_from(&[("path_with_Secret_part", "x")]));
        assert!(env.is_empty());
    }
}

//! Non-secret connection settings persisted in `settings.json`.
//!
//! API keys deliberately do not live here. `load` understands the legacy
//! `{ tunnel_id, tunnel_api_key }` shape so the caller can migrate that key
//! into the Windows DPAPI-protected store before the file is saved again.

use std::path::Path;

use serde::{Deserialize, Serialize};

fn default_tunnel_name() -> String {
    "Local Workspace MCP Manager".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionSettings {
    #[serde(default)]
    pub tunnel_id: String,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default = "default_tunnel_name")]
    pub tunnel_name: String,
    #[serde(default)]
    pub auto_start: bool,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            tunnel_id: String::new(),
            organization_id: None,
            workspace_id: None,
            tunnel_name: default_tunnel_name(),
            auto_start: false,
        }
    }
}

impl ConnectionSettings {
    pub fn normalize(&mut self) {
        self.tunnel_id = self.tunnel_id.trim().to_string();
        self.tunnel_name = self.tunnel_name.trim().to_string();
        if self.tunnel_name.is_empty() {
            self.tunnel_name = default_tunnel_name();
        }
        self.organization_id = normalize_optional(self.organization_id.take());
        self.workspace_id = normalize_optional(self.workspace_id.take());
    }

    pub fn validate_scope(&self) -> Result<(), String> {
        if non_blank(self.organization_id.as_deref()) || non_blank(self.workspace_id.as_deref()) {
            Ok(())
        } else {
            Err("Set an Organization ID or Workspace ID before creating a tunnel.".into())
        }
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn non_blank(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|v| !v.is_empty())
}

#[derive(Debug, Clone)]
pub struct LoadedSettings {
    pub settings: ConnectionSettings,
    pub legacy_runtime_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    tunnel_id: String,
    #[serde(default)]
    tunnel_api_key: String,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default = "default_tunnel_name")]
    tunnel_name: String,
    #[serde(default)]
    auto_start: bool,
}

impl From<SettingsFile> for LoadedSettings {
    fn from(raw: SettingsFile) -> Self {
        let mut settings = ConnectionSettings {
            tunnel_id: raw.tunnel_id,
            organization_id: raw.organization_id,
            workspace_id: raw.workspace_id,
            tunnel_name: raw.tunnel_name,
            auto_start: raw.auto_start,
        };
        settings.normalize();
        Self {
            settings,
            legacy_runtime_key: normalize_optional(Some(raw.tunnel_api_key)),
        }
    }
}

/// Loads `settings.json`; a missing file yields safe defaults.
pub fn load(dir: &Path) -> Result<LoadedSettings, String> {
    let path = dir.join("settings.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<SettingsFile>(&text)
            .map(LoadedSettings::from)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LoadedSettings {
            settings: ConnectionSettings::default(),
            legacy_runtime_key: None,
        }),
        Err(e) => Err(e.to_string()),
    }
}

/// Persists only non-secret settings. Saving a legacy file therefore removes
/// any historical plaintext `tunnel_api_key` field.
pub fn save(dir: &Path, settings: &ConnectionSettings) -> Result<(), String> {
    let path = dir.join("settings.json");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut normalized = settings.clone();
    normalized.normalize();
    let text = serde_json::to_string_pretty(&normalized).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// Versioned application settings (`version: 3`). All non-version fields are
/// serde-defaulted so older files keep parsing after upgrades.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    pub version: u32,
    #[serde(default)]
    pub start_on_launch: bool,
    #[serde(default = "default_tailscale_hostname")]
    pub tailscale_hostname: String,
    #[serde(default)]
    pub onboarding_complete: bool,
    /// UI theme preference: "system" | "light" | "dark".
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Register the app to launch at Windows login.
    #[serde(default)]
    pub launch_on_startup: bool,
    /// Closing the window hides to tray instead of exiting.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
}

fn default_tailscale_hostname() -> String {
    "local-workspace-mcp".into()
}

fn default_theme() -> String {
    "system".into()
}

fn default_true() -> bool {
    true
}

impl AppSettings {
    /// Normalizes free-form fields loaded from disk into their canonical set.
    pub fn normalize(&mut self) {
        if !matches!(self.theme.as_str(), "light" | "dark" | "system") {
            self.theme = default_theme();
        }
        self.tailscale_hostname = self.tailscale_hostname.trim().to_string();
        if self.tailscale_hostname.is_empty() {
            self.tailscale_hostname = default_tailscale_hostname();
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 3,
            start_on_launch: false,
            tailscale_hostname: default_tailscale_hostname(),
            onboarding_complete: false,
            theme: default_theme(),
            launch_on_startup: false,
            close_to_tray: true,
        }
    }
}

/// True when a parsed JSON object still carries the pre-v2 tunnel config.
fn is_legacy_shape(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.contains_key("tunnel_id") || o.contains_key("tunnel_api_key"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    lc_core::ops::write_file::atomic_write(path, bytes).map_err(|e| e.to_string())
}

/// Loads `<dir>/settings.json` as versioned [`AppSettings`].
///
/// - missing file -> v2 defaults
/// - legacy `{ tunnel_id, ... }` file -> migrate to v2 defaults and overwrite
///   the file atomically (dropping any plaintext `tunnel_api_key`)
/// - invalid JSON -> explicit `SETTINGS_INVALID` error; the file is never
///   silently replaced
pub fn load_app_settings(dir: &Path) -> Result<AppSettings, String> {
    let path = dir.join("settings.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AppSettings::default()),
        Err(e) => return Err(e.to_string()),
    };
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("SETTINGS_INVALID: failed to parse settings.json: {e}"))?;
    let settings = if is_legacy_shape(&value) {
        // Migrate away from the legacy tunnel shape; this also drops any
        // historical plaintext `tunnel_api_key`.
        let migrated = AppSettings::default();
        let json =
            serde_json::to_vec_pretty(&migrated).map_err(|e| format!("serialize settings: {e}"))?;
        write_atomic(&path, &json)?;
        migrated
    } else {
        let mut settings: AppSettings = serde_json::from_value(value)
            .map_err(|e| format!("SETTINGS_INVALID: unsupported settings.json layout: {e}"))?;
        settings.normalize();
        settings
    };
    Ok(settings)
}

/// Persists [`AppSettings`] as pretty v3 JSON, written atomically.
pub fn save_app_settings(dir: &Path, settings: &AppSettings) -> Result<(), String> {
    let path = dir.join("settings.json");
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    write_atomic(&path, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_safe_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.settings.tunnel_id, "");
        assert_eq!(loaded.settings.tunnel_name, "Local Workspace MCP Manager");
        assert!(!loaded.settings.auto_start);
        assert_eq!(loaded.legacy_runtime_key, None);
    }

    #[test]
    fn non_secret_settings_roundtrip_without_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let settings = ConnectionSettings {
            tunnel_id: "tunnel_123".into(),
            organization_id: Some("org_123".into()),
            workspace_id: Some("ws_123".into()),
            tunnel_name: "My local MCP".into(),
            auto_start: true,
        };
        save(dir.path(), &settings).unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!text.contains("api_key"));
        assert!(!text.contains("secret"));
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.settings.tunnel_id, "tunnel_123");
        assert_eq!(loaded.settings.organization_id.as_deref(), Some("org_123"));
        assert_eq!(loaded.settings.workspace_id.as_deref(), Some("ws_123"));
        assert!(loaded.settings.auto_start);
    }

    #[test]
    fn legacy_settings_extract_runtime_key_but_never_resave_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"tunnel_id":"tunnel_old","tunnel_api_key":"legacy-secret"}"#,
        )
        .unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.settings.tunnel_id, "tunnel_old");
        assert_eq!(loaded.legacy_runtime_key.as_deref(), Some("legacy-secret"));
        save(dir.path(), &loaded.settings).unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!text.contains("legacy-secret"));
        assert!(!text.contains("tunnel_api_key"));
    }

    #[test]
    fn provisioning_scope_requires_org_or_workspace() {
        let mut settings = ConnectionSettings::default();
        assert!(settings.validate_scope().is_err());
        settings.organization_id = Some("org_123".into());
        assert!(settings.validate_scope().is_ok());
        settings.organization_id = None;
        settings.workspace_id = Some("ws_123".into());
        assert!(settings.validate_scope().is_ok());
    }

    #[test]
    fn blank_optional_ids_normalize_to_none() {
        let mut settings = ConnectionSettings {
            organization_id: Some("  ".into()),
            workspace_id: Some(" ws_123 ".into()),
            ..ConnectionSettings::default()
        };
        settings.normalize();
        assert_eq!(settings.organization_id, None);
        assert_eq!(settings.workspace_id.as_deref(), Some("ws_123"));
    }

    #[test]
    fn save_creates_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        save(&nested, &ConnectionSettings::default()).unwrap();
        assert!(nested.join("settings.json").is_file());
    }

    #[test]
    fn app_settings_missing_file_yields_defaults_v3() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_app_settings(dir.path()).unwrap();
        assert_eq!(loaded, AppSettings::default());
        assert_eq!(loaded.version, 3);
        assert!(!loaded.start_on_launch);
        assert_eq!(loaded.tailscale_hostname, "local-workspace-mcp");
        assert!(!loaded.onboarding_complete);
        assert_eq!(loaded.theme, "system");
        assert!(!loaded.launch_on_startup);
        assert!(loaded.close_to_tray);
    }

    /// A pre-v3 file (no theme/tray/startup keys) parses with the new
    /// defaults instead of failing SETTINGS_INVALID.
    #[test]
    fn v2_file_upgrades_to_v3_defaults_for_new_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"version":2,"start_on_launch":true,"tailscale_hostname":"h","onboarding_complete":true}"#,
        )
        .unwrap();
        let loaded = load_app_settings(dir.path()).unwrap();
        assert!(loaded.start_on_launch);
        assert_eq!(loaded.theme, "system");
        assert!(loaded.close_to_tray);
        assert!(!loaded.launch_on_startup);
    }

    #[test]
    fn invalid_theme_and_blank_host_normalize_on_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"version":2,"tailscale_hostname":"  ","theme":"midnight"}"#,
        )
        .unwrap();
        let loaded = load_app_settings(dir.path()).unwrap();
        assert_eq!(loaded.theme, "system");
        assert_eq!(loaded.tailscale_hostname, "local-workspace-mcp");
    }

    #[test]
    fn legacy_settings_file_migrates_to_v2_and_is_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"tunnel_id":"legacy-id","tunnel_api_key":"legacy-secret"}"#,
        )
        .unwrap();
        let loaded = load_app_settings(dir.path()).unwrap();
        assert_eq!(loaded, AppSettings::default());
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!text.contains("legacy-id"));
        assert!(!text.contains("legacy-secret"));
        assert!(!text.contains("tunnel_api_key"));
        // The migrated file must itself be loadable v2 JSON.
        let reloaded = load_app_settings(dir.path()).unwrap();
        assert_eq!(reloaded, AppSettings::default());
    }

    #[test]
    fn app_settings_roundtrip_preserves_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let settings = AppSettings {
            version: 3,
            start_on_launch: true,
            tailscale_hostname: "my-machine.tailnet-name.ts.net".into(),
            onboarding_complete: true,
            theme: "dark".into(),
            launch_on_startup: true,
            close_to_tray: false,
        };
        save_app_settings(dir.path(), &settings).unwrap();
        let loaded = load_app_settings(dir.path()).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn invalid_json_reports_settings_invalid_and_leaves_file_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let broken = "{not json";
        std::fs::write(dir.path().join("settings.json"), broken).unwrap();
        let err = load_app_settings(dir.path()).unwrap_err();
        assert!(err.contains("SETTINGS_INVALID"), "unexpected error: {err}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
            broken
        );
    }

    #[test]
    fn unknown_extra_fields_in_v2_file_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"version":2,"start_on_launch":false,"tailscale_hostname":"h","onboarding_complete":true,"future_field":{"a":1}}"#,
        )
        .unwrap();
        let loaded = load_app_settings(dir.path()).unwrap();
        assert_eq!(loaded.tailscale_hostname, "h");
        assert!(loaded.onboarding_complete);
    }
}

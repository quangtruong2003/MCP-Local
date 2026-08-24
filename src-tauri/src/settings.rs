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
}

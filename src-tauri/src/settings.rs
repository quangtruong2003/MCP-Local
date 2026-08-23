//! Persisted manager settings (tunnel credentials), stored as
//! `settings.json` inside the OS app-data directory.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub tunnel_id: String,
    #[serde(default)]
    pub tunnel_api_key: String,
}

/// Loads settings.json from `dir`; missing file yields defaults.
pub fn load(dir: &Path) -> Result<Settings, String> {
    let path = dir.join("settings.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Writes settings.json into `dir`, creating the directory if needed.
pub fn save(dir: &Path, settings: &Settings) -> Result<(), String> {
    let path = dir.join("settings.json");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let s = load(dir.path()).unwrap();
        assert_eq!(s.tunnel_id, "");
        assert_eq!(s.tunnel_api_key, "");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            tunnel_id: "t-123".into(),
            tunnel_api_key: "secret".into(),
        };
        save(dir.path(), &s).unwrap();
        let back = load(dir.path()).unwrap();
        assert_eq!(back.tunnel_id, "t-123");
        assert_eq!(back.tunnel_api_key, "secret");
    }

    #[test]
    fn save_creates_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        save(&nested, &Settings::default()).unwrap();
        assert!(nested.join("settings.json").is_file());
    }
}

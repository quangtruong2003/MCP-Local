use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRuntimeSettings {
    pub enabled: bool,
}

impl From<&crate::settings::AppSettings> for BrowserRuntimeSettings {
    fn from(settings: &crate::settings::AppSettings) -> Self {
        Self {
            enabled: settings.browser_automation_enabled,
        }
    }
}

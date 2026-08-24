use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How a workspace may execute commands. Stored as its snake_case name in
/// SQLite (`approved_commands` / `full_powershell`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    ApprovedCommands,
    #[serde(rename = "full_powershell")]
    FullPowerShell,
}

impl ExecutionMode {
    pub fn from_storage(s: &str) -> Result<Self, String> {
        match s {
            "approved_commands" => Ok(Self::ApprovedCommands),
            "full_powershell" => Ok(Self::FullPowerShell),
            other => Err(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ApprovedCommands => "approved_commands",
            Self::FullPowerShell => "full_powershell",
        }
    }
}

/// A registered workspace directory with its permission toggles.
/// Timestamps are ISO 8601 UTC strings.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub root_path: PathBuf,
    pub can_read: bool,
    pub can_write: bool,
    pub can_execute: bool,
    pub execution_mode: ExecutionMode,
    pub created_at: String,
    pub updated_at: String,
}

/// One audit log entry. `id` is assigned by the database on read-back.
/// `ts` is an ISO 8601 UTC string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    #[serde(default)]
    pub id: i64,
    pub ts: String,
    pub source: String, // "mcp" | "ui"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub op: String,
    pub decision: String, // "allow" | "deny" | "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_serializes() {
        let ws = Workspace {
            id: "w1".into(),
            name: "Docs".into(),
            root_path: std::path::PathBuf::from("C:\\proj"),
            can_read: true,
            can_write: false,
            can_execute: false,
            execution_mode: ExecutionMode::ApprovedCommands,
            created_at: "2026-08-23T00:00:00Z".into(),
            updated_at: "2026-08-23T00:00:00Z".into(),
        };
        let v = serde_json::to_value(&ws).unwrap();
        assert_eq!(v["can_write"], serde_json::Value::Bool(false));
        let back: Workspace = serde_json::from_value(v).unwrap();
        assert_eq!(back.name, "Docs");
    }

    #[test]
    fn execution_mode_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(ExecutionMode::ApprovedCommands).unwrap(),
            serde_json::json!("approved_commands")
        );
        assert_eq!(
            serde_json::to_value(ExecutionMode::FullPowerShell).unwrap(),
            serde_json::json!("full_powershell")
        );
        let back: ExecutionMode =
            serde_json::from_value(serde_json::json!("full_powershell")).unwrap();
        assert_eq!(back, ExecutionMode::FullPowerShell);
    }
}

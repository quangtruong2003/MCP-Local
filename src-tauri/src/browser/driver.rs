use lc_core::error::CoreError;
use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};

use crate::settings::BrowserEngine;

use super::types::BrowserLaunchSpec;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserDriverStatus {
    pub running: bool,
    pub actual_engine: BrowserEngine,
    pub sidecar_version: Option<String>,
    pub browser_version: Option<String>,
    /// Local diagnostics only. Browser MCP handlers intentionally omit this.
    pub child_pid: Option<u32>,
}

#[async_trait::async_trait]
pub trait BrowserDriver: Send + Sync {
    async fn start(&self, spec: BrowserLaunchSpec) -> Result<BrowserDriverStatus, CoreError>;
    async fn stop(&self) -> Result<(), CoreError>;
    async fn call(
        &self,
        tool: &'static str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, CoreError>;
    async fn chromium_performance_metrics(&self) -> Result<serde_json::Value, CoreError>;
}

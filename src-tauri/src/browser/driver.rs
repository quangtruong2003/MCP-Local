use std::sync::Arc;

use lc_core::error::CoreError;
use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::types::{BrowserEngine, BrowserLaunchSpec};

/// Launch spec for one logical page = one Lightpanda child process. Contains
/// only the app-generated page id and fixed WSL-owned distro paths; nothing
/// here is ever derived from MCP/model input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageLaunchSpec {
    pub page_id: String,
    /// Canonical cookie snapshot JSON path inside the managed distro.
    pub cookie_path: String,
    /// Page-specific exit cookie jar path inside the managed distro.
    pub cookie_jar_path: String,
    /// HTTP cache directory inside the managed distro.
    pub http_cache_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageDriverStatus {
    pub running: bool,
    pub page_id: String,
    /// Local diagnostics only. Browser MCP handlers intentionally omit this.
    pub child_pid: Option<u32>,
}

/// One-page browser driver. One logical page owns exactly one child process;
/// a sibling page crashing never affects this one.
#[async_trait::async_trait]
pub trait BrowserPageDriver: Send + Sync {
    async fn start(&self, spec: PageLaunchSpec) -> Result<PageDriverStatus, CoreError>;
    async fn call(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CoreError>;
    async fn stop(&self) -> Result<(), CoreError>;
    async fn is_alive(&self) -> bool;
    /// OS pid of the page child, for local diagnostics and integration tests
    /// only. Browser MCP handlers intentionally omit this.
    fn child_pid(&self) -> Option<u32> {
        None
    }
}

/// Creates one fresh page driver per logical page.
pub trait BrowserPageDriverFactory: Send + Sync {
    fn create(&self) -> Arc<dyn BrowserPageDriver>;
}

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
    /// Whether DevTools auto-open was part of the actual launch for `spec`.
    /// Drivers must report the launch-time state truthfully; the default is
    /// false so an unknown driver never claims visible DevTools.
    fn launch_devtools_auto_open(&self, _spec: &BrowserLaunchSpec) -> bool {
        false
    }
    async fn device_descriptor(&self, _name: &str) -> Result<serde_json::Value, CoreError> {
        Err(CoreError::new(
            lc_core::error::ErrorCode::BrowserFeatureUnsupported,
            "browser device presets are not available from this driver",
        ))
    }
}

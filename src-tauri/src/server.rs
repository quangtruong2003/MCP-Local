//! MCP tool server: 7 workspace-scoped file tools behind the fail-closed
//! audit pipeline.
//!
//! Note: requests rmcp rejects before reaching a handler (malformed args,
//! unknown tool names) produce a protocol error with no audit row; the
//! one-audit-row-per-request guarantee covers requests that reach these
//! handlers. Flagged for spec Task 15 ruling.

use axum::Router;
use lc_core::{error::CoreError, CoreServices};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    },
    ServerHandler,
};
use std::sync::Arc;

/// Serializes a CoreError as an `isError` tool result carrying the stable
/// error code.
fn err_result(e: &CoreError) -> CallToolResult {
    let payload = serde_json::json!({
        "code": e.code.as_str(),
        "message": e.message,
        "detail": e.detail,
    });
    CallToolResult::error(vec![ContentBlock::text(payload.to_string())])
}

fn ok_json<T: serde::Serialize>(v: &T) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(v).unwrap_or_default(),
    )])
}

#[derive(Clone)]
pub struct WorkspaceServer {
    core: Arc<CoreServices>,
}

// ---- argument structs (all take workspace_id first) ----

fn default_dot() -> String {
    ".".into()
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ListDirArgs {
    pub workspace_id: String,
    /// Path relative to the workspace root, e.g. "." or "src"
    #[serde(default = "default_dot")]
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct FindArgs {
    pub workspace_id: String,
    /// Glob pattern relative to the workspace root, e.g. "**/*.rs"
    pub pattern: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    pub workspace_id: String,
    pub query: String,
    /// Restrict search to this subtree
    #[serde(default)]
    pub path_prefix: Option<String>,
    #[serde(default)]
    pub case_sensitive: bool,
    /// Treat query as a regular expression (default: literal text)
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    pub workspace_id: String,
    pub path: String,
    #[serde(default)]
    pub offset_line: Option<u64>,
    #[serde(default)]
    pub max_lines: Option<u64>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ReadManyArgs {
    pub workspace_id: String,
    /// Up to 8 paths relative to the workspace root
    pub paths: Vec<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct CreateArgs {
    pub workspace_id: String,
    pub path: String,
    pub content: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct PatchArgs {
    pub workspace_id: String,
    pub path: String,
    /// Exact text to replace; must occur exactly once in the current file
    pub old_string: String,
    pub new_string: String,
    /// sha256 revision from the last read_file of this path
    pub base_revision: String,
}

// The #[tool_router] macro generates an associated `Self::tool_router()`
// consumed by #[tool_handler]; no runtime field is needed.
#[tool_router]
impl WorkspaceServer {
    pub fn new(core: Arc<CoreServices>) -> Self {
        Self { core }
    }

    #[tool(
        description = "List entries of a directory inside an approved workspace",
        annotations(read_only_hint = true)
    )]
    fn list_directory(&self, Parameters(a): Parameters<ListDirArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited(
            &self.core,
            "list_directory",
            &a.workspace_id,
            false,
            |_, root| lc_core::ops::list_dir::list_dir(&self.core, root, &a.path, a.recursive),
        ) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Find files by glob pattern inside an approved workspace",
        annotations(read_only_hint = true)
    )]
    fn find_files(&self, Parameters(a): Parameters<FindArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited(
            &self.core,
            "find_files",
            &a.workspace_id,
            false,
            |_, root| lc_core::ops::find_files::find_files(&self.core, root, &a.pattern),
        ) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Search text (literal or regex) across files in an approved workspace",
        annotations(read_only_hint = true)
    )]
    fn search_text(&self, Parameters(a): Parameters<SearchArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited(
            &self.core,
            "search_text",
            &a.workspace_id,
            false,
            |_, root| {
                lc_core::ops::search_text::search_text(
                    &self.core,
                    root,
                    &a.query,
                    a.path_prefix.as_deref(),
                    a.case_sensitive,
                    a.is_regex,
                    a.max_results,
                )
            },
        ) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Read a file (with line offset/limit). Returns sha256 revision for patching.",
        annotations(read_only_hint = true)
    )]
    fn read_file(&self, Parameters(a): Parameters<ReadArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited(
            &self.core,
            "read_file",
            &a.workspace_id,
            false,
            |_, root| {
                lc_core::ops::read_file::read_file(
                    &self.core,
                    root,
                    &a.path,
                    a.offset_line,
                    a.max_lines,
                )
            },
        ) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Read up to 8 files at once; per-file results",
        annotations(read_only_hint = true)
    )]
    fn read_files(&self, Parameters(a): Parameters<ReadManyArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited(
            &self.core,
            "read_files",
            &a.workspace_id,
            false,
            |_, root| lc_core::ops::read_file::read_files(&self.core, root, &a.paths),
        ) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Create a new file (fails if it exists)",
        annotations(read_only_hint = false)
    )]
    fn create_file(&self, Parameters(a): Parameters<CreateArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited(
            &self.core,
            "create_file",
            &a.workspace_id,
            true,
            |_, root| lc_core::ops::write_file::create_file(&self.core, root, &a.path, &a.content),
        ) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Replace one unique occurrence of old_string in a file. Requires base_revision from a prior read.",
        annotations(read_only_hint = false)
    )]
    fn apply_patch(&self, Parameters(a): Parameters<PatchArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited(
            &self.core,
            "apply_patch",
            &a.workspace_id,
            true,
            |_, root| {
                lc_core::ops::write_file::apply_patch(
                    &self.core,
                    root,
                    &lc_core::ops::write_file::PatchArgs {
                        rel: a.path.clone(),
                        old_string: a.old_string.clone(),
                        new_string: a.new_string.clone(),
                        base_revision: a.base_revision.clone(),
                    },
                )
            },
        ) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }
}

#[tool_handler]
impl ServerHandler for WorkspaceServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "File access scoped to user-approved workspace folders. \
                 Every tool takes workspace_id. Read tools return a `revision` \
                 (sha256) that apply_patch requires as base_revision.",
        )
    }
}

/// Bind axum to a free loopback port and serve /mcp. Returns the chosen port.
pub async fn serve(core: Arc<CoreServices>) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let service = StreamableHttpService::new(
        move || Ok(WorkspaceServer::new(core.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );
    let app = Router::new().nest_service("/mcp", service);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("mcp server error: {e}");
        }
    });
    Ok(port)
}

//! MCP tool server: 7 workspace-scoped file tools behind the fail-closed
//! audit pipeline.
//!
//! Note: requests rmcp rejects before reaching a handler (malformed args,
//! unknown tool names) produce a protocol error with no audit row; the
//! one-audit-row-per-request guarantee covers requests that reach these
//! handlers. Flagged for spec Task 15 ruling.

use axum::Router;
use lc_core::error::ErrorCode;
use lc_core::pipeline::{Audited, RequiredCapability};
use lc_core::{error::CoreError, CoreServices};
use rmcp::{
    handler::server::{common::schema_for_output, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ServerHandler,
};
use std::sync::Arc;

/// Builds a success result carrying both the pretty-printed text (for
/// clients that ignore schemas) and the typed `structuredContent` payload
/// (for clients that validate against the tool's outputSchema).
fn ok_structured<T: serde::Serialize>(v: &T) -> CallToolResult {
    let json = serde_json::to_value(v).unwrap_or(serde_json::Value::Null);
    let mut r = ok_json(&json);
    r.structured_content = Some(json);
    r
}

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

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct EmptyArgs {}

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

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct RunCommandArgs {
    pub workspace_id: String,
    /// Approved command basename, e.g. "git" — never a path
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory relative to the workspace root (default ".")
    #[serde(default)]
    pub cwd: Option<String>,
    /// Kill the process tree after this many milliseconds (max 600000)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct RunPowerShellArgs {
    pub workspace_id: String,
    /// PowerShell script executed via stdin (requires FullPowerShell mode)
    pub script: String,
    /// Working directory relative to the workspace root (default ".")
    #[serde(default)]
    pub cwd: Option<String>,
    /// Kill the process tree after this many milliseconds (max 600000)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Client-facing mirror of the runner's `ExecutionResult` (which stays
/// policy-internal and unserialized). ChatGPT gets partial output on timeout.
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct ExecutionOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration_ms: u64,
}

/// Maps a runner/policy failure to a stable CoreError. PolicyError codes that
/// are already stable pass through as-is (`COMMAND_NOT_ALLOWED`,
/// `INVALID_ARGUMENT`, and Core-inherited codes like
/// `PATH_OUTSIDE_WORKSPACE`); anything else is an execution failure.
fn execution_error(e: crate::execution::policy::PolicyError) -> CoreError {
    match e.code {
        "COMMAND_NOT_ALLOWED" => CoreError::new(ErrorCode::CommandNotAllowed, e.message),
        "INVALID_ARGUMENT" => CoreError::new(ErrorCode::InvalidArgument, e.message),
        "PATH_OUTSIDE_WORKSPACE" => CoreError::new(ErrorCode::PathOutsideWorkspace, e.message),
        _ => CoreError::new(ErrorCode::ExecutionFailed, e.message),
    }
}

// The #[tool_router] macro generates an associated `Self::tool_router()`
// consumed by #[tool_handler]; no runtime field is needed.
#[tool_router]
impl WorkspaceServer {
    pub fn new(core: Arc<CoreServices>) -> Self {
        Self { core }
    }

    #[tool(
        description = "List the approved workspaces. Call this first to get valid workspace_id values for every other tool.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<Vec<lc_core::models::Workspace>>()
    )]
    fn list_workspaces(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        match lc_core::workspace_repo::list_workspaces(&self.core.db) {
            Ok(ws) => ok_structured(&ws),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "List entries of a directory inside an approved workspace",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<lc_core::ops::list_dir::ListDirResult>()
    )]
    fn list_directory(&self, Parameters(a): Parameters<ListDirArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "list_directory",
            &a.workspace_id,
            false,
            Some(&a.path),
            Some(serde_json::json!({ "recursive": a.recursive })),
            |_, root| lc_core::ops::list_dir::list_dir(&self.core, root, &a.path, a.recursive),
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Find files by glob pattern inside an approved workspace",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<lc_core::ops::find_files::FindResult>()
    )]
    fn find_files(&self, Parameters(a): Parameters<FindArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "find_files",
            &a.workspace_id,
            false,
            None,
            Some(serde_json::json!({ "pattern": &a.pattern })),
            |_, root| lc_core::ops::find_files::find_files(&self.core, root, &a.pattern),
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Search text (literal or regex) across files in an approved workspace",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<lc_core::ops::search_text::SearchResult>()
    )]
    fn search_text(&self, Parameters(a): Parameters<SearchArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "search_text",
            &a.workspace_id,
            false,
            a.path_prefix.as_deref(),
            Some(serde_json::json!({ "regex": a.is_regex, "case_sensitive": a.case_sensitive })),
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
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Read a file (with line offset/limit). Returns sha256 revision for patching.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<lc_core::ops::read_file::ReadResult>()
    )]
    fn read_file(&self, Parameters(a): Parameters<ReadArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "read_file",
            &a.workspace_id,
            false,
            Some(&a.path),
            None,
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
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Read up to 8 files at once; per-file results",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<Vec<lc_core::ops::read_file::FileResult>>()
    )]
    fn read_files(&self, Parameters(a): Parameters<ReadManyArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "read_files",
            &a.workspace_id,
            false,
            None,
            Some(serde_json::json!({ "file_count": a.paths.len() })),
            |_, root| lc_core::ops::read_file::read_files(&self.core, root, &a.paths),
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Create a new file (fails if it exists)",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<lc_core::ops::write_file::CreateResult>()
    )]
    fn create_file(&self, Parameters(a): Parameters<CreateArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "create_file",
            &a.workspace_id,
            true,
            Some(&a.path),
            Some(serde_json::json!({ "bytes": a.content.len() })),
            |_, root| lc_core::ops::write_file::create_file(&self.core, root, &a.path, &a.content),
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Replace one unique occurrence of old_string in a file. Requires base_revision from a prior read.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<lc_core::ops::write_file::PatchResult>()
    )]
    fn apply_patch(&self, Parameters(a): Parameters<PatchArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "apply_patch",
            &a.workspace_id,
            true,
            Some(&a.path),
            None,
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
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Run an approved developer command (git, cargo, npm, node, python, ...) inside a workspace. Requires command execution to be enabled for the workspace.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<ExecutionOutput>()
    )]
    fn run_command(&self, Parameters(a): Parameters<RunCommandArgs>) -> CallToolResult {
        let detail_ctx = serde_json::json!({ "program": &a.program });
        match lc_core::pipeline::run_tool_with_capability(
            &self.core,
            "run_command",
            &a.workspace_id,
            RequiredCapability::Execute,
            None,
            Some(detail_ctx),
            |_ws, root| {
                // Note: powershell/pwsh are not on the approved-command list,
                // so the policy layer already rejects them — the only path to
                // PowerShell is run_powershell below.
                let req = crate::execution::policy::RunCommandRequest {
                    program: a.program.clone(),
                    args: a.args.clone(),
                    cwd: a.cwd.clone(),
                    timeout_ms: a.timeout_ms,
                };
                let result =
                    crate::execution::runner::run_command(root, req).map_err(execution_error)?;
                let audited = Audited::new(ExecutionOutput {
                    exit_code: result.exit_code,
                    stdout: result.stdout,
                    stderr: result.stderr,
                    timed_out: result.timed_out,
                    stdout_truncated: result.stdout_truncated,
                    stderr_truncated: result.stderr_truncated,
                    duration_ms: result.duration_ms,
                })
                .with_detail(serde_json::json!({
                    "program": a.program,
                    "arg_count": a.args.len(),
                    "cwd": a.cwd.as_deref().unwrap_or("."),
                    "exit_code": result.exit_code,
                    "duration_ms": result.duration_ms,
                    "timed_out": result.timed_out,
                    "stdout_truncated": result.stdout_truncated,
                    "stderr_truncated": result.stderr_truncated,
                }));
                Ok(audited)
            },
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Run a PowerShell script (via stdin) inside a workspace. Requires FullPowerShell execution mode; ApprovedCommands mode returns POWERSHELL_DISABLED.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<ExecutionOutput>()
    )]
    fn run_powershell(&self, Parameters(a): Parameters<RunPowerShellArgs>) -> CallToolResult {
        use lc_core::ops::sha256_hex;
        let script_bytes = a.script.as_bytes();
        let script_sha256 = sha256_hex(script_bytes);
        let detail_ctx = serde_json::json!({ "script_bytes": script_bytes.len() });
        match lc_core::pipeline::run_tool_with_capability(
            &self.core,
            "run_powershell",
            &a.workspace_id,
            RequiredCapability::Execute,
            None,
            Some(detail_ctx),
            |ws, root| {
                if ws.execution_mode != lc_core::models::ExecutionMode::FullPowerShell {
                    return Err(CoreError::new(
                        ErrorCode::PowershellDisabled,
                        format!("workspace '{}' does not allow full PowerShell", ws.name),
                    ));
                }
                let req = crate::execution::policy::RunPowerShellRequest {
                    script: a.script.clone(),
                    cwd: a.cwd.clone(),
                    timeout_ms: a.timeout_ms,
                };
                let result = crate::execution::runner::run_power_shell(root, req)
                    .map_err(execution_error)?;
                Ok(Audited::new(ExecutionOutput {
                    exit_code: result.exit_code,
                    stdout: result.stdout,
                    stderr: result.stderr,
                    timed_out: result.timed_out,
                    stdout_truncated: result.stdout_truncated,
                    stderr_truncated: result.stderr_truncated,
                    duration_ms: result.duration_ms,
                })
                .with_detail(serde_json::json!({
                    "script_bytes": script_bytes.len(),
                    "script_sha256": script_sha256,
                    "cwd": a.cwd.as_deref().unwrap_or("."),
                    "exit_code": result.exit_code,
                    "duration_ms": result.duration_ms,
                    "timed_out": result.timed_out,
                    "stdout_truncated": result.stdout_truncated,
                    "stderr_truncated": result.stderr_truncated,
                })))
            },
        ) {
            Ok(r) => ok_structured(&r),
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
///
/// rmcp defaults `allowed_hosts` to loopback only (DNS-rebinding defense).
/// Requests arrive through tunnels with the public tunnel hostname in
/// `Host`, so host validation is disabled here — access control stays at
/// the workspace/pipeline layer, and the server still binds loopback only.
pub async fn serve(core: Arc<CoreServices>) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let mut config = StreamableHttpServerConfig::default();
    config.allowed_hosts = vec![]; // empty = Host validation off (see doc comment)
    let service = StreamableHttpService::new(
        move || Ok(WorkspaceServer::new(core.clone())),
        LocalSessionManager::default().into(),
        config,
    );
    let app = Router::new().nest_service("/mcp", service);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("mcp server error: {e}");
        }
    });
    Ok(port)
}

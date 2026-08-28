//! MCP integration suite for the exact Lightpanda native browser tool surface.
//!
//! The harness wires the public MCP server to a fake [`BrowserNativeCaller`]
//! instead of a real runtime, so every test exercises the real public
//! handlers, `session_id` routing, navigation policy, and audit path while
//! controlling the upstream native response. No real browser is started.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use lc_core::error::CoreError;
use local_workspace_mcp_manager::{
    browser::{contract, http::UpstreamToolResponse, lightpanda::BrowserNativeCaller},
    screen_capture::{CaptureTarget, CapturedImage, ScreenCaptureProvider},
    settings::{AgentPolicyState, AgentSettingsController, AppSettings},
};
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, ContentBlock},
    transport::StreamableHttpClientTransport,
    ServiceExt,
};
use serde_json::{Map, Value};

/// Fake native backend: records every `call_native` and replays queued
/// upstream responses. When the queue is empty it returns a generic success so
/// simple forwarding tests need no setup.
#[derive(Default)]
struct FakeNativeBrowser {
    calls: tokio::sync::Mutex<Vec<(String, Map<String, Value>, Option<String>)>>,
    next: tokio::sync::Mutex<VecDeque<Result<UpstreamToolResponse, CoreError>>>,
}

impl FakeNativeBrowser {
    async fn push(&self, item: Result<UpstreamToolResponse, CoreError>) {
        self.next.lock().await.push_back(item);
    }

    async fn calls(&self) -> Vec<(String, Map<String, Value>, Option<String>)> {
        self.calls.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl BrowserNativeCaller for FakeNativeBrowser {
    async fn call_native(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
        session_id: Option<&str>,
    ) -> Result<UpstreamToolResponse, CoreError> {
        self.calls
            .lock()
            .await
            .push((tool.to_string(), arguments, session_id.map(str::to_string)));
        self.next.lock().await.pop_front().unwrap_or_else(|| {
            Ok(UpstreamToolResponse::Result(CallToolResult::success(vec![
                ContentBlock::text("ok"),
            ])))
        })
    }
}

#[derive(Default)]
struct EmptyCapture;
impl ScreenCaptureProvider for EmptyCapture {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>, CoreError> {
        Ok(vec![])
    }
    fn capture(
        &self,
        _target_id: &str,
        _max_edge: Option<u32>,
    ) -> Result<CapturedImage, CoreError> {
        unreachable!()
    }
}

async fn spawn(
    settings: AppSettings,
) -> (
    u16,
    tempfile::TempDir,
    Arc<lc_core::CoreServices>,
    Arc<FakeNativeBrowser>,
) {
    let dir = tempfile::tempdir().unwrap();
    local_workspace_mcp_manager::settings::save_app_settings(dir.path(), &settings).unwrap();
    let core = Arc::new(lc_core::CoreServices::open(&dir.path().join("test.db")).unwrap());
    let policy = Arc::new(AgentPolicyState::from_settings(&settings));
    let controller = Arc::new(AgentSettingsController::new(
        dir.path().to_path_buf(),
        policy.clone(),
    ));
    let fake = Arc::new(FakeNativeBrowser::default());
    let capture: Arc<dyn ScreenCaptureProvider> = Arc::new(EmptyCapture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let core_for_server = core.clone();
    let browser_for_server: Arc<dyn BrowserNativeCaller> = fake.clone();
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || {
            Ok(local_workspace_mcp_manager::server::WorkspaceServer::new(
                core_for_server.clone(),
                policy.clone(),
                controller.clone(),
                browser_for_server.clone(),
                capture.clone(),
            ))
        },
        Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
        Default::default(),
    );
    let app = axum::Router::new().nest_service("/mcp", service);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (port, dir, core, fake)
}

async fn client(port: u16) -> rmcp::service::RunningService<rmcp::service::RoleClient, ()> {
    ().serve(StreamableHttpClientTransport::from_uri(format!(
        "http://127.0.0.1:{port}/mcp"
    )))
    .await
    .unwrap()
}

fn arguments(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn matching_audit(core: &lc_core::CoreServices, op: &str) -> Vec<lc_core::models::AuditEvent> {
    lc_core::audit::list_recent(&core.db, 100)
        .unwrap()
        .into_iter()
        .filter(|row| row.op == op)
        .collect()
}

/// Text of the first content block of a result.
fn first_text(result: &CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.to_string())
        .unwrap_or_default()
}

fn assert_ok(result: &CallToolResult) {
    assert_ne!(result.is_error, Some(true), "expected success, got error");
}

// --- Structural input-schema parity helpers --------------------------------
//
// The public schema is generated by schemars (nullable arrays, `$ref`/`$defs`,
// no descriptions) while the pinned upstream schema uses plain types and marks
// optionality via `required`. These helpers reduce both sides to their
// effective type/enum/default/required facts before comparing.

fn resolve_ref(schema: &Value, root: &Value) -> Value {
    if let Some(ref_path) = schema.get("$ref").and_then(Value::as_str) {
        if let Some(pointer) = ref_path.strip_prefix('#') {
            if let Some(resolved) = root.pointer(pointer) {
                return resolved.clone();
            }
        }
    }
    schema.clone()
}

fn effective(schema: &Value, root: &Value) -> Value {
    let mut current = resolve_ref(schema, root);
    if let Some(branches) = current.get("anyOf").and_then(Value::as_array).cloned() {
        let non_null: Vec<Value> = branches
            .into_iter()
            .filter(|branch| branch.get("type").and_then(Value::as_str) != Some("null"))
            .map(|branch| resolve_ref(&branch, root))
            .collect();
        if non_null.len() == 1 {
            current = non_null.into_iter().next().unwrap();
        }
    }
    current
}

fn base_type(schema: &Value) -> Option<String> {
    match schema.get("type") {
        Some(Value::String(t)) => Some(t.clone()),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .map(str::to_string),
        _ => None,
    }
}

fn enum_values(schema: &Value) -> Option<BTreeSet<String>> {
    schema.get("enum").and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

fn required_set(schema: &Value) -> BTreeSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn assert_parity(tool: &str, public: &Value, pinned: &Value) {
    let empty = serde_json::Map::new();
    let public_props = public
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let pinned_props = pinned
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let public_names: BTreeSet<&String> = public_props.keys().collect();
    let pinned_names: BTreeSet<&String> = pinned_props.keys().collect();
    assert_eq!(public_names, pinned_names, "{tool}: property names differ");
    for name in &pinned_names {
        let public_prop = effective(&public_props[*name], public);
        let pinned_prop = effective(&pinned_props[*name], pinned);
        assert_eq!(
            base_type(&public_prop),
            base_type(&pinned_prop),
            "{tool}.{name}: type differs"
        );
        assert_eq!(
            enum_values(&public_prop),
            enum_values(&pinned_prop),
            "{tool}.{name}: enum values differ"
        );
        assert_eq!(
            public_prop.get("default"),
            pinned_prop.get("default"),
            "{tool}.{name}: default differs"
        );
    }
    assert_eq!(
        required_set(public),
        required_set(pinned),
        "{tool}: required set differs"
    );
}

#[tokio::test]
async fn browser_tool_discovery_is_exact_native_31() {
    let (port, _dir, _core, _fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let tools = client.list_all_tools().await.unwrap();

    // 53 legacy total - 39 legacy browser + 31 native browser = 45.
    assert_eq!(tools.len(), 45, "total discovery must move 53 -> 45");

    let browser_tools: Vec<&rmcp::model::Tool> = tools
        .iter()
        .filter(|tool| tool.name.starts_with("browser_"))
        .collect();
    let browser_names: BTreeSet<String> = browser_tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    assert_eq!(browser_names.len(), 31, "exactly 31 browser tools");
    assert!(
        browser_tools
            .iter()
            .all(|tool| tool.name.starts_with("browser_lightpanda_")),
        "every browser tool must be namespaced browser_lightpanda_*"
    );

    // Exact expected public set from the frozen contract mapping.
    let expected: BTreeSet<String> = contract::NATIVE_TOOL_MAPPINGS
        .iter()
        .map(|(_, public)| public.to_string())
        .collect();
    assert_eq!(
        browser_names, expected,
        "public browser tool set must match the contract mapping"
    );

    // Removed legacy names must be gone.
    for removed in [
        "browser_get_status",
        "browser_open",
        "browser_snapshot",
        "browser_tabs",
        "browser_screenshot",
        "browser_network",
        "browser_trace",
        "browser_run_steps",
    ] {
        assert!(
            !browser_names.contains(removed),
            "{removed} must be removed"
        );
    }

    // Upstream 0.3.7 advertises no outputSchema, so none may be published.
    for tool in &browser_tools {
        assert!(
            tool.output_schema.is_none(),
            "{} must not advertise an outputSchema",
            tool.name
        );
    }

    // Structural input-schema parity per tool, minus the routing session_id.
    let pinned: std::collections::HashMap<String, Value> = contract::pinned_contract()
        .unwrap()
        .tools
        .into_iter()
        .map(|tool| (tool.name, tool.input_schema))
        .collect();
    let public_to_native: std::collections::HashMap<String, String> =
        contract::NATIVE_TOOL_MAPPINGS
            .iter()
            .map(|(native, public)| (public.to_string(), native.to_string()))
            .collect();
    for tool in &browser_tools {
        let name = tool.name.to_string();
        let native = public_to_native
            .get(&name)
            .unwrap_or_else(|| panic!("{name} missing from contract mapping"));
        let mut public_schema = Value::Object((*tool.input_schema).clone());
        if let Some(props) = public_schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            props.remove("session_id");
        }
        let pinned_schema = pinned
            .get(native)
            .unwrap_or_else(|| panic!("pinned contract has no schema for {native}"));
        assert_parity(&name, &public_schema, pinned_schema);
    }
}

#[tokio::test]
async fn fill_forwards_value_but_audit_omits_it() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let secret = "hunter2-fill-secret";
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_fill").with_arguments(arguments(
                serde_json::json!({"selector": "#login", "value": secret}),
            )),
        )
        .await
        .unwrap();
    assert_ok(&result);
    let calls = fake.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "fill");
    assert_eq!(
        calls[0].1["value"], secret,
        "value must be forwarded unchanged"
    );
    let rows = matching_audit(&core, "browser_lightpanda_fill");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "allow");
    let detail = serde_json::to_string(rows[0].detail.as_ref().unwrap()).unwrap();
    assert!(
        !detail.contains(secret),
        "audit must not contain the fill value"
    );
}

#[tokio::test]
async fn evaluate_forwards_script_but_audit_omits_it() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let script = "document.title + '-secret-script'";
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_evaluate")
                .with_arguments(arguments(serde_json::json!({"script": script}))),
        )
        .await
        .unwrap();
    assert_ok(&result);
    let calls = fake.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "evaluate");
    assert_eq!(
        calls[0].1["script"], script,
        "script must be forwarded unchanged"
    );
    let rows = matching_audit(&core, "browser_lightpanda_evaluate");
    assert_eq!(rows.len(), 1);
    let detail = serde_json::to_string(rows[0].detail.as_ref().unwrap()).unwrap();
    assert!(
        !detail.contains(script),
        "audit must not contain the script"
    );
}

#[tokio::test]
async fn extract_forwards_schema_string_unchanged() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let schema = "{\"title\":{\"selector\":\"h1.head\"}}";
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_extract")
                .with_arguments(arguments(serde_json::json!({"schema": schema}))),
        )
        .await
        .unwrap();
    assert_ok(&result);
    let calls = fake.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "extract");
    assert_eq!(
        calls[0].1["schema"], schema,
        "schema must be forwarded unchanged"
    );
    let rows = matching_audit(&core, "browser_lightpanda_extract");
    assert_eq!(rows.len(), 1);
    let detail = serde_json::to_string(rows[0].detail.as_ref().unwrap()).unwrap();
    assert!(
        !detail.contains("h1.head"),
        "audit must not contain the schema"
    );
}

#[tokio::test]
async fn get_cookies_returns_native_text_but_audit_omits_it() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let cookie_text = "{\"cookies\":[{\"name\":\"sid\",\"value\":\"secret-cookie-abc\"}]}";
    fake.push(Ok(UpstreamToolResponse::Result(CallToolResult::success(
        vec![ContentBlock::text(cookie_text)],
    ))))
    .await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_get_cookies")
                .with_arguments(arguments(serde_json::json!({"all": true}))),
        )
        .await
        .unwrap();
    assert_ok(&result);
    assert!(
        first_text(&result).contains("secret-cookie-abc"),
        "native cookie text must reach the authorized caller"
    );
    let rows = matching_audit(&core, "browser_lightpanda_get_cookies");
    assert_eq!(rows.len(), 1);
    let detail = serde_json::to_string(rows[0].detail.as_ref().unwrap()).unwrap();
    assert!(
        !detail.contains("secret-cookie-abc"),
        "audit database must not contain cookie values"
    );
}

#[tokio::test]
async fn get_env_returns_native_text_but_audit_omits_it() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let env_text = "MY_VAR=secret-env-value-xyz";
    fake.push(Ok(UpstreamToolResponse::Result(CallToolResult::success(
        vec![ContentBlock::text(env_text)],
    ))))
    .await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_get_env")
                .with_arguments(arguments(serde_json::json!({"name": "MY_VAR"}))),
        )
        .await
        .unwrap();
    assert_ok(&result);
    assert!(
        first_text(&result).contains("secret-env-value-xyz"),
        "native env text must reach the authorized caller"
    );
    let rows = matching_audit(&core, "browser_lightpanda_get_env");
    assert_eq!(rows.len(), 1);
    let detail = serde_json::to_string(rows[0].detail.as_ref().unwrap()).unwrap();
    assert!(
        !detail.contains("secret-env-value-xyz"),
        "audit database must not contain env values"
    );
}

#[tokio::test]
async fn save_forwards_path_and_script_but_audit_omits_both() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let path = "results/secret-out.json";
    let script = "return { token: 'save-script-secret' }";
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_save").with_arguments(arguments(
                serde_json::json!({"path": path, "script": script}),
            )),
        )
        .await
        .unwrap();
    assert_ok(&result);
    let calls = fake.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "save");
    assert_eq!(calls[0].1["path"], path);
    assert_eq!(calls[0].1["script"], script);
    let rows = matching_audit(&core, "browser_lightpanda_save");
    assert_eq!(rows.len(), 1);
    let detail = serde_json::to_string(rows[0].detail.as_ref().unwrap()).unwrap();
    assert!(
        !detail.contains(path),
        "audit must not contain the save path"
    );
    assert!(
        !detail.contains(script),
        "audit must not contain the save script"
    );
}

#[tokio::test]
async fn search_audit_omits_query() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let query = "secret search query xyz";
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_search")
                .with_arguments(arguments(serde_json::json!({"query": query}))),
        )
        .await
        .unwrap();
    assert_ok(&result);
    let calls = fake.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "search");
    assert_eq!(calls[0].1["query"], query);
    let rows = matching_audit(&core, "browser_lightpanda_search");
    assert_eq!(rows.len(), 1);
    let detail = serde_json::to_string(rows[0].detail.as_ref().unwrap()).unwrap();
    assert!(!detail.contains(query), "audit must not contain the query");
}

#[tokio::test]
async fn session_id_is_stripped_from_args_and_reaches_routing() {
    let (port, _dir, _core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_get_url")
                .with_arguments(arguments(serde_json::json!({"session_id": "alpha"}))),
        )
        .await
        .unwrap();
    assert_ok(&result);
    let calls = fake.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "getUrl");
    assert!(
        !calls[0].1.contains_key("session_id"),
        "session_id must be stripped from the native JSON arguments"
    );
    assert_eq!(
        calls[0].2.as_deref(),
        Some("alpha"),
        "session_id must reach the caller routing parameter"
    );
}

#[tokio::test]
async fn file_url_is_rejected_before_native_call() {
    let (port, _dir, _core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_goto")
                .with_arguments(arguments(serde_json::json!({"url": "file:///etc/passwd"}))),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "file:// must be rejected");
    assert!(
        first_text(&result).contains("http and https"),
        "rejection must cite the http(s)-only policy"
    );
    assert!(
        fake.calls().await.is_empty(),
        "the native caller must never be invoked for a rejected URL"
    );
}

#[tokio::test]
async fn get_cookies_url_is_not_treated_as_navigation() {
    let (port, _dir, _core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    // A non-http(s) url here only scopes cookie output; it must not trip the
    // navigation policy and must reach the native caller.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_get_cookies")
                .with_arguments(arguments(serde_json::json!({"url": "file:///cookies.txt"}))),
        )
        .await
        .unwrap();
    assert_ok(&result);
    let calls = fake.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "getCookies");
    assert_eq!(calls[0].1["url"], "file:///cookies.txt");
}

#[tokio::test]
async fn upstream_is_error_remains_is_error() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    fake.push(Ok(UpstreamToolResponse::Result(CallToolResult::error(
        vec![ContentBlock::text("native tool failed")],
    ))))
    .await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_click")
                .with_arguments(arguments(serde_json::json!({"selector": "#go"}))),
        )
        .await
        .unwrap();
    assert_eq!(
        result.is_error,
        Some(true),
        "upstream isError must be preserved"
    );
    let rows = matching_audit(&core, "browser_lightpanda_click");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
    assert_eq!(rows[0].error_code.as_deref(), Some("NATIVE_TOOL_ERROR"));
}

#[tokio::test]
async fn upstream_json_rpc_error_keeps_upstream_code() {
    let (port, _dir, core, fake) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    fake.push(Ok(UpstreamToolResponse::JsonRpcError {
        code: -32604,
        message: "FrameNotLoaded: no frame is loaded".into(),
        data: None,
    }))
    .await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_lightpanda_goto").with_arguments(arguments(
                serde_json::json!({"url": "https://example.com/"}),
            )),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let payload: Value = serde_json::from_str(&first_text(&result)).unwrap();
    assert_eq!(payload["upstream_code"], -32604);
    assert!(payload["message"]
        .as_str()
        .unwrap()
        .contains("FrameNotLoaded"));
    let rows = matching_audit(&core, "browser_lightpanda_goto");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
    assert_eq!(rows[0].error_code.as_deref(), Some("UPSTREAM_ERROR"));
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["upstream_code"], -32604);
    assert!(
        !serde_json::to_string(detail)
            .unwrap()
            .contains("no frame is loaded"),
        "audit must not contain the raw upstream message"
    );
}

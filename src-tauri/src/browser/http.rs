//! App-owned JSON-RPC HTTP client for the managed Lightpanda MCP server.
//!
//! Local Workspaces deliberately does not use a generic MCP client transport
//! here: it must select the native browsing session per request through the
//! `Mcp-Session-Id` header. Bootstrap, session-control calls, and ordinary
//! calls with omitted `session_id` explicitly route on `default` so
//! initialization never creates a hidden generated session.
//!
//! The endpoint must be a loopback `http://127.0.0.1:<port>/mcp` URL inside
//! the managed WSL2 distro. Failed requests are never replayed automatically.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use lc_core::error::{CoreError, ErrorCode};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

/// Native session that always exists on a Lightpanda HTTP MCP server.
pub const DEFAULT_SESSION_ID: &str = "default";

/// Maximum accepted `session_id` length in bytes.
const MAX_SESSION_ID_BYTES: usize = 256;

const SESSION_HEADER: &str = "mcp-session-id";

/// Outcome of a native `tools/call`.
///
/// A valid upstream JSON-RPC tool error is preserved verbatim; only
/// malformed transport/protocol data becomes `BrowserProtocolError`.
#[derive(Debug, Clone)]
pub enum UpstreamToolResponse {
    Result(CallToolResult),
    JsonRpcError {
        code: i64,
        message: String,
        data: Option<Value>,
    },
}

/// Protocol diagnostics from `initialize`.
///
/// `server_version` is the MCP `serverInfo.version` (for example `0.1.0`),
/// which is NOT the Lightpanda binary version and must never be used as the
/// binary pin authority.
#[derive(Debug, Clone)]
pub struct InitializeMetadata {
    pub protocol_version: String,
    pub server_name: String,
    pub server_version: String,
    pub instructions: Option<String>,
}

#[derive(Debug)]
pub struct LightpandaHttpClient {
    client: reqwest::Client,
    endpoint: url::Url,
    next_id: AtomicU64,
    request_timeout: Duration,
}

/// Validate a caller-supplied session id for safe use as an HTTP header value.
pub fn validate_session_id(session_id: &str) -> Result<(), CoreError> {
    if session_id.is_empty() {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser session_id must not be empty",
        ));
    }
    if session_id.len() > MAX_SESSION_ID_BYTES {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("browser session_id exceeds {MAX_SESSION_ID_BYTES} bytes"),
        ));
    }
    if session_id
        .bytes()
        .any(|byte| !(0x20..=0x7e).contains(&byte))
    {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser session_id contains control or non-ASCII characters",
        ));
    }
    Ok(())
}

fn effective_session_id(session_id: Option<&str>) -> Result<&str, CoreError> {
    match session_id {
        None => Ok(DEFAULT_SESSION_ID),
        Some(id) => {
            validate_session_id(id)?;
            Ok(id)
        }
    }
}

/// Parsed JSON-RPC response envelope for a request that expects one.
enum Envelope {
    Result(Value),
    RpcError {
        code: i64,
        message: String,
        data: Option<Value>,
    },
}

impl LightpandaHttpClient {
    /// Build a client for a loopback Lightpanda MCP endpoint.
    pub fn new(endpoint: url::Url, request_timeout: Duration) -> Result<Self, CoreError> {
        if endpoint.scheme() != "http" {
            return Err(CoreError::new(
                ErrorCode::InvalidArgument,
                "Lightpanda MCP endpoint must use http on the local loopback",
            ));
        }
        let loopback = match endpoint.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if !loopback {
            return Err(CoreError::new(
                ErrorCode::InvalidArgument,
                "Lightpanda MCP endpoint must bind the local loopback",
            ));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserProcessLost,
                    format!("failed to build Lightpanda HTTP client: {e}"),
                )
            })?;
        Ok(Self {
            client,
            endpoint,
            next_id: AtomicU64::new(1),
            request_timeout,
        })
    }

    pub fn endpoint(&self) -> &url::Url {
        &self.endpoint
    }

    /// MCP `initialize` on the `default` session.
    pub async fn initialize_default(&self) -> Result<InitializeMetadata, CoreError> {
        let params = serde_json::json!({
            "protocolVersion": rmcp::model::ProtocolVersion::default().to_string(),
            "capabilities": {},
            "clientInfo": {
                "name": "local-workspaces",
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        let envelope = self
            .request("initialize", Some(params), Some(DEFAULT_SESSION_ID))
            .await?;
        let value = match envelope {
            Envelope::Result(value) => value,
            Envelope::RpcError { code, message, .. } => {
                return Err(CoreError::new(
                    ErrorCode::BrowserProtocolError,
                    format!("Lightpanda initialize rejected: {code}: {message}"),
                ));
            }
        };
        let result: rmcp::model::InitializeResult = serde_json::from_value(value).map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserProtocolError,
                format!("invalid Lightpanda initialize result: {e}"),
            )
        })?;
        Ok(InitializeMetadata {
            protocol_version: result.protocol_version.to_string(),
            server_name: result.server_info.name,
            server_version: result.server_info.version,
            instructions: result.instructions,
        })
    }

    /// MCP `notifications/initialized` on the `default` session.
    pub async fn notify_initialized_default(&self) -> Result<(), CoreError> {
        self.notify("notifications/initialized", Some(DEFAULT_SESSION_ID))
            .await
    }

    /// MCP `tools/list` on the `default` session.
    pub async fn list_tools_default(&self) -> Result<Vec<rmcp::model::Tool>, CoreError> {
        let envelope = self
            .request(
                "tools/list",
                Some(serde_json::json!({})),
                Some(DEFAULT_SESSION_ID),
            )
            .await?;
        let value = match envelope {
            Envelope::Result(value) => value,
            Envelope::RpcError { code, message, .. } => {
                return Err(CoreError::new(
                    ErrorCode::BrowserProtocolError,
                    format!("Lightpanda tools/list rejected: {code}: {message}"),
                ));
            }
        };
        let result: rmcp::model::ListToolsResult = serde_json::from_value(value).map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserProtocolError,
                format!("invalid Lightpanda tools/list result: {e}"),
            )
        })?;
        Ok(result.tools)
    }

    /// Native `tools/call` routed to the selected session.
    ///
    /// `session_id = None` routes on `default`. A valid upstream JSON-RPC
    /// error is returned as `UpstreamToolResponse::JsonRpcError`, never as a
    /// transport error. Failed calls are never retried.
    pub async fn call_tool(
        &self,
        tool: &str,
        arguments: Map<String, Value>,
        session_id: Option<&str>,
    ) -> Result<UpstreamToolResponse, CoreError> {
        let params = serde_json::json!({
            "name": tool,
            "arguments": arguments,
        });
        match self.request("tools/call", Some(params), session_id).await? {
            Envelope::Result(value) => {
                let result: CallToolResult = serde_json::from_value(value).map_err(|e| {
                    CoreError::new(
                        ErrorCode::BrowserProtocolError,
                        format!("invalid Lightpanda CallToolResult: {e}"),
                    )
                })?;
                Ok(UpstreamToolResponse::Result(result))
            }
            Envelope::RpcError {
                code,
                message,
                data,
            } => Ok(UpstreamToolResponse::JsonRpcError {
                code,
                message,
                data,
            }),
        }
    }

    /// Send one JSON-RPC request and return the matched response envelope.
    async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        session_id: Option<&str>,
    ) -> Result<Envelope, CoreError> {
        let session = effective_session_id(session_id)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(params) = params {
            body["params"] = params;
        }
        match self.post(body, session).await? {
            PostOutcome::Body(bytes) => parse_envelope(&bytes, id),
            PostOutcome::PreservedError {
                code,
                message,
                data,
            } => Ok(Envelope::RpcError {
                code,
                message,
                data,
            }),
        }
    }

    /// Send one JSON-RPC notification (no id, no response body expected).
    async fn notify(&self, method: &str, session_id: Option<&str>) -> Result<(), CoreError> {
        let session = effective_session_id(session_id)?;
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        self.post(body, session).await?;
        Ok(())
    }

    async fn post(&self, body: Value, session: &str) -> Result<PostOutcome, CoreError> {
        let payload = serde_json::to_vec(&body).map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserProtocolError,
                format!("failed to encode Lightpanda request: {e}"),
            )
        })?;
        let response = self
            .client
            .post(self.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .header(SESSION_HEADER, session)
            .timeout(self.request_timeout)
            .body(payload)
            .send()
            .await
            .map_err(|e| transport_error(&e, &self.endpoint))?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| transport_error(&e, &self.endpoint))?
            .to_vec();

        if status.is_success() {
            return Ok(PostOutcome::Body(normalize_body(&content_type, &bytes)?));
        }

        // Non-2xx: preserve a valid JSON-RPC error envelope (for example a
        // native rejection of an unknown session) instead of hiding it.
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(error) = value.get("error") {
                if let Some((code, message, data)) = parse_rpc_error(error) {
                    return Ok(PostOutcome::PreservedError {
                        code,
                        message,
                        data,
                    });
                }
            }
        }
        Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            format!(
                "Lightpanda MCP endpoint returned HTTP {status} for {}",
                body.get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("request")
            ),
        ))
    }
}

/// Successful HTTP exchange outcome.
enum PostOutcome {
    /// Normalized raw JSON-RPC response bytes.
    Body(Vec<u8>),
    /// Valid JSON-RPC error returned with a non-success HTTP status.
    PreservedError {
        code: i64,
        message: String,
        data: Option<Value>,
    },
}

fn transport_error(error: &reqwest::Error, endpoint: &url::Url) -> CoreError {
    if error.is_timeout() {
        return CoreError::new(
            ErrorCode::BrowserActionTimeout,
            format!("Lightpanda MCP request to {endpoint} timed out"),
        );
    }
    CoreError::new(
        ErrorCode::BrowserProcessLost,
        format!("Lightpanda MCP endpoint {endpoint} is unreachable: {error}"),
    )
}

/// Decode a successful HTTP response into raw JSON-RPC response bytes.
///
/// `application/json` bodies pass through; `text/event-stream` bodies are
/// reduced to the first `data:` payload that parses as a JSON-RPC object.
fn normalize_body(content_type: &str, bytes: &[u8]) -> Result<Vec<u8>, CoreError> {
    if content_type.contains("text/event-stream") {
        let text = std::str::from_utf8(bytes).map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserProtocolError,
                "Lightpanda SSE response was not valid UTF-8",
            )
        })?;
        for event in text.split("\n\n") {
            let mut data = String::new();
            for line in event.lines() {
                if let Some(payload) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(payload.trim_start());
                }
            }
            if data.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                if value.get("jsonrpc").is_some() {
                    return serde_json::to_vec(&value).map_err(|e| {
                        CoreError::new(
                            ErrorCode::BrowserProtocolError,
                            format!("failed to re-encode Lightpanda SSE payload: {e}"),
                        )
                    });
                }
            }
        }
        return Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Lightpanda SSE response contained no JSON-RPC message",
        ));
    }
    if content_type.is_empty() || content_type.contains("application/json") {
        return Ok(bytes.to_vec());
    }
    Err(CoreError::new(
        ErrorCode::BrowserProtocolError,
        format!("unsupported Lightpanda MCP content type: {content_type}"),
    ))
}

fn parse_envelope(response: &[u8], expected_id: u64) -> Result<Envelope, CoreError> {
    let value: Value = serde_json::from_slice(response).map_err(|_| {
        CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Lightpanda MCP response was not valid JSON",
        )
    })?;
    let envelope = value.as_object().ok_or_else(|| {
        CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Lightpanda MCP response was not a JSON object",
        )
    })?;
    if envelope.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Lightpanda MCP response missing jsonrpc 2.0 marker",
        ));
    }
    let id_matches = envelope
        .get("id")
        .and_then(|id| id.as_u64().or_else(|| id.as_i64().map(|v| v as u64)))
        == Some(expected_id);
    if !id_matches {
        return Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            format!("Lightpanda MCP response id does not match request id {expected_id}"),
        ));
    }
    if let Some(error) = envelope.get("error") {
        return match parse_rpc_error(error) {
            Some((code, message, data)) => Ok(Envelope::RpcError {
                code,
                message,
                data,
            }),
            None => Err(CoreError::new(
                ErrorCode::BrowserProtocolError,
                "Lightpanda MCP error object was malformed",
            )),
        };
    }
    match envelope.get("result") {
        Some(result) => Ok(Envelope::Result(result.clone())),
        None => Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Lightpanda MCP response carried neither result nor error",
        )),
    }
}

fn parse_rpc_error(error: &Value) -> Option<(i64, String, Option<Value>)> {
    let object = error.as_object()?;
    let code = object.get("code")?.as_i64()?;
    let message = object.get("message")?.as_str()?.to_string();
    let data = object.get("data").filter(|d| !d.is_null()).cloned();
    Some((code, message, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use axum::Router;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        method: String,
        session_id: Option<String>,
        content_type: Option<String>,
        accept: Option<String>,
        body: Value,
    }

    #[derive(Debug, Clone, Default)]
    enum FakeMode {
        #[default]
        Normal,
        Sse,
        IdMismatch,
        MalformedJson,
        Slow(Duration),
        ToolError,
        RpcError,
    }

    #[derive(Clone)]
    struct FakeState {
        recorded: Arc<Mutex<Vec<RecordedRequest>>>,
        mode: FakeMode,
    }

    async fn mcp_handler(
        State(state): State<FakeState>,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> Response {
        let body_value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let method = body_value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        state.recorded.lock().await.push(RecordedRequest {
            method: method.clone(),
            session_id: headers
                .get(SESSION_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            content_type: headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            accept: headers
                .get("accept")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            body: body_value.clone(),
        });

        if let FakeMode::Slow(delay) = state.mode {
            tokio::time::sleep(delay).await;
        }

        if method == "notifications/initialized" {
            return axum::http::StatusCode::ACCEPTED.into_response();
        }

        let id = body_value.get("id").cloned().unwrap_or(Value::Null);
        let response_id = match state.mode {
            FakeMode::IdMismatch => serde_json::json!(9999),
            _ => id.clone(),
        };
        let result = match method.as_str() {
            "initialize" => serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "lightpanda", "version": "0.1.0"},
            }),
            "tools/list" => serde_json::json!({ "tools": [] }),
            _ => match state.mode {
                FakeMode::ToolError => serde_json::json!({
                    "content": [{"type": "text", "text": "boom"}],
                    "isError": true,
                }),
                _ => serde_json::json!({
                    "content": [{"type": "text", "text": "ok"}],
                    "isError": false,
                }),
            },
        };

        match state.mode {
            FakeMode::RpcError => axum::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32604,
                    "message": "FrameNotLoaded: frame is not loaded",
                },
            }))
            .into_response(),
            FakeMode::MalformedJson => (
                axum::http::StatusCode::OK,
                [("content-type", "application/json")],
                "{not-json",
            )
                .into_response(),
            FakeMode::Sse => {
                let payload = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                });
                let event = format!("event: message\ndata: {payload}\n\n");
                (
                    axum::http::StatusCode::OK,
                    [("content-type", "text/event-stream")],
                    event,
                )
                    .into_response()
            }
            _ => axum::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": response_id,
                "result": result,
            }))
            .into_response(),
        }
    }

    async fn start_fake(mode: FakeMode) -> (url::Url, FakeState) {
        let state = FakeState {
            recorded: Arc::new(Mutex::new(Vec::new())),
            mode,
        };
        let app = Router::new()
            .route("/mcp", post(mcp_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake MCP server");
        let url: url::Url = format!("http://{}/mcp", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, state)
    }

    fn client_for(url: &url::Url) -> LightpandaHttpClient {
        LightpandaHttpClient::new(url.clone(), Duration::from_secs(5)).unwrap()
    }

    #[tokio::test]
    async fn bootstrap_and_session_routing_use_explicit_headers() {
        let (url, state) = start_fake(FakeMode::Normal).await;
        let client = client_for(&url);

        let metadata = client.initialize_default().await.unwrap();
        assert_eq!(metadata.server_name, "lightpanda");
        assert_eq!(metadata.server_version, "0.1.0");
        client.notify_initialized_default().await.unwrap();
        client.list_tools_default().await.unwrap();

        let seen = state.recorded.lock().await.clone();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].method, "initialize");
        assert_eq!(seen[0].session_id.as_deref(), Some("default"));
        assert_eq!(seen[1].method, "notifications/initialized");
        assert_eq!(seen[1].session_id.as_deref(), Some("default"));
        assert_eq!(seen[2].method, "tools/list");
        assert_eq!(seen[2].session_id.as_deref(), Some("default"));
        for request in &seen {
            assert_eq!(request.content_type.as_deref(), Some("application/json"));
            assert_eq!(
                request.accept.as_deref(),
                Some("application/json, text/event-stream")
            );
        }
        // Notifications carry no JSON-RPC id; requests carry monotonic ids.
        assert!(seen[1].body.get("id").is_none());
        assert_eq!(seen[0].body["id"], 1);
        assert_eq!(seen[2].body["id"], 2);

        client.call_tool("getUrl", Map::new(), None).await.unwrap();
        let last_session = state
            .recorded
            .lock()
            .await
            .last()
            .unwrap()
            .session_id
            .clone();
        assert_eq!(last_session.as_deref(), Some("default"));

        client
            .call_tool("getUrl", Map::new(), Some("alpha"))
            .await
            .unwrap();
        let last = state.recorded.lock().await.pop().unwrap();
        assert_eq!(last.session_id.as_deref(), Some("alpha"));
        assert_eq!(last.body["params"]["name"], "getUrl");
        assert_eq!(last.body["params"]["arguments"], serde_json::json!({}));
    }

    #[tokio::test]
    async fn session_id_validation_rejects_unsafe_header_values() {
        for bad in ["", "a\rb", "a\nb", "a\0b", "a\x1fb"] {
            let err = validate_session_id(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument);
        }
        assert!(validate_session_id("team-a").is_ok());
        assert!(validate_session_id(&"x".repeat(256)).is_ok());
        assert!(validate_session_id(&"x".repeat(257)).is_err());

        let (url, _state) = start_fake(FakeMode::Normal).await;
        let client = client_for(&url);
        let err = client
            .call_tool("getUrl", Map::new(), Some("bad\nid"))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[tokio::test]
    async fn sse_response_is_unwrapped() {
        let (url, state) = start_fake(FakeMode::Sse).await;
        let client = client_for(&url);
        let response = client.call_tool("getUrl", Map::new(), None).await.unwrap();
        match response {
            UpstreamToolResponse::Result(result) => {
                assert_ne!(result.is_error, Some(true));
            }
            other => panic!("expected result, got {other:?}"),
        }
        assert_eq!(state.recorded.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn request_id_mismatch_is_protocol_error() {
        let (url, _state) = start_fake(FakeMode::IdMismatch).await;
        let client = client_for(&url);
        let err = client
            .call_tool("getUrl", Map::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserProtocolError);
        assert!(err.message.contains("id"), "{}", err.message);
    }

    #[tokio::test]
    async fn malformed_json_is_protocol_error() {
        let (url, _state) = start_fake(FakeMode::MalformedJson).await;
        let client = client_for(&url);
        let err = client
            .call_tool("getUrl", Map::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserProtocolError);
    }

    #[tokio::test]
    async fn timeout_maps_to_action_timeout_without_replay() {
        let (url, state) = start_fake(FakeMode::Slow(Duration::from_secs(2))).await;
        let client = LightpandaHttpClient::new(url, Duration::from_millis(200)).unwrap();
        let err = client
            .call_tool("getUrl", Map::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserActionTimeout);
        // The failed request must not be replayed automatically.
        assert_eq!(state.recorded.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn connection_loss_maps_to_process_lost() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url: url::Url = format!("http://{addr}/mcp").parse().unwrap();
        let client = client_for(&url);
        let err = client
            .call_tool("getUrl", Map::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserProcessLost);
    }

    #[tokio::test]
    async fn upstream_tool_error_is_preserved() {
        let (url, _state) = start_fake(FakeMode::ToolError).await;
        let client = client_for(&url);
        let response = client.call_tool("click", Map::new(), None).await.unwrap();
        match response {
            UpstreamToolResponse::Result(result) => {
                assert_eq!(result.is_error, Some(true));
            }
            other => panic!("expected tool error result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upstream_jsonrpc_error_is_preserved_verbatim() {
        let (url, _state) = start_fake(FakeMode::RpcError).await;
        let client = client_for(&url);
        let response = client.call_tool("click", Map::new(), None).await.unwrap();
        match response {
            UpstreamToolResponse::JsonRpcError { code, message, .. } => {
                assert_eq!(code, -32604);
                assert!(message.starts_with("FrameNotLoaded"), "{message}");
            }
            other => panic!("expected JSON-RPC error, got {other:?}"),
        }
    }

    #[test]
    fn endpoint_must_be_loopback_http() {
        let bad_hosts = [
            "http://0.0.0.0:38741/mcp",
            "http://192.168.1.10:38741/mcp",
            "https://127.0.0.1:38741/mcp",
            "http://example.com/mcp",
        ];
        for bad in bad_hosts {
            let err = LightpandaHttpClient::new(bad.parse().unwrap(), Duration::from_secs(5))
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument, "{bad}");
        }
        assert!(LightpandaHttpClient::new(
            "http://127.0.0.1:38741/mcp".parse().unwrap(),
            Duration::from_secs(5)
        )
        .is_ok());
    }
}

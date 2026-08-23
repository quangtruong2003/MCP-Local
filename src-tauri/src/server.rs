use std::sync::Arc;
use axum::Router;
use rmcp::{
    ServerHandler, tool, tool_handler, tool_router,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo},
    transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    },
};
use lc_core::CoreServices;

#[derive(Clone)]
pub struct EchoServer {
    _core: Arc<CoreServices>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl EchoServer {
    pub fn new(core: Arc<CoreServices>) -> Self {
        Self { _core: core, tool_router: Self::tool_router() }
    }

    #[tool(description = "Echoes its input back. Scaffold health check.")]
    fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> CallToolResult {
        CallToolResult::success(vec![ContentBlock::text(args.text)])
    }
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct EchoArgs {
    /// Text to echo back
    pub text: String,
}

#[tool_handler]
impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Local workspace file access. Tools are scoped to approved workspaces.")
    }
}

/// Bind axum to a free loopback port and serve /mcp. Returns the chosen port.
pub async fn serve(core: Arc<CoreServices>) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let service = StreamableHttpService::new(
        move || Ok(EchoServer::new(core.clone())),
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

mod config;
mod error;
mod services;
mod tools;

use std::sync::Arc;

use anyhow::Result;
use rmcp::{
    ServiceExt,
    ServerHandler,
    model::{
        CallToolRequestParam, CallToolResult, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities,
        ServerInfo,
    },
    service::RequestContext,
    transport::stdio,
    RoleServer,
};
use serde_json::Value;
use tracing::info;

use config::Config;
use services::tracking_time::TrackingTimeClient;
use tools::tracking_time::TrackingTimeTools;

// ─── Servidor MCP ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct TtimeServer {
    tracking_time_tools: Arc<TrackingTimeTools>,
    /// Token del usuario en modo HTTP (None en modo stdio)
    user_token: Option<String>,
}

impl TtimeServer {
    fn new(config: Config, user_token: Option<String>) -> Result<Self> {
        let tt_client = Arc::new(TrackingTimeClient::new(&config.tracking_time)?);
        let tracking_time_tools = Arc::new(TrackingTimeTools::new(tt_client, user_token.clone()));
        Ok(Self { tracking_time_tools, user_token })
    }

    /// Crea un servidor en modo HTTP para un usuario identificado por token.
    /// Si no hay credenciales guardadas, crea un servidor en modo "setup".
    fn for_token(token: String) -> Result<Self> {
        let config = Config::from_token(&token).unwrap_or_else(|_| Config::unconfigured());
        let tt_client = Arc::new(TrackingTimeClient::new(&config.tracking_time)?);
        let tracking_time_tools = Arc::new(TrackingTimeTools::new(tt_client, Some(token.clone())));
        Ok(Self { tracking_time_tools, user_token: Some(token) })
    }
}

impl ServerHandler for TtimeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ttime-bot", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Servidor MCP para gestión inteligente de tiempo con TrackingTime. \
                 Permite crear tareas, iniciar/detener timers y consultar entradas de tiempo. \
                 Prefijo de tools: 'tt_' para operaciones de TrackingTime.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::Error> {
        Ok(ListToolsResult::with_all_items(TrackingTimeTools::tool_definitions()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::Error> {
        let name = request.name.as_ref();
        let args: Value = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

        if name.starts_with("tt_") {
            self.tracking_time_tools.call(name, args).await
        } else {
            Err(rmcp::Error::invalid_params(
                format!("Tool desconocida: {}", name),
                None,
            ))
        }
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ttime_bot=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    let _ = dotenvy::dotenv();

    // Detectar modo por env var o argumento
    let port = std::env::var("PORT").ok().and_then(|p| p.parse::<u16>().ok());

    if let Some(port) = port {
        run_http(port).await
    } else {
        run_stdio().await
    }
}

async fn run_stdio() -> Result<()> {
    info!("Iniciando ttime-bot en modo stdio v{}", env!("CARGO_PKG_VERSION"));
    let config = Config::from_env()?;
    let server = TtimeServer::new(config, None)?;
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}

async fn run_http(port: u16) -> Result<()> {
    use axum::{Router, extract::Query};
    use rmcp::transport::streamable_http_server::{StreamableHttpService, session::local::LocalSessionManager};
    use std::collections::HashMap;
    use tower::Service;

    info!("Iniciando ttime-bot en modo HTTP v{} en puerto {}", env!("CARGO_PKG_VERSION"), port);

    let app = Router::new()
        .route("/mcp", axum::routing::any(|
            Query(params): Query<HashMap<String, String>>,
            req: axum::extract::Request,
        | async move {
            let token = params.get("token").cloned().unwrap_or_default();
            let mut service = StreamableHttpService::new(
                move || {
                    let token = token.clone();
                    TtimeServer::for_token(token)
                        .map_err(|e| std::io::Error::other(e.to_string()))
                },
                Arc::new(LocalSessionManager::default()),
                Default::default(),
            );
            service.call(req).await
        }));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!("Escuchando en http://0.0.0.0:{}/mcp?token=TU_TOKEN", port);
    axum::serve(listener, app).await?;
    Ok(())
}

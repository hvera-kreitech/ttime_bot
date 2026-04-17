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
}

impl TtimeServer {
    fn new(config: Config) -> Result<Self> {
        let tt_client = Arc::new(TrackingTimeClient::new(&config.tracking_time)?);
        let tracking_time_tools = Arc::new(TrackingTimeTools::new(tt_client));
        Ok(Self { tracking_time_tools })
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
        let tools = TrackingTimeTools::tool_definitions();

        // Aquí se agregan tools de otros servicios en el futuro:
        // tools.extend(OtroServicioTools::tool_definitions());

        Ok(ListToolsResult::with_all_items(tools))
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

        // Ruteamos por prefijo para escalar fácilmente a múltiples servicios
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
    // Logging hacia stderr (stdout es del protocolo MCP)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ttime_bot=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    // Cargamos variables de entorno desde .env si existe
    let _ = dotenvy::dotenv();

    info!("Iniciando ttime-bot MCP server v{}", env!("CARGO_PKG_VERSION"));

    let config = Config::from_env()?;
    let server = TtimeServer::new(config)?;

    // Transport stdio: Claude Desktop / Claude Code se comunica por stdin/stdout
    server.serve(stdio()).await?.waiting().await?;

    Ok(())
}

//! Server setup and initialization

use crate::api::create_router;
use crate::db::init_db;
use crate::state::{AppState, GatewayConfig};
use crate::tools::{
    BashTool, EditTool, GlobTool, GrepTool, ReadTool, ToolRegistry, WriteTool,
};
use chrono::{Datelike, Timelike};
use river_core::AgentBirth;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

/// Server configuration from CLI args
pub struct ServerConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub model_url: Option<String>,
    pub model_name: Option<String>,
}

/// Initialize and run the gateway server
pub async fn run(config: ServerConfig) -> anyhow::Result<()> {
    // Initialize database
    let db_path = config.data_dir.join("river.db");
    let db = init_db(&db_path)?;

    // Create tool registry with core tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(&config.workspace)));
    registry.register(Box::new(WriteTool::new(&config.workspace)));
    registry.register(Box::new(EditTool::new(&config.workspace)));
    registry.register(Box::new(GlobTool::new(&config.workspace)));
    registry.register(Box::new(GrepTool::new(&config.workspace)));
    registry.register(Box::new(BashTool::new(&config.workspace)));

    tracing::info!("Registered {} tools", registry.len());

    // Create agent birth (current time)
    let now = chrono::Utc::now();
    let agent_birth = AgentBirth::new(
        now.year() as u16,
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )?;

    // Create gateway config
    let gateway_config = GatewayConfig {
        workspace: config.workspace,
        data_dir: config.data_dir,
        port: config.port,
        model_url: config.model_url.unwrap_or_else(|| "http://localhost:8080".to_string()),
        model_name: config.model_name.unwrap_or_else(|| "default".to_string()),
        context_limit: 65536,
        heartbeat_minutes: 45,
        agent_birth,
    };

    // Create app state
    let state = Arc::new(AppState::new(gateway_config, db, registry));

    // Create router
    let app = create_router(state);

    // Bind and serve
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    tracing::info!("Gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

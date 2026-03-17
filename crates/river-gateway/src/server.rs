//! Server setup and initialization

use crate::api::create_router;
use crate::db::init_db;
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::r#loop::{AgentLoop, LoopConfig, MessageQueue, ModelClient};
use crate::redis::{RedisClient, RedisConfig};
use crate::state::{AppState, GatewayConfig};
use crate::tools::{
    BashTool, EditTool, EmbedTool, GlobTool, GrepTool, MemoryDeleteTool, MemoryDeleteBySourceTool,
    MemorySearchTool, ReadTool, ToolRegistry, WriteTool,
};
use chrono::{Datelike, Timelike};
use river_core::AgentBirth;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Server configuration from CLI args
pub struct ServerConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub agent_name: String,
    pub model_url: Option<String>,
    pub model_name: Option<String>,
    pub embedding_url: Option<String>,
    pub redis_url: Option<String>,
    pub orchestrator_url: Option<String>,
}

/// Initialize and run the gateway server
pub async fn run(config: ServerConfig) -> anyhow::Result<()> {
    // Initialize database
    let db_path = config.data_dir.join("river.db");
    let db = init_db(&db_path)?;

    // Create embedding client if configured
    let embedding_client = if let Some(url) = &config.embedding_url {
        let embed_config = EmbeddingConfig {
            url: url.clone(),
            ..Default::default()
        };
        Some(EmbeddingClient::new(embed_config))
    } else {
        None
    };

    // Create Redis client if configured
    let redis_client = if let Some(url) = &config.redis_url {
        let redis_config = RedisConfig {
            url: url.clone(),
            agent_name: config.agent_name.clone(),
        };
        Some(RedisClient::new(redis_config).await?)
    } else {
        None
    };

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
    let agent_name = config.agent_name.clone();
    let gateway_config = GatewayConfig {
        workspace: config.workspace.clone(),
        data_dir: config.data_dir.clone(),
        port: config.port,
        model_url: config.model_url.unwrap_or_else(|| "http://localhost:8080".to_string()),
        model_name: config.model_name.unwrap_or_else(|| "default".to_string()),
        context_limit: 65536,
        heartbeat_minutes: 45,
        agent_birth,
        agent_name: agent_name.clone(),
        embedding: config.embedding_url.as_ref().map(|url| EmbeddingConfig {
            url: url.clone(),
            ..Default::default()
        }),
        redis: config.redis_url.as_ref().map(|url| RedisConfig {
            url: url.clone(),
            agent_name: agent_name.clone(),
        }),
    };

    // Wrap database in Arc for sharing
    let db_arc = Arc::new(std::sync::Mutex::new(db));
    let snowflake_gen = Arc::new(river_core::SnowflakeGenerator::new(gateway_config.agent_birth));

    // Create tool registry with core tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(&config.workspace)));
    registry.register(Box::new(WriteTool::new(&config.workspace)));
    registry.register(Box::new(EditTool::new(&config.workspace)));
    registry.register(Box::new(GlobTool::new(&config.workspace)));
    registry.register(Box::new(GrepTool::new(&config.workspace)));
    registry.register(Box::new(BashTool::new(&config.workspace)));

    // Register memory tools if embedding client is available
    if let Some(ref embed_client) = embedding_client {
        let embed_arc = Arc::new(embed_client.clone());
        registry.register(Box::new(EmbedTool::new(
            db_arc.clone(),
            embed_arc.clone(),
            snowflake_gen.clone(),
        )));
        registry.register(Box::new(MemorySearchTool::new(db_arc.clone(), embed_arc.clone())));
        registry.register(Box::new(MemoryDeleteTool::new(db_arc.clone())));
        registry.register(Box::new(MemoryDeleteBySourceTool::new(db_arc.clone())));
        tracing::info!("Registered memory tools (embed, memory_search, memory_delete, memory_delete_by_source)");
    }

    // Register Redis tools if client is available
    if let Some(ref redis) = redis_client {
        let redis_arc = Arc::new(redis.clone());
        use crate::redis::*;
        registry.register(Box::new(WorkingMemorySetTool::new(redis_arc.clone())));
        registry.register(Box::new(WorkingMemoryGetTool::new(redis_arc.clone())));
        registry.register(Box::new(WorkingMemoryDeleteTool::new(redis_arc.clone())));
        registry.register(Box::new(MediumTermSetTool::new(redis_arc.clone())));
        registry.register(Box::new(MediumTermGetTool::new(redis_arc.clone())));
        registry.register(Box::new(ResourceLockTool::new(redis_arc.clone())));
        registry.register(Box::new(CounterIncrementTool::new(redis_arc.clone())));
        registry.register(Box::new(CounterGetTool::new(redis_arc.clone())));
        registry.register(Box::new(CacheSetTool::new(redis_arc.clone())));
        registry.register(Box::new(CacheGetTool::new(redis_arc.clone())));
        tracing::info!("Registered Redis tools (10 tools)");
    }

    tracing::info!("Registered {} tools total", registry.names().len());

    // Create loop components
    let (loop_tx, loop_rx) = mpsc::channel(256);
    let message_queue = Arc::new(MessageQueue::new());

    // Create model client
    let model_client = ModelClient::new(
        gateway_config.model_url.clone(),
        gateway_config.model_name.clone(),
        Duration::from_secs(120),
    )?;

    // Create loop config
    let loop_config = LoopConfig {
        workspace: gateway_config.workspace.clone(),
        default_heartbeat_minutes: gateway_config.heartbeat_minutes,
        context_limit: gateway_config.context_limit,
        model_timeout: Duration::from_secs(120),
        max_tool_calls_per_generation: 50,
    };

    // Create app state
    let state = Arc::new(AppState::new(
        gateway_config,
        db_arc.clone(),
        registry,
        embedding_client,
        redis_client,
        loop_tx,
        message_queue.clone(),
    ));

    // Spawn the agent loop
    let mut agent_loop = AgentLoop::new(
        loop_rx,
        message_queue,
        model_client,
        state.tool_executor.clone(),
        db_arc,
        loop_config,
    );
    tokio::spawn(async move {
        agent_loop.run().await;
        // Log if the loop exits (shouldn't happen in normal operation)
        tracing::error!("Agent loop exited unexpectedly");
    });

    // Create router
    let app = create_router(state);

    // Start heartbeat task if orchestrator configured
    if let Some(orchestrator_url) = &config.orchestrator_url {
        let gateway_url = format!("http://127.0.0.1:{}", config.port);
        let heartbeat_client = crate::heartbeat::HeartbeatClient::new(
            orchestrator_url.clone(),
            config.agent_name.clone(),
            gateway_url,
        );

        tokio::spawn(async move {
            heartbeat_client.run_loop(30).await;
        });

        tracing::info!("Started heartbeat to orchestrator: {}", orchestrator_url);
    }

    // Bind and serve
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    tracing::info!("Gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

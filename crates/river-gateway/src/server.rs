//! Server setup and initialization

use crate::agent::{AgentTask, AgentTaskConfig};
use crate::api::create_router;
use crate::coordinator::Coordinator;
use crate::db::init_db;
use crate::embeddings::{SyncService, VectorStore};
use crate::flash::FlashQueue;
use crate::spectator::{SpectatorTask, SpectatorConfig};
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::metrics::AgentMetrics;
use crate::policy::HealthPolicy;
use crate::r#loop::{MessageQueue, ModelClient};
use crate::watchdog::{spawn_watchdog_task, notify_ready};
use crate::redis::{RedisClient, RedisConfig};
use crate::state::{AppState, GatewayConfig};
use crate::subagent::SubagentManager;
use crate::tools::{
    BashTool, EditTool, EmbedTool, GlobTool, GrepTool, MemoryDeleteTool, MemoryDeleteBySourceTool,
    MemorySearchTool, ReadTool, ToolRegistry, WriteTool,
    SpawnSubagentTool, ListSubagentsTool, SubagentStatusTool, StopSubagentTool,
    InternalSendTool, InternalReceiveTool, WaitForSubagentTool,
    // Web tools
    WebFetchTool, WebSearchTool,
    // Communication tools
    SendMessageTool, ListAdaptersTool, ReadChannelTool, AdapterRegistry, SyncConversationTool,
    // Model management tools
    RequestModelTool, ReleaseModelTool, SwitchModelTool, ModelManagerConfig, ModelManagerState,
    // Scheduling tools
    ScheduleHeartbeatTool, RotateContextTool, HeartbeatScheduler, ContextRotation,
    // Logging tools
    LogReadTool,
};
use chrono::Utc;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};

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
    pub auth_token_file: Option<PathBuf>,
    pub context_limit: u32,
    /// Communication adapters: (name, outbound_url, read_url)
    pub adapters: Vec<(String, String, Option<String>)>,
    /// Spectator model URL (defaults to same as agent)
    pub spectator_model_url: Option<String>,
    /// Spectator model name (defaults to same as agent)
    pub spectator_model_name: Option<String>,
}

/// Initialize and run the gateway server
pub async fn run(config: ServerConfig) -> anyhow::Result<()> {
    // Migrate inbox/ to conversations/ if needed
    let inbox_path = config.workspace.join("inbox");
    let conversations_path = config.workspace.join("conversations");
    if inbox_path.exists() && !conversations_path.exists() {
        std::fs::rename(&inbox_path, &conversations_path)?;
        tracing::info!("Migrated inbox/ to conversations/");
    }

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

    // Create vector store and run initial sync if embeddings are configured
    let embeddings_dir = config.workspace.join("embeddings");
    let _vector_store = if config.embedding_url.is_some() {
        let vectors_db_path = config.data_dir.join("vectors.db");
        match VectorStore::open(&vectors_db_path) {
            Ok(store) => {
                tracing::info!("Opened vector store at {:?}", vectors_db_path);

                // Run initial sync
                let sync_service = SyncService::new(embeddings_dir.clone(), store.clone());
                match sync_service.full_sync().await {
                    Ok(stats) => {
                        tracing::info!(
                            updated = stats.updated,
                            skipped = stats.skipped,
                            errors = stats.errors,
                            "Initial embedding sync complete"
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Initial embedding sync failed: {}", e);
                    }
                }

                Some(Arc::new(store))
            }
            Err(e) => {
                tracing::warn!("Failed to open vector store: {}", e);
                None
            }
        }
    } else {
        tracing::info!("Embeddings not configured - vector store disabled");
        None
    };

    // Load agent birth from database (must have been created via `river-gateway birth`)
    let birth_memory = db.get_birth_memory()?.ok_or_else(|| {
        anyhow::anyhow!(
            "Agent not birthed. Run `river-gateway birth --data-dir {:?} --name <name>` first.",
            config.data_dir
        )
    })?;
    let agent_birth = birth_memory.id.birth();
    tracing::info!("Agent birth: {} (from memory: \"{}\")", agent_birth, birth_memory.content);

    // Create gateway config
    let agent_name = config.agent_name.clone();
    let gateway_config = GatewayConfig {
        workspace: config.workspace.clone(),
        data_dir: config.data_dir.clone(),
        port: config.port,
        model_url: config.model_url.unwrap_or_else(|| "http://localhost:8080".to_string()),
        model_name: config.model_name.unwrap_or_else(|| "default".to_string()),
        context_limit: config.context_limit as u64,
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

    // Create subagent manager
    let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen.clone())));

    // Create tool registry with core tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(&config.workspace)));
    registry.register(Box::new(WriteTool::new(&config.workspace)));
    registry.register(Box::new(EditTool::new(&config.workspace)));
    registry.register(Box::new(GlobTool::new(&config.workspace)));
    registry.register(Box::new(GrepTool::new(&config.workspace)));
    registry.register(Box::new(BashTool::new(&config.workspace)));

    // Register subagent tools
    registry.register(Box::new(SpawnSubagentTool::new(
        subagent_manager.clone(),
        config.workspace.clone(),
        gateway_config.model_url.clone(),
        gateway_config.model_name.clone(),
    )));
    registry.register(Box::new(ListSubagentsTool::new(subagent_manager.clone())));
    registry.register(Box::new(SubagentStatusTool::new(subagent_manager.clone())));
    registry.register(Box::new(StopSubagentTool::new(subagent_manager.clone())));
    registry.register(Box::new(InternalSendTool::new(subagent_manager.clone())));
    registry.register(Box::new(InternalReceiveTool::new(subagent_manager.clone())));
    registry.register(Box::new(WaitForSubagentTool::new(subagent_manager.clone())));
    tracing::info!("Registered subagent tools (7 tools)");

    // Register web tools
    registry.register(Box::new(WebFetchTool::new(&config.workspace)));
    registry.register(Box::new(WebSearchTool::new()));
    tracing::info!("Registered web tools (webfetch, websearch)");

    // Register logging tools
    registry.register(Box::new(LogReadTool::new(Some(config.agent_name.clone()))));
    tracing::info!("Registered logging tools (log_read)");

    // Create and register scheduling tools
    let heartbeat_scheduler = Arc::new(HeartbeatScheduler::new(gateway_config.heartbeat_minutes));
    let context_rotation = Arc::new(ContextRotation::new());
    registry.register(Box::new(ScheduleHeartbeatTool::new(heartbeat_scheduler.clone())));
    registry.register(Box::new(RotateContextTool::new(context_rotation.clone())));
    tracing::info!("Registered scheduling tools (schedule_heartbeat, rotate_context)");

    // Create and register communication tools with configured adapters
    let adapter_registry = Arc::new(RwLock::new(AdapterRegistry::new()));

    // Register adapters from config
    {
        use crate::tools::AdapterConfig;
        let mut reg = adapter_registry.write().await;
        for (name, outbound_url, read_url) in &config.adapters {
            tracing::info!(
                adapter_name = %name,
                outbound_url = %outbound_url,
                read_url = ?read_url,
                "Registering communication adapter"
            );
            reg.register(AdapterConfig {
                name: name.clone(),
                outbound_url: outbound_url.clone(),
                read_url: read_url.clone(),
                features: std::collections::HashSet::new(),
            });
        }
        if config.adapters.is_empty() {
            tracing::warn!("No communication adapters configured - send_message will not work");
        } else {
            tracing::info!("Registered {} communication adapter(s)", config.adapters.len());
        }
    }

    // Create conversation writer channel
    let (conv_writer_tx, conv_writer_rx) = mpsc::channel::<crate::conversations::WriteOp>(256);

    // Spawn ConversationWriter task
    use crate::conversations::writer::ConversationWriter;
    let mut conversation_writer = ConversationWriter::new(conv_writer_rx);
    tokio::spawn(async move {
        conversation_writer.run().await;
    });
    tracing::info!("Spawned ConversationWriter");

    registry.register(Box::new(SendMessageTool::new(
        adapter_registry.clone(),
        config.workspace.clone(),
        config.agent_name.clone(),
        agent_birth.to_string(),
        conv_writer_tx.clone(),
    )));
    registry.register(Box::new(ListAdaptersTool::new(adapter_registry.clone())));
    registry.register(Box::new(ReadChannelTool::new(adapter_registry.clone())));
    registry.register(Box::new(SyncConversationTool::new(
        adapter_registry.clone(),
        config.workspace.clone(),
        conv_writer_tx.clone(),
    )));
    tracing::info!("Registered communication tools (send_message, list_adapters, read_channel, sync_conversation)");

    // Create and register model management tools (only if orchestrator is configured)
    if let Some(ref orchestrator_url) = config.orchestrator_url {
        let model_config = ModelManagerConfig {
            orchestrator_url: orchestrator_url.clone(),
            timeout: Duration::from_secs(120),
        };
        let model_state = Arc::new(RwLock::new(ModelManagerState::default()));
        registry.register(Box::new(RequestModelTool::new(model_config.clone(), model_state.clone())));
        registry.register(Box::new(ReleaseModelTool::new(model_config, model_state.clone())));
        registry.register(Box::new(SwitchModelTool::new(model_state)));
        tracing::info!("Registered model management tools (request_model, release_model, switch_model)");
    }

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

    // Create loop components (loop_tx used by API, message_queue shared)
    let (loop_tx, _loop_rx) = mpsc::channel(256);
    let message_queue = Arc::new(MessageQueue::new());

    // Load auth token if configured
    let auth_token = if let Some(ref token_file) = config.auth_token_file {
        let token = tokio::fs::read_to_string(token_file)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read auth token file: {}", e))?
            .trim()
            .to_string();
        tracing::info!("Loaded auth token from {:?}", token_file);
        Some(token)
    } else {
        tracing::warn!("No auth token configured - API endpoints are unprotected");
        None
    };

    // Create metrics
    let metrics = Arc::new(RwLock::new(AgentMetrics::new(
        gateway_config.agent_name.clone(),
        Utc::now(),
        gateway_config.context_limit,
    )));

    // Create health policy
    let policy = Arc::new(RwLock::new(HealthPolicy::new(
        gateway_config.agent_name.clone(),
        gateway_config.data_dir.clone(),
    )));

    // Create app state
    let state = Arc::new(AppState::new(
        gateway_config,
        db_arc.clone(),
        registry,
        embedding_client,
        redis_client,
        loop_tx,
        message_queue.clone(),
        auth_token,
        subagent_manager,
        metrics,
        policy,
    ));

    // Extract config values needed for agent before state takes ownership
    let agent_workspace = state.config.workspace.clone();
    let agent_model_url = state.config.model_url.clone();
    let agent_model_name = state.config.model_name.clone();
    let agent_context_limit = state.config.context_limit;
    let agent_heartbeat_minutes = state.config.heartbeat_minutes;

    // Coordinator-based agent (default)
    let mut coordinator = Coordinator::new();
    let flash_queue = Arc::new(FlashQueue::new(20));

    // Determine spectator model (defaults to same as agent)
    let spectator_model_url = config.spectator_model_url
        .clone()
        .unwrap_or_else(|| agent_model_url.clone());
    let spectator_model_name = config.spectator_model_name
        .clone()
        .unwrap_or_else(|| agent_model_name.clone());

    // Create model client for agent task
    let agent_model_client = ModelClient::new(
        agent_model_url.clone(),
        agent_model_name.clone(),
        Duration::from_secs(120),
    )?;

    let agent_config = AgentTaskConfig {
        workspace: agent_workspace.clone(),
        embeddings_dir: config.workspace.join("embeddings"),
        context_limit: agent_context_limit,
        max_tool_calls: 50,
        history_limit: 50,
        heartbeat_interval: Duration::from_secs(agent_heartbeat_minutes as u64 * 60),
        ..Default::default()
    };

    let agent_task = AgentTask::new(
        agent_config,
        coordinator.bus().clone(),
        message_queue,
        agent_model_client,
        state.tool_executor.clone(),
        flash_queue.clone(),
        db_arc.clone(),
        snowflake_gen.clone(),
    );

    coordinator.spawn_task("agent", |_| agent_task.run());

    // Create and spawn spectator task
    let spectator_model = ModelClient::new(
        spectator_model_url.clone(),
        spectator_model_name.clone(),
        Duration::from_secs(60),
    )?;

    let spectator_config = SpectatorConfig {
        spectator_dir: config.workspace.join("spectator"),
        moments_dir: config.workspace.join("embeddings").join("moments"),
        model_timeout: Duration::from_secs(60),
    };

    let spectator_task = SpectatorTask::new(
        spectator_config,
        coordinator.bus().clone(),
        spectator_model,
        db_arc.clone(),
        snowflake_gen.clone(),
    );

    coordinator.spawn_task("spectator", |_| spectator_task.run());

    tracing::info!("Spawned agent and spectator tasks via coordinator");

    tokio::spawn(async move {
        // Keep coordinator alive until shutdown
        // In a full implementation, this would listen for shutdown signals
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
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

    // Spawn systemd watchdog task (30s interval, 60s timeout)
    let _watchdog_handle = spawn_watchdog_task(30);

    // Notify systemd that service is ready
    notify_ready();

    axum::serve(listener, app).await?;

    Ok(())
}

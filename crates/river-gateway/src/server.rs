//! Server setup and initialization

use crate::agent::{AgentTask, AgentTaskConfig};
use crate::api::create_router;
use crate::coordinator::Coordinator;
use crate::embeddings::{SyncService, VectorStore};
use crate::flash::FlashQueue;
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::metrics::AgentMetrics;
use crate::model::ModelClient;
use crate::policy::HealthPolicy;
use crate::queue::MessageQueue;
use crate::redis::{RedisClient, RedisConfig};
use crate::spectator::{SpectatorConfig, SpectatorTask};
use crate::state::{AppState, GatewayConfig};
use crate::subagent::SubagentManager;
use crate::tools::{
    AdapterRegistry, BashTool, ContextRotation, EditTool, GlobTool, GrepTool, HeartbeatScheduler,
    ReadTool, SendMessageTool, ToolRegistry, WriteTool,
};
use crate::watchdog::{notify_ready, spawn_watchdog_task};
use chrono::Utc;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Server configuration from CLI args
pub struct ServerConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub agent_name: String,
    pub model_url: Option<String>,
    pub model_name: Option<String>,
    pub embedding_url: Option<String>,
    pub embedding_model_name: Option<String>,
    pub redis_url: Option<String>,
    pub orchestrator_url: Option<String>,
    pub auth_token_file: Option<PathBuf>,
    pub context_limit: u32,
    /// Compaction threshold (fraction of context limit)
    pub compaction_threshold: f64,
    /// Post-compaction fill target (fraction of context limit)
    pub fill_target: f64,
    /// Minimum messages kept in context
    pub min_messages: usize,
    /// Communication adapters: (name, outbound_url, read_url)
    pub adapters: Vec<(String, String, Option<String>)>,
    /// Spectator model URL (defaults to same as agent)
    pub spectator_model_url: Option<String>,
    /// Spectator model name (defaults to same as agent)
    pub spectator_model_name: Option<String>,
    /// Env var name for agent model API key
    pub model_api_key_env: Option<String>,
    /// Env var name for spectator model API key
    pub spectator_api_key_env: Option<String>,
}

/// Initialize and run the gateway server
pub async fn run(config: ServerConfig) -> anyhow::Result<()> {

    // Create embedding client if configured
    let embedding_client = if let Some(url) = &config.embedding_url {
        let model_name = config.embedding_model_name.clone().unwrap_or_else(|| "nomic-embed-text".to_string());
        let embed_config = EmbeddingConfig {
            url: url.clone(),
            model: model_name,
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
    // Open vector store if embeddings are configured
    let vector_store = if config.embedding_url.is_some() {
        let vectors_db_path = config.data_dir.join("vectors.db");
        match VectorStore::open(&vectors_db_path) {
            Ok(store) => {
                tracing::info!("Opened vector store at {:?}", vectors_db_path);
                Some(store)
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

    // Load agent birth from birth.json
    let birth_path = config.data_dir.join("birth.json");
    let birth_record: crate::BirthRecord = if birth_path.exists() {
        let content = std::fs::read_to_string(&birth_path)
            .map_err(|e| anyhow::anyhow!("Failed to read birth file: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse birth file: {}", e))?
    } else {
        anyhow::bail!(
            "Agent not birthed. Run `river-gateway birth --data-dir {:?} --name <name>` first.",
            config.data_dir
        );
    };
    let agent_birth = birth_record.id.birth();
    tracing::info!(
        "Agent birth: {} (name: \"{}\")",
        agent_birth,
        birth_record.name
    );

    // Create gateway config
    let agent_name = config.agent_name.clone();
    let gateway_config = GatewayConfig {
        workspace: config.workspace.clone(),
        data_dir: config.data_dir.clone(),
        port: config.port,
        model_url: config
            .model_url
            .unwrap_or_else(|| "http://localhost:8080".to_string()),
        model_name: config.model_name.unwrap_or_else(|| "default".to_string()),
        context_limit: config.context_limit as u64,
        heartbeat_minutes: 45,
        agent_birth,
        agent_name: agent_name.clone(),
        embedding: config.embedding_url.as_ref().map(|url| {
            let model = config.embedding_model_name.clone().unwrap_or_else(|| "nomic-embed-text".to_string());
            EmbeddingConfig {
                url: url.clone(),
                model,
                ..Default::default()
            }
        }),
        redis: config.redis_url.as_ref().map(|url| RedisConfig {
            url: url.clone(),
            agent_name: agent_name.clone(),
        }),
    };

    let snowflake_gen = Arc::new(river_core::SnowflakeGenerator::new(
        gateway_config.agent_birth,
    ));

    // Create subagent manager
    let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen.clone())));

    // Load auth token: env var first, then --auth-token-file fallback
    let auth_token = match river_core::require_auth_token() {
        Ok(token) => {
            tracing::info!("Auth token loaded from RIVER_AUTH_TOKEN");
            token
        }
        Err(_) => {
            if let Some(ref token_file) = config.auth_token_file {
                let token = tokio::fs::read_to_string(token_file)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read auth token file: {}", e))?
                    .trim()
                    .to_string();
                if token.is_empty() {
                    return Err(anyhow::anyhow!("Auth token file is empty"));
                }
                tracing::info!("Auth token loaded from {:?}", token_file);
                token
            } else {
                return Err(anyhow::anyhow!(
                    "No auth token configured. Set RIVER_AUTH_TOKEN in .env or pass --auth-token-file"
                ));
            }
        }
    };

    // Build authed HTTP client for outbound calls
    let authed_http_client = river_core::build_authed_client(&auth_token);

    // Create tool registry with core tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(&config.workspace)));
    registry.register(Box::new(WriteTool::new(&config.workspace)));
    registry.register(Box::new(EditTool::new(&config.workspace)));
    registry.register(Box::new(GlobTool::new(&config.workspace)));
    registry.register(Box::new(GrepTool::new(&config.workspace)));
    registry.register(Box::new(BashTool::new(&config.workspace)));

    // NOTE: Subagent, web, logging, and scheduling tools are disabled for now.
    // With 27+ tools, small local models (hermes3:8b, gemma4:e2b) get confused
    // and call the wrong tool — e.g. internal_send instead of speak. Stripping
    // down to core tools until we have either smarter models or tool filtering.
    //
    // Disabled tools:
    //   subagent: spawn_subagent, list_subagents, subagent_status, stop_subagent,
    //             internal_send, internal_receive, wait_for_subagent
    //   web: webfetch, websearch
    //   logging: log_read
    //   scheduling: schedule_heartbeat, rotate_context

    // Keep heartbeat scheduler alive even though the tool isn't registered —
    // the built-in heartbeat timer still needs it
    let heartbeat_scheduler = Arc::new(HeartbeatScheduler::new(gateway_config.heartbeat_minutes));
    let context_rotation = Arc::new(ContextRotation::new());

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
            tracing::info!(
                "Registered {} communication adapter(s)",
                config.adapters.len()
            );
        }
    }

    // NOTE: speak requires a shared channel_context (not yet wired up).
    // Using send_message instead — it takes explicit adapter+channel args.
    // list_adapters, read_channel, sync_conversation disabled to reduce tool count.
    registry.register(Box::new(SendMessageTool::new(
        adapter_registry.clone(),
        config.workspace.clone(),
        snowflake_gen.clone(),
        authed_http_client.clone(),
    )));
    tracing::info!("Registered communication tools (send_message)");

    // Register search tool if embeddings are configured
    if let (Some(ref store), Some(ref embed_client)) = (&vector_store, &embedding_client) {
        registry.register(Box::new(crate::tools::SearchTool::new(
            store.clone(),
            Arc::new(embed_client.clone()),
        )));
        tracing::info!("Registered search tool");
    }

    // Register write_atomic tool
    registry.register(Box::new(crate::tools::WriteAtomicTool::new(
        config.workspace.clone(),
        snowflake_gen.clone(),
        agent_name.clone(),
    )));
    tracing::info!("Registered write_atomic tool");

    // NOTE: Model management and Redis tools disabled to reduce tool count.
    // Re-enable when using larger models that can handle 27+ tools reliably.

    tracing::info!("Registered {} tools total", registry.names().len());

    let message_queue = Arc::new(MessageQueue::new());

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

    // Create home channel writer
    let home_channel_path = config
        .workspace
        .join("channels/home")
        .join(format!("{}.jsonl", agent_name));
    let home_channel_writer =
        crate::channels::writer::HomeChannelWriter::spawn(home_channel_path.clone());

    // Create home channel directories
    let home_dir = config.workspace.join("channels/home").join(&agent_name);
    tokio::fs::create_dir_all(home_dir.join("moves")).await.ok();
    tokio::fs::create_dir_all(home_dir.join("tool-results"))
        .await
        .ok();

    // Clone embedding client for sync service (before AppState consumes it)
    let embedding_client_for_sync = embedding_client.clone();

    // Create app state
    let mut app_state = AppState::new(
        gateway_config,
        registry,
        embedding_client,
        redis_client,
        message_queue.clone(),
        auth_token,
        authed_http_client,
        subagent_manager,
        metrics,
        policy,
    );
    // Share the same adapter registry between tools and HTTP registration
    app_state.adapter_registry = adapter_registry.clone();
    app_state.home_channel_writer = Some(home_channel_writer.clone());
    let state = Arc::new(app_state);

    // Extract config values needed for agent before state takes ownership
    let agent_workspace = state.config.workspace.clone();
    let agent_model_url = state.config.model_url.clone();
    let agent_model_name = state.config.model_name.clone();
    let agent_heartbeat_minutes = state.config.heartbeat_minutes;

    // Coordinator-based agent (default)
    let mut coordinator = Coordinator::new();
    let flash_queue = Arc::new(FlashQueue::new(20));

    // Determine spectator model (defaults to same as agent)
    let spectator_model_url = config
        .spectator_model_url
        .clone()
        .unwrap_or_else(|| agent_model_url.clone());
    let spectator_model_name = config
        .spectator_model_name
        .clone()
        .unwrap_or_else(|| agent_model_name.clone());

    // Create model client for agent task
    let agent_model_client = ModelClient::new(
        agent_model_url.clone(),
        agent_model_name.clone(),
        Duration::from_secs(120),
        config.model_api_key_env.as_deref(),
    )?;

    let agent_config = AgentTaskConfig {
        workspace: agent_workspace.clone(),
        max_tool_calls: 50,
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
        snowflake_gen.clone(),
        home_channel_writer, // moved, not cloned — app_state already has its clone
        home_channel_path,
        agent_name.clone(),
    )?;

    coordinator.spawn_task("agent", |_| agent_task.run());

    // Create and spawn spectator task
    let spectator_model = ModelClient::new(
        spectator_model_url.clone(),
        spectator_model_name.clone(),
        Duration::from_secs(300),
        config.spectator_api_key_env.as_deref(),
    )?;

    let spectator_config = SpectatorConfig {
        spectator_dir: config.workspace.join("spectator"),
        home_channel_path: config
            .workspace
            .join("channels/home")
            .join(format!("{}.jsonl", agent_name)),
        moves_path: config
            .workspace
            .join("channels/home")
            .join(&agent_name)
            .join("moves.jsonl"),
        sweep_interval: Duration::from_secs(300),
        sweep_token_budget: 16384,
        moves_tail: 10,
    };

    let spectator_home_writer = state
        .home_channel_writer
        .as_ref()
        .expect("Home channel writer must be configured")
        .clone();

    let spectator_task = SpectatorTask::new(
        spectator_config,
        coordinator.bus().clone(),
        spectator_model,
        spectator_home_writer,
        snowflake_gen.clone(),
    );

    coordinator.spawn_task("spectator", |_| spectator_task.run());

    // Spawn embedding sync service
    if let (Some(store), Some(embed_client)) = (vector_store, embedding_client_for_sync) {
        let sync_service = SyncService::new(
            embeddings_dir.clone(),
            store,
            embed_client,
        );
        let sync_rx = coordinator.bus().subscribe();
        coordinator.spawn_task("sync", move |_| async move {
            sync_service.run(sync_rx).await;
        });
        tracing::info!("Spawned embedding sync service");
    }

    tracing::info!("Spawned agent, spectator, and sync tasks via coordinator");

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

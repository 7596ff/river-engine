use clap::Parser;
use river_orchestrator::{
    api::create_router,
    config::OrchestratorConfig,
    discovery::ModelScanner,
    external::ExternalModelsFile,
    process::ProcessConfig,
    resources::ResourceConfig,
    OrchestratorState,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "river-orchestrator")]
#[command(about = "River Engine Orchestrator - Coordination Service")]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "5000")]
    port: u16,

    /// Health threshold in seconds
    #[arg(long, default_value = "120")]
    health_threshold: u64,

    /// Directories to scan for GGUF models (comma-separated)
    #[arg(long, value_delimiter = ',')]
    model_dirs: Vec<PathBuf>,

    /// Path to external models config JSON file
    #[arg(long)]
    external_models: Option<PathBuf>,

    /// Idle timeout in seconds before unloading models
    #[arg(long, default_value = "900")]
    idle_timeout: u64,

    /// Path to llama-server binary
    #[arg(long, default_value = "llama-server")]
    llama_server_path: PathBuf,

    /// Port range for llama-server instances (start-end)
    #[arg(long, default_value = "8080-8180")]
    port_range: String,

    /// Reserved VRAM in MB
    #[arg(long, default_value = "500")]
    reserve_vram_mb: u64,

    /// Reserved RAM in MB
    #[arg(long, default_value = "2000")]
    reserve_ram_mb: u64,

    /// Path to JSON config file (starts full system: orchestrator + gateways + adapters)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Path to environment file (loaded before config, existing env wins)
    #[arg(long)]
    env_file: Option<PathBuf>,
}

fn parse_port_range(s: &str) -> (u16, u16) {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 2 {
        let start = parts[0].parse().unwrap_or(8080);
        let end = parts[1].parse().unwrap_or(8180);
        (start, end)
    } else {
        (8080, 8180)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    // Config-driven mode: load JSON, spawn gateways + adapters
    if let Some(config_path) = args.config {
        return run_from_config(config_path, args.env_file).await;
    }

    let auth_token = river_core::require_auth_token()
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    tracing::info!("Starting River Orchestrator");
    tracing::info!("Port: {}", args.port);
    tracing::info!("Health threshold: {}s", args.health_threshold);

    let (port_start, port_end) = parse_port_range(&args.port_range);

    // Scan for local models
    let scanner = ModelScanner::new(args.model_dirs.clone());
    let local_models = scanner.scan();
    tracing::info!("Discovered {} local models", local_models.len());

    // Load external models
    let external_models = if let Some(path) = &args.external_models {
        tracing::info!("Loading external models from {:?}", path);
        let content = std::fs::read_to_string(path)?;
        let file: ExternalModelsFile = serde_json::from_str(&content)?;
        file.external_models
    } else {
        Vec::new()
    };
    tracing::info!("Loaded {} external models", external_models.len());

    let config = OrchestratorConfig {
        port: args.port,
        health_threshold_seconds: args.health_threshold,
        model_dirs: args.model_dirs,
        external_models_config: args.external_models,
        idle_timeout_seconds: args.idle_timeout,
        llama_server_path: args.llama_server_path.clone(),
        port_range_start: port_start,
        port_range_end: port_end,
        reserve_vram_mb: args.reserve_vram_mb,
        reserve_ram_mb: args.reserve_ram_mb,
    };

    let resource_config = ResourceConfig {
        reserve_vram_bytes: args.reserve_vram_mb * 1024 * 1024,
        reserve_ram_bytes: args.reserve_ram_mb * 1024 * 1024,
    };

    let process_config = ProcessConfig {
        llama_server_path: args.llama_server_path,
        port_range_start: port_start,
        port_range_end: port_end,
        default_ctx_size: 8192,
        health_check_timeout: Duration::from_secs(30),
    };

    let state = Arc::new(OrchestratorState::new(
        config,
        local_models,
        external_models,
        resource_config,
        process_config,
        auth_token,
    ));

    // Spawn background loops
    // Idle eviction loop
    let state_clone = state.clone();
    let idle_timeout = Duration::from_secs(args.idle_timeout);
    tokio::spawn(async move {
        idle_eviction_loop(state_clone, idle_timeout).await;
    });

    // Health check loop
    let process_manager = state.process_manager.clone();
    tokio::spawn(async move {
        river_orchestrator::process::health_check_loop(
            process_manager,
            Duration::from_secs(10),
        ).await;
    });

    let app = create_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    tracing::info!("Orchestrator listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Config-driven startup: read JSON config, spawn all gateways + adapters
async fn run_from_config(config_path: PathBuf, env_file: Option<PathBuf>) -> anyhow::Result<()> {
    // 1. Load env file
    if let Some(ref env_path) = env_file {
        river_orchestrator::env::load_env_file(env_path)?;
        tracing::info!("Loaded env file: {:?}", env_path);
    }

    // 2. Read and parse config
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| anyhow::anyhow!("Failed to read config {:?}: {}", config_path, e))?;
    let expanded = river_orchestrator::env::expand_vars(&raw)?;
    let config: river_orchestrator::config_file::RiverConfig = serde_json::from_str(&expanded)
        .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;

    // 3. Validate
    let errors = river_orchestrator::validate::validate(&config);
    if !errors.is_empty() {
        for e in &errors {
            tracing::error!("Config error: {}", e);
        }
        anyhow::bail!("{} config validation error(s)", errors.len());
    }

    tracing::info!(
        port = config.port,
        agents = config.agents.len(),
        models = config.models.len(),
        "Config loaded and validated"
    );

    // 4. Build OrchestratorState from config
    let (port_start, port_end) = parse_port_range(&config.resources.port_range);

    let orch_config = OrchestratorConfig {
        port: config.port,
        health_threshold_seconds: 120,
        model_dirs: vec![],
        external_models_config: None,
        idle_timeout_seconds: 900,
        llama_server_path: config.resources.llama_server_path.clone(),
        port_range_start: port_start,
        port_range_end: port_end,
        reserve_vram_mb: config.resources.reserve_vram_mb,
        reserve_ram_mb: config.resources.reserve_ram_mb,
    };

    let resource_config = ResourceConfig {
        reserve_vram_bytes: config.resources.reserve_vram_mb * 1024 * 1024,
        reserve_ram_bytes: config.resources.reserve_ram_mb * 1024 * 1024,
    };

    let process_config = ProcessConfig {
        llama_server_path: config.resources.llama_server_path.clone(),
        port_range_start: port_start,
        port_range_end: port_end,
        default_ctx_size: 8192,
        health_check_timeout: Duration::from_secs(30),
    };

    // Build local and external model lists from config
    let mut local_models = Vec::new();
    let mut external_models = Vec::new();

    for (model_id, model) in &config.models {
        if model.is_gguf() {
            if let Some(ref path) = model.path {
                match river_orchestrator::discovery::parse_gguf(path) {
                    Ok(metadata) => {
                        local_models.push(river_orchestrator::discovery::LocalModel {
                            id: model_id.clone(),
                            path: path.clone(),
                            metadata,
                        });
                    }
                    Err(e) => {
                        tracing::error!(model = %model_id, error = %e, "Failed to parse GGUF file");
                    }
                }
            }
        } else if !model.is_embedding() {
            if let Some(ref endpoint) = model.endpoint {
                external_models.push(river_orchestrator::external::ExternalModel {
                    id: model_id.clone(),
                    provider: model.provider.clone(),
                    litellm_model: model.name.clone().unwrap_or_default(),
                    api_base: endpoint.clone(),
                });
            }
        }
    }

    let config_auth_token = river_core::require_auth_token()
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let state = Arc::new(OrchestratorState::new(
        orch_config, local_models, external_models, resource_config, process_config, config_auth_token,
    ));

    // Start orchestrator HTTP server
    let app = create_router(state.clone());
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Orchestrator listening on {}", addr);
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    // Spawn background loops
    let state_for_idle = state.clone();
    tokio::spawn(async move {
        idle_eviction_loop(state_for_idle, Duration::from_secs(900)).await;
    });
    let pm = state.process_manager.clone();
    tokio::spawn(async move {
        river_orchestrator::process::health_check_loop(pm, Duration::from_secs(10)).await;
    });

    // 5-6. Resolve models — for GGUF, wait for health before spawning gateways
    let mut resolved_models = std::collections::HashMap::new();
    for (model_id, model) in &config.models {
        if model.is_gguf() {
            tracing::info!(model = %model_id, "Loading GGUF model (waiting for health)...");
            match state.request_model(model_id, 120).await {
                Ok(river_orchestrator::state::ModelRequestResponse::Ready { endpoint, .. }) => {
                    resolved_models.insert(model_id.clone(), river_orchestrator::cli_builder::ResolvedModel {
                        endpoint,
                        name: model.name.clone(),
                    });
                    tracing::info!(model = %model_id, "GGUF model ready");
                }
                Ok(river_orchestrator::state::ModelRequestResponse::Loading { .. }) => {
                    tracing::error!(model = %model_id, "GGUF model still loading after 120s, skipping");
                }
                Err(e) => {
                    tracing::error!(model = %model_id, error = %e, "Failed to load GGUF model");
                }
            }
        } else if let Some(ref endpoint) = model.endpoint {
            resolved_models.insert(model_id.clone(), river_orchestrator::cli_builder::ResolvedModel {
                endpoint: endpoint.clone(),
                name: model.name.clone(),
            });
        }
    }

    // 7. Spawn supervised children
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut handles = Vec::new();
    let mut started_agents = 0u32;

    // Resolve sibling binaries relative to the orchestrator's own binary
    let bin_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    for (name, agent) in &config.agents {
        // Check birth
        let db_path = agent.data_dir.join("river.db");
        if !db_path.exists() {
            tracing::error!(
                agent = %name,
                "Agent not birthed. Run: river-gateway birth --data-dir {} --name {}",
                agent.data_dir.display(), name
            );
            continue;
        }

        // Spawn gateway
        let gateway_args = river_orchestrator::cli_builder::gateway_args(
            name, agent, &config.models, config.port, &resolved_models,
        );
        let gateway_spec = river_orchestrator::supervisor::ChildSpec {
            label: format!("{}/gateway", name),
            bin: bin_dir.join("river-gateway"),
            args: gateway_args,
        };
        let rx = shutdown_tx.subscribe();
        handles.push(tokio::spawn(river_orchestrator::supervisor::supervise(gateway_spec, rx)));

        // Spawn adapters
        for adapter in &agent.adapters {
            let adapter_args = match adapter.adapter_type.as_str() {
                "discord" => river_orchestrator::cli_builder::discord_args(adapter, agent.port),
                other => {
                    tracing::warn!(adapter_type = %other, "Unknown adapter type, skipping");
                    continue;
                }
            };
            let adapter_bin = if adapter.bin_path().is_absolute() {
                adapter.bin_path()
            } else {
                bin_dir.join(adapter.bin_path())
            };
            let adapter_spec = river_orchestrator::supervisor::ChildSpec {
                label: format!("{}/{}", name, adapter.adapter_type),
                bin: adapter_bin,
                args: adapter_args,
            };
            let rx = shutdown_tx.subscribe();
            handles.push(tokio::spawn(river_orchestrator::supervisor::supervise(adapter_spec, rx)));
        }

        started_agents += 1;
    }

    if started_agents == 0 {
        anyhow::bail!("No agents could start");
    }

    tracing::info!(agents = started_agents, children = handles.len(), "All children spawned");

    // 8. Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutdown signal received");
    let _ = shutdown_tx.send(());

    // Wait for all supervisors to stop (with timeout)
    let _ = tokio::time::timeout(
        Duration::from_secs(15),
        futures::future::join_all(handles),
    ).await;

    tracing::info!("Shutdown complete");
    Ok(())
}

async fn idle_eviction_loop(state: Arc<OrchestratorState>, timeout: Duration) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        let idle_models = state.process_manager.idle_models(timeout).await;
        for model_id in idle_models {
            tracing::info!("Evicting idle model: {}", model_id);
            if let Err(e) = state.unload_model(&model_id).await {
                tracing::warn!("Failed to unload {}: {}", model_id, e);
            }
        }
    }
}

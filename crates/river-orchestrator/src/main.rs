use clap::Parser;
use river_orchestrator::{
    api::create_router,
    config::{ModelsFile, OrchestratorConfig},
    discovery::ModelScanner,
    external::ExternalModelsFile,
    models::{ModelInfo, ModelProvider},
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

    /// Path to models config JSON file (legacy)
    #[arg(long)]
    models_config: Option<PathBuf>,

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
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    tracing::info!("Starting River Orchestrator");
    tracing::info!("Port: {}", args.port);
    tracing::info!("Health threshold: {}s", args.health_threshold);

    let (port_start, port_end) = parse_port_range(&args.port_range);

    // Check for advanced mode (model_dirs specified)
    let use_advanced = !args.model_dirs.is_empty();

    let state = if use_advanced {
        tracing::info!("Advanced mode: scanning for GGUF models");

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
            models_config: args.models_config,
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

        Arc::new(OrchestratorState::new_advanced(
            config,
            local_models,
            external_models,
            resource_config,
            process_config,
        ))
    } else {
        // Legacy mode
        let models = if let Some(path) = &args.models_config {
            tracing::info!("Loading models from {:?}", path);
            let content = std::fs::read_to_string(path)?;
            let file: ModelsFile = serde_json::from_str(&content)?;
            file.models
                .into_iter()
                .map(|m| ModelInfo::new(m.name, ModelProvider::from(m.provider.as_str())))
                .collect()
        } else {
            tracing::info!("No models config provided, starting with empty registry");
            vec![]
        };

        tracing::info!("Loaded {} models (legacy mode)", models.len());

        let config = OrchestratorConfig {
            port: args.port,
            health_threshold_seconds: args.health_threshold,
            models_config: args.models_config,
            ..Default::default()
        };

        Arc::new(OrchestratorState::new(config, models))
    };

    // Spawn background loops if in advanced mode
    if use_advanced {
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
    }

    let app = create_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    tracing::info!("Orchestrator listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

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

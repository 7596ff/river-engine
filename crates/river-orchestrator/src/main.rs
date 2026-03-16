use clap::Parser;
use river_orchestrator::{
    api::create_router,
    config::{ModelsFile, OrchestratorConfig},
    models::{ModelInfo, ModelProvider},
    OrchestratorState,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

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

    /// Path to models config JSON file
    #[arg(long)]
    models_config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    tracing::info!("Starting River Orchestrator");
    tracing::info!("Port: {}", args.port);
    tracing::info!("Health threshold: {}s", args.health_threshold);

    // Load models from config file if provided
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

    tracing::info!("Loaded {} models", models.len());

    let config = OrchestratorConfig {
        port: args.port,
        health_threshold_seconds: args.health_threshold,
        models_config: args.models_config,
    };

    let state = Arc::new(OrchestratorState::new(config, models));
    let app = create_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    tracing::info!("Orchestrator listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

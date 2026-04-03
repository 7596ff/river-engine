//! River Orchestrator - Process supervisor for River Engine.
//!
//! Spawns workers, adapters, and embed services, maintains registry,
//! handles model/role switching, and manages respawn policy.

mod config;
mod http;
mod model;
mod registry;
mod respawn;
mod supervisor;

use clap::Parser;
use config::load_config;
use http::{router, AppState};
use registry::new_shared_registry;
use respawn::new_shared_respawn_manager;
use supervisor::{new_shared_supervisor, run_health_checks, spawn_dyad};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "river-orchestrator")]
#[command(about = "Process supervisor for River Engine")]
struct Args {
    /// Config file path
    #[arg(short, long, default_value = "river.json")]
    config: PathBuf,

    /// Override config port
    #[arg(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("river_orchestrator=info".parse()?))
        .init();

    let args = Args::parse();

    // Load config
    tracing::info!("Loading config from {:?}", args.config);
    let mut config = load_config(&args.config)?;

    // Override port if specified
    if let Some(port) = args.port {
        config.port = port;
    }

    let port = config.port;
    let config = Arc::new(config);

    // Create shared state
    let registry = new_shared_registry();
    let supervisor = new_shared_supervisor();
    let respawn = new_shared_respawn_manager();
    let client = reqwest::Client::new();
    let dyad_locks = Arc::new(RwLock::new(HashMap::new()));

    let state = AppState {
        config: config.clone(),
        registry: registry.clone(),
        supervisor: supervisor.clone(),
        respawn: respawn.clone(),
        client: client.clone(),
        dyad_locks,
    };

    // Bind HTTP server
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Orchestrator listening on http://{}", addr);

    let orchestrator_url = format!("http://localhost:{}", port);

    // Spawn the HTTP server
    let app = router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await
    });

    // Give server a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Spawn embed service if configured
    if config.embed.is_some() {
        tracing::info!("Spawning embed service");
        let mut sup = supervisor.write().await;
        if let Err(e) = sup.spawn_embed(&orchestrator_url, "embed").await {
            tracing::warn!("Failed to spawn embed service: {}. Continuing without embedding.", e);
        }
        drop(sup);

        // Wait for registration
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // Spawn dyads
    for (dyad_name, dyad_config) in &config.dyads {
        tracing::info!("Spawning dyad: {}", dyad_name);
        if let Err(e) = spawn_dyad(&supervisor, &orchestrator_url, dyad_name, dyad_config).await {
            tracing::error!("Failed to spawn dyad {}: {}", dyad_name, e);
        }

        // Wait for registrations
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    tracing::info!("Startup complete. Entering supervision loop.");

    // Health check interval
    let health_interval = Duration::from_secs(60);
    let mut health_ticker = tokio::time::interval(health_interval);

    // Main supervision loop with shutdown handling
    loop {
        tokio::select! {
            _ = health_ticker.tick() => {
                let dead = run_health_checks(&client, &supervisor).await;
                if !dead.is_empty() {
                    tracing::warn!("Dead processes detected: {:?}", dead);
                    // TODO: Trigger respawns for dead processes
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received shutdown signal");
                break;
            }
        }
    }

    // Graceful shutdown
    tracing::info!("Initiating graceful shutdown");
    {
        let mut sup = supervisor.write().await;
        sup.shutdown(Duration::from_secs(300)).await;
    }

    // Stop the HTTP server
    server.abort();

    tracing::info!("Shutdown complete");
    Ok(())
}

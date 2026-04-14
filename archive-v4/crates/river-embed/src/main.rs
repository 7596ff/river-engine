//! River Embed server binary.

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::Mutex;

mod chunk;
mod config;
mod embed;
mod http;
mod index;
mod search;
mod store;

use config::{RegistrationRequest, RegistrationResponse, EmbedServiceInfo};
use embed::EmbedClient;
use http::AppState;
use river_snowflake::{AgentBirth, GeneratorCache};
use search::CursorManager;
use store::Store;

#[derive(Parser)]
#[command(name = "river-embed")]
#[command(about = "Embedding and vector search service")]
struct Args {
    /// Orchestrator endpoint for registration.
    #[arg(long)]
    orchestrator: String,

    /// Service name.
    #[arg(long, default_value = "embed")]
    name: String,

    /// Port to bind (0 for OS-assigned).
    #[arg(long, default_value = "0")]
    port: u16,

    /// Path to database file.
    #[arg(long, default_value = "embed.db")]
    db: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Bind HTTP server first to get port
    let addr = format!("127.0.0.1:{}", args.port);
    let listener = TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    let endpoint = format!("http://{}", local_addr);

    eprintln!("Binding to {}", endpoint);

    // Register with orchestrator
    let client = reqwest::Client::new();
    let reg_request = RegistrationRequest {
        endpoint: endpoint.clone(),
        embed: EmbedServiceInfo { name: args.name.clone() },
    };

    eprintln!("Registering with orchestrator at {}", args.orchestrator);

    let reg_response: RegistrationResponse = client
        .post(format!("{}/register", args.orchestrator))
        .json(&reg_request)
        .send()
        .await?
        .json()
        .await?;

    if !reg_response.accepted {
        return Err("Registration rejected by orchestrator".into());
    }

    let model_config = reg_response
        .model
        .ok_or("Orchestrator did not provide model config")?;

    eprintln!(
        "Registered. Using model {} with {} dimensions",
        model_config.name, model_config.dimensions
    );

    // Initialize store
    let store = Store::open(&args.db, model_config.dimensions)?;

    // Create embed client
    let embed_client = EmbedClient::new(model_config);

    // Create birth for this service
    let birth = AgentBirth::now();

    // Build state
    let state = Arc::new(AppState {
        store: Mutex::new(store),
        embed_client,
        cursor_manager: CursorManager::default(),
        id_cache: GeneratorCache::new(),
        birth,
    });

    // Spawn cursor cleanup task
    {
        let cursor_manager = state.cursor_manager.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                cursor_manager.cleanup_expired();
            }
        });
    }

    // Build router
    let app = http::router(state);

    eprintln!("Embed server listening on {}", local_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    eprintln!("Shutdown signal received");
}

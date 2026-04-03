//! River Mock Adapter - TUI-based adapter for debugging.

mod adapter;
mod http;
mod tui;

use adapter::AdapterState;
use clap::Parser;
use http::router;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};

#[derive(Parser, Debug)]
#[command(name = "river-mock-adapter")]
#[command(about = "TUI-based mock adapter for River Engine debugging")]
struct Args {
    /// Orchestrator endpoint
    #[arg(long)]
    orchestrator: String,

    /// Dyad name
    #[arg(long)]
    dyad: String,

    /// Adapter type (must match config)
    #[arg(long, default_value = "mock")]
    adapter_type: String,

    /// Default channel name
    #[arg(long, default_value = "general")]
    channel: String,

    /// Port to bind (0 for OS-assigned)
    #[arg(long, default_value = "0")]
    port: u16,
}

/// Registration request to orchestrator.
#[derive(Debug, serde::Serialize)]
struct RegistrationRequest {
    endpoint: String,
    adapter: AdapterRegistration,
}

#[derive(Debug, serde::Serialize)]
struct AdapterRegistration {
    #[serde(rename = "type")]
    adapter_type: String,
    dyad: String,
    features: Vec<u16>,
}

/// Registration response from orchestrator.
#[derive(Debug, serde::Deserialize)]
struct RegistrationResponse {
    accepted: bool,
    config: serde_json::Value, // Mock adapter doesn't need config
    worker_endpoint: String,
}

pub type SharedState = Arc<RwLock<AdapterState>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    // Bind HTTP server
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;

    let adapter_endpoint = format!("http://localhost:{}", local_addr.port());

    // Create shared state
    let state: SharedState = Arc::new(RwLock::new(AdapterState::new(
        args.dyad.clone(),
        args.adapter_type.clone(),
        args.channel.clone(),
    )));

    // Channel for UI events
    let (ui_tx, ui_rx) = mpsc::channel::<tui::UiEvent>(256);

    // Get features
    let features = adapter::supported_features();

    // Register with orchestrator
    let client = reqwest::Client::new();
    let reg_request = RegistrationRequest {
        endpoint: adapter_endpoint.clone(),
        adapter: AdapterRegistration {
            adapter_type: args.adapter_type.clone(),
            dyad: args.dyad.clone(),
            features: features.iter().map(|f| *f as u16).collect(),
        },
    };

    let response = client
        .post(format!("{}/register", args.orchestrator))
        .json(&reg_request)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;

    if !response.status().is_success() {
        let msg = response.text().await?;
        eprintln!("Registration failed: {}", msg);
        std::process::exit(1);
    }

    let registration: RegistrationResponse = response.json().await?;
    if !registration.accepted {
        eprintln!("Registration rejected by orchestrator");
        std::process::exit(1);
    }

    // Store worker endpoint
    {
        let mut s = state.write().await;
        s.worker_endpoint = Some(registration.worker_endpoint.clone());
        s.add_system_message("Connected to orchestrator");
        s.add_system_message(&format!("Worker at {}", registration.worker_endpoint));
    }

    // Start HTTP server
    let http_state = state.clone();
    let http_tx = ui_tx.clone();
    tokio::spawn(async move {
        let app = router(http_state, http_tx);
        axum::serve(listener, app).await.ok();
    });

    // Run TUI on main thread
    tui::run(state, ui_rx, registration.worker_endpoint).await?;

    Ok(())
}

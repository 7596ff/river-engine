//! River Discord - Discord adapter for River Engine.
//!
//! Connects to Discord gateway, forwards events to Worker,
//! and executes outbound requests.

mod discord;
mod http;

use clap::Parser;
use discord::DiscordClient;
use http::router;
use river_protocol::{AdapterRegistration, AdapterRegistrationRequest, AdapterRegistrationResponse};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "river-discord")]
#[command(about = "Discord adapter for River Engine")]
struct Args {
    /// Orchestrator endpoint
    #[arg(long)]
    orchestrator: String,

    /// Dyad name
    #[arg(long)]
    dyad: String,

    /// Adapter type (must match config)
    #[arg(long, rename_all = "kebab-case", name = "type")]
    adapter_type: String,

    /// Port to bind (0 for OS-assigned)
    #[arg(long, default_value = "0")]
    port: u16,
}

/// Discord-specific config from orchestrator.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub guild_id: Option<u64>,
    pub intents: Option<u64>,
}

/// Shared adapter state (without the DiscordClient to maintain Sync).
pub struct AdapterState {
    pub config: Option<DiscordConfig>,
}

impl AdapterState {
    fn new() -> Self {
        Self { config: None }
    }
}

pub type SharedState = Arc<RwLock<AdapterState>>;

/// Combined state for HTTP handlers - includes discord client
pub struct HttpState {
    pub state: SharedState,
    pub discord: Arc<DiscordClient>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("river_discord=info".parse()?),
        )
        .init();

    let args = Args::parse();

    // Bind HTTP server
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!("Discord adapter listening on http://{}", local_addr);

    let adapter_endpoint = format!("http://localhost:{}", local_addr.port());

    // Create shared state
    let state: SharedState = Arc::new(RwLock::new(AdapterState::new()));

    // Get features this adapter supports
    let features = discord::supported_features();

    // Register with orchestrator
    tracing::info!("Registering with orchestrator at {}", args.orchestrator);
    let client = reqwest::Client::new();

    let reg_request = AdapterRegistrationRequest {
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
        tracing::error!("Registration failed: {}", msg);
        std::process::exit(1);
    }

    let registration: AdapterRegistrationResponse = response.json().await?;
    let discord_config: DiscordConfig = serde_json::from_value(registration.config)?;
    if !registration.accepted {
        tracing::error!("Registration rejected by orchestrator");
        std::process::exit(1);
    }

    tracing::info!(
        "Registered successfully, worker at {}",
        registration.worker_endpoint
    );

    // Store config
    {
        let mut s = state.write().await;
        s.config = Some(discord_config.clone());
    }

    // Initialize Discord client
    let discord = Arc::new(DiscordClient::new(discord_config.clone(), args.adapter_type.clone()).await?);

    // Create HTTP state
    let http_state = Arc::new(HttpState {
        state: state.clone(),
        discord: discord.clone(),
    });

    // Start event forwarding loop
    let discord_clone = discord.clone();
    let worker_endpoint = registration.worker_endpoint.clone();
    let event_task = tokio::spawn(async move {
        let http_client = reqwest::Client::new();
        loop {
            let events = discord_clone.poll_events().await;

            for event in events {
                if let Err(e) = http_client
                    .post(format!("{}/notify", worker_endpoint))
                    .json(&event)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                {
                    tracing::warn!("Failed to forward event to worker: {}", e);
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    // Start HTTP server
    let server = tokio::spawn(async move {
        let app = router(http_state);
        axum::serve(listener, app).await
    });

    tracing::info!("Discord adapter running");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down Discord adapter");

    event_task.abort();
    server.abort();

    Ok(())
}

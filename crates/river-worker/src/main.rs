//! River Worker - Agent runtime for River Engine.
//!
//! Runs the think→act loop: calling the LLM, executing tools,
//! handling notifications, and managing context.

mod config;
mod http;
mod llm;
mod persistence;
mod state;
mod tools;
mod worker_loop;

use clap::Parser;
use config::WorkerConfig;
use river_protocol::{WorkerRegistration, WorkerRegistrationRequest};
use http::router;
use river_adapter::Side;
use state::new_shared_state;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use worker_loop::run_loop;

#[derive(Parser, Debug)]
#[command(name = "river-worker")]
#[command(about = "Worker runtime for River Engine")]
struct Args {
    /// Orchestrator endpoint
    #[arg(long)]
    orchestrator: String,

    /// Dyad name
    #[arg(long)]
    dyad: String,

    /// Worker side
    #[arg(long)]
    side: String,

    /// Port to bind (0 for OS-assigned)
    #[arg(long, default_value = "0")]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("river_worker=info".parse()?),
        )
        .init();

    let args = Args::parse();

    let side = match args.side.as_str() {
        "left" => Side::Left,
        "right" => Side::Right,
        _ => {
            tracing::error!("Invalid side: {}. Must be 'left' or 'right'", args.side);
            std::process::exit(1);
        }
    };

    let config = WorkerConfig {
        orchestrator_endpoint: args.orchestrator.clone(),
        dyad: args.dyad.clone(),
        side: side.clone(),
        port: args.port,
    };

    // Bind HTTP server
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!("Worker listening on http://{}", local_addr);

    let worker_endpoint = format!("http://localhost:{}", local_addr.port());

    // Register with orchestrator
    tracing::info!("Registering with orchestrator at {}", args.orchestrator);
    let client = reqwest::Client::new();

    let reg_request = WorkerRegistrationRequest {
        endpoint: worker_endpoint.clone(),
        worker: WorkerRegistration {
            dyad: config.dyad.clone(),
            side: config.side.clone(),
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

    let registration: config::RegistrationResponse = response.json().await?;
    tracing::info!(
        "Registered as {:?} ({:?})",
        registration.baton,
        config.side
    );

    // Create shared state
    let state = new_shared_state(&config, registration.clone());

    // Update context limit from model config
    {
        let mut s = state.write().await;
        s.context_limit = s.model_config.context_limit;
    }

    // Load role file and inject into state
    let role_path = config.role_path(&registration);
    if role_path.exists() {
        let role_content = tokio::fs::read_to_string(&role_path).await?;
        tracing::info!("Loaded role from {:?}", role_path);
        let mut s = state.write().await;
        s.role_content = Some(role_content);
    }

    // Load identity file and inject into state
    let identity_path = config.identity_path(&registration);
    if identity_path.exists() {
        let identity_content = tokio::fs::read_to_string(&identity_path).await?;
        tracing::info!("Loaded identity from {:?}", identity_path);
        let mut s = state.write().await;
        s.identity_content = Some(identity_content);
    }

    // Store initial message if provided
    if let Some(initial_msg) = &registration.initial_message {
        let mut s = state.write().await;
        s.initial_message = Some(initial_msg.clone());
    }

    // Start HTTP server
    let state_clone = state.clone();
    let server = tokio::spawn(async move {
        let app = router(state_clone);
        axum::serve(listener, app).await
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    tracing::info!("Starting worker loop");

    // Run the main loop
    let output = run_loop(state.clone(), &config, &client).await;

    tracing::info!("Worker loop exited: {:?}", output.status);

    // Send output to orchestrator
    let _ = client
        .post(format!("{}/worker/output", args.orchestrator))
        .json(&output)
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    // Stop server
    server.abort();

    Ok(())
}

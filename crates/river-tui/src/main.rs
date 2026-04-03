//! River TUI - Terminal UI for debugging River Engine.

mod adapter;
mod http;
mod tui;

use adapter::AdapterState;
use clap::Parser;
use http::router;
use river_context::OpenAIMessage;
use river_protocol::{AdapterRegistration, AdapterRegistrationRequest, AdapterRegistrationResponse};
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};

#[derive(Parser, Debug)]
#[command(name = "river-tui")]
#[command(about = "Terminal UI for River Engine debugging")]
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

    /// Workspace directory (tails both left/context.jsonl and right/context.jsonl)
    #[arg(long)]
    workspace: Option<PathBuf>,
}

pub type SharedState = Arc<RwLock<AdapterState>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    // Initialize tracing - output to stderr so it doesn't interfere with TUI
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

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
        eprintln!("Registration failed: {}", msg);
        std::process::exit(1);
    }

    let registration: AdapterRegistrationResponse = response.json().await?;
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

    // Start context tailers for both sides if workspace provided
    if let Some(workspace) = args.workspace {
        // Tail left side
        let left_path = workspace.join("left").join("context.jsonl");
        let left_state = state.clone();
        let left_tx = ui_tx.clone();
        tokio::spawn(async move {
            tail_context(left_path, "left", left_state, left_tx).await;
        });

        // Tail right side
        let right_path = workspace.join("right").join("context.jsonl");
        let right_state = state.clone();
        let right_tx = ui_tx.clone();
        tokio::spawn(async move {
            tail_context(right_path, "right", right_state, right_tx).await;
        });
    }

    // Run TUI on main thread
    tui::run(state, ui_rx, registration.worker_endpoint).await?;

    Ok(())
}

/// Tail a context.jsonl file and update state with new entries.
async fn tail_context(
    path: PathBuf,
    side: &'static str,
    state: SharedState,
    ui_tx: mpsc::Sender<tui::UiEvent>,
) {
    loop {
        // Wait for file to exist
        if !path.exists() {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }

        // Read new lines
        let lines_read = {
            let s = state.read().await;
            s.context_lines_read(side)
        };

        let new_entries = match read_context_from_line(&path, lines_read) {
            Ok(entries) => entries,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
        };

        if !new_entries.is_empty() {
            let mut s = state.write().await;
            for entry in new_entries {
                s.add_context_entry(side, entry);
            }
            let _ = ui_tx.send(tui::UiEvent::Refresh).await;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Read context entries starting from a specific line.
fn read_context_from_line(path: &PathBuf, skip_lines: usize) -> std::io::Result<Vec<OpenAIMessage>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);

    let entries: Vec<OpenAIMessage> = reader
        .lines()
        .skip(skip_lines)
        .filter_map(|line| line.ok())
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect();

    Ok(entries)
}

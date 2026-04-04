//! River TUI - Terminal UI for debugging River Engine.

mod adapter;
mod http;
mod tui;

use adapter::AdapterState;
use clap::Parser;
use http::router;
use river_context::OpenAIMessage;
use river_protocol::conversation::Conversation;
use river_protocol::{AdapterRegistration, AdapterRegistrationRequest, AdapterRegistrationResponse};
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

    // Clone workspace before passing to tailers (we need it for TUI too)
    let workspace_for_tui = args.workspace.clone();

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

        // Tail backchannel
        let bc_workspace = workspace.clone();
        let bc_state = state.clone();
        let bc_tx = ui_tx.clone();
        tokio::spawn(async move {
            tail_backchannel(bc_workspace, bc_state, bc_tx).await;
        });
    }

    // Run TUI on main thread
    tui::run(state, ui_rx, registration.worker_endpoint, workspace_for_tui).await?;

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

        let new_entries = match read_context_from_line(&path, lines_read).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(side = side, error = %e, "Context read error");
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
async fn read_context_from_line(path: &PathBuf, skip_lines: usize) -> std::io::Result<Vec<OpenAIMessage>> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let file = tokio::fs::File::open(path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let mut entries = Vec::new();
    let mut line_num = 0;

    while let Some(line) = lines.next_line().await? {
        if line_num >= skip_lines && !line.trim().is_empty() {
            if let Ok(msg) = serde_json::from_str::<OpenAIMessage>(&line) {
                entries.push(msg);
            }
        }
        line_num += 1;
    }

    Ok(entries)
}

/// Tail the backchannel file and update state with new lines.
async fn tail_backchannel(
    workspace: PathBuf,
    state: SharedState,
    ui_tx: mpsc::Sender<tui::UiEvent>,
) {
    let path = workspace.join("conversations").join("backchannel.txt");
    let mut last_line_count = 0;

    loop {
        if path.exists() {
            if let Ok(convo) = Conversation::load(&path) {
                if convo.lines.len() > last_line_count {
                    let new_lines = convo.lines[last_line_count..].to_vec();
                    let mut s = state.write().await;
                    for line in new_lines {
                        s.add_backchannel_line(line);
                    }
                    last_line_count = convo.lines.len();
                    let _ = ui_tx.send(tui::UiEvent::Refresh).await;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_read_context_from_line_empty_file() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();

        let result = read_context_from_line(&path, 0).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_read_context_from_line_parses_valid_json() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"role": "user", "content": "Hello"}}"#).unwrap();
        writeln!(file, r#"{{"role": "assistant", "content": "Hi there"}}"#).unwrap();
        file.flush().unwrap();
        let path = file.path().to_path_buf();

        let result = read_context_from_line(&path, 0).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[0].content, Some("Hello".to_string()));
        assert_eq!(result[1].role, "assistant");
        assert_eq!(result[1].content, Some("Hi there".to_string()));
    }

    #[tokio::test]
    async fn test_read_context_from_line_skips_lines() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"role": "user", "content": "First"}}"#).unwrap();
        writeln!(file, r#"{{"role": "user", "content": "Second"}}"#).unwrap();
        writeln!(file, r#"{{"role": "user", "content": "Third"}}"#).unwrap();
        file.flush().unwrap();
        let path = file.path().to_path_buf();

        let result = read_context_from_line(&path, 2).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, Some("Third".to_string()));
    }

    #[tokio::test]
    async fn test_read_context_from_line_skips_empty_lines() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"role": "user", "content": "Hello"}}"#).unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "   ").unwrap();
        writeln!(file, r#"{{"role": "assistant", "content": "Hi"}}"#).unwrap();
        file.flush().unwrap();
        let path = file.path().to_path_buf();

        let result = read_context_from_line(&path, 0).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, Some("Hello".to_string()));
        assert_eq!(result[1].content, Some("Hi".to_string()));
    }

    #[tokio::test]
    async fn test_read_context_from_line_handles_invalid_json() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"role": "user", "content": "Valid"}}"#).unwrap();
        writeln!(file, "not valid json").unwrap();
        writeln!(file, r#"{{"role": "assistant", "content": "Also valid"}}"#).unwrap();
        file.flush().unwrap();
        let path = file.path().to_path_buf();

        let result = read_context_from_line(&path, 0).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, Some("Valid".to_string()));
        assert_eq!(result[1].content, Some("Also valid".to_string()));
    }

    #[tokio::test]
    async fn test_read_context_from_line_file_not_found() {
        let path = PathBuf::from("/nonexistent/path/to/file.jsonl");
        let result = read_context_from_line(&path, 0).await;
        assert!(result.is_err());
    }
}

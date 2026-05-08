# TUI Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `river-tui`, a terminal chat adapter that speaks the same HTTP protocol as the Discord adapter but uses a ratatui terminal interface as the platform.

**Architecture:** New crate `crates/river-tui` with five modules: CLI config, gateway HTTP client, inbound HTTP server (receives agent messages), shared message state, and the ratatui TUI. Two async tasks run concurrently: the TUI event loop and the HTTP server. They share a message buffer via `Arc<Mutex<Vec<ChatLine>>>` and a `tokio::sync::Notify` so the TUI re-renders immediately when the server receives a message.

**Tech Stack:** Rust, ratatui, crossterm, axum, reqwest, tokio, clap

**Revised:** Incorporates Gemini review findings — protocol alignment, terminal cleanup guard, tracing to file, proper notify usage, HTTP server error handling.

---

## File Structure

| File | Purpose |
|------|---------|
| `crates/river-tui/Cargo.toml` | Crate manifest |
| `crates/river-tui/src/main.rs` | CLI parsing, task spawning, main entry |
| `crates/river-tui/src/config.rs` | CLI args and runtime config |
| `crates/river-tui/src/gateway.rs` | HTTP client for gateway (send incoming, health check, registration) |
| `crates/river-tui/src/server.rs` | Axum HTTP server (receives /send from gateway, serves /health) |
| `crates/river-tui/src/state.rs` | Shared message buffer and notify channel |
| `crates/river-tui/src/tui.rs` | Ratatui rendering and crossterm input loop |
| `crates/river-tui/src/lib.rs` | Module declarations and re-exports |

## Reference Files

- `crates/river-discord/src/config.rs` — CLI arg pattern to follow
- `crates/river-discord/src/gateway.rs` — Gateway client pattern to follow
- `crates/river-discord/src/adapter.rs` — Registration pattern to follow
- `crates/river-discord/src/outbound.rs` — HTTP server pattern to follow
- `crates/river-adapter/src/types.rs` — `SendRequest`, `SendResponse` types
- `crates/river-adapter/src/registration.rs` — `AdapterInfo`, `RegisterRequest`
- `crates/river-gateway/src/api/routes.rs:100-129` — `IncomingMessage` and `Author` structs the gateway expects

## Protocol Notes

The gateway's `IncomingMessage` (`routes.rs:102`) has these fields:
- `adapter: String` (required)
- `event_type: String` (required)
- `channel: String` (required)
- `channel_name: Option<String>` (optional, serde default)
- `guild_id: Option<String>` (optional, serde default)
- `guild_name: Option<String>` (optional, serde default)
- `author: Author` (required — `Author` has only `id: String` and `name: String`, NO `is_bot`)
- `content: String` (required)
- `message_id: Option<String>` (optional)
- `metadata: Option<Value>` (optional)
- `priority: Priority` (optional, defaults to Interactive)

The TUI's outgoing struct must serialize to match this. Note: `Author` does NOT have `is_bot` — the gateway ignores unknown fields via serde but we should match the contract exactly.

## Known Limitations (v1)

- Scroll auto-follow counts messages, not wrapped lines. Long messages that wrap will cause the auto-scroll to stop short of the true bottom. Accepted for v1.

---

### Task 1: Crate Scaffold

**Files:**
- Create: `crates/river-tui/Cargo.toml`
- Create: `crates/river-tui/src/lib.rs`
- Create: `crates/river-tui/src/main.rs`

- [ ] **Step 1: Create Cargo.toml**

Create `crates/river-tui/Cargo.toml`:

```toml
[package]
name = "river-tui"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "River Engine TUI adapter"

[[bin]]
name = "river-tui"
path = "src/main.rs"

[dependencies]
river-adapter = { path = "../river-adapter" }
tokio.workspace = true
axum.workspace = true
reqwest.workspace = true
tracing.workspace = true
tracing-subscriber = { workspace = true, features = ["env-filter"] }
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
chrono.workspace = true
anyhow.workspace = true
ratatui.workspace = true
crossterm.workspace = true

[dev-dependencies]
tower = { workspace = true, features = ["util"] }
```

- [ ] **Step 2: Create lib.rs**

Create `crates/river-tui/src/lib.rs`:

```rust
pub mod config;
pub mod gateway;
pub mod server;
pub mod state;
pub mod tui;
```

- [ ] **Step 3: Create minimal main.rs**

Create `crates/river-tui/src/main.rs`:

```rust
fn main() {
    println!("river-tui");
}
```

- [ ] **Step 4: Verify crate compiles**

```bash
cd /home/cassie/river-engine && cargo check -p river-tui
```

Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add crates/river-tui/
git commit -m "feat(tui): scaffold river-tui crate"
```

---

### Task 2: Config and Shared State

**Files:**
- Create: `crates/river-tui/src/config.rs`
- Create: `crates/river-tui/src/state.rs`

- [ ] **Step 1: Write config.rs**

Create `crates/river-tui/src/config.rs`:

```rust
//! CLI args and runtime config

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-tui")]
#[command(about = "River Engine TUI Adapter")]
pub struct Args {
    /// River gateway URL
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    pub gateway_url: String,

    /// Port for the TUI's HTTP server (0 = OS-assigned)
    #[arg(long, default_value = "0")]
    pub listen_port: u16,

    /// User display name
    #[arg(long)]
    pub name: Option<String>,

    /// Channel ID for messages
    #[arg(long, default_value = "terminal")]
    pub channel: String,

    /// Path to file containing gateway auth token
    #[arg(long)]
    pub auth_token_file: Option<PathBuf>,

    /// Log file path (default: river-tui.log in current directory)
    #[arg(long)]
    pub log_file: Option<PathBuf>,
}

/// Runtime configuration
#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub gateway_url: String,
    pub listen_port: u16,
    pub user_name: String,
    pub channel: String,
    pub auth_token: Option<String>,
    pub log_file: PathBuf,
}

impl TuiConfig {
    pub fn from_args(args: Args) -> anyhow::Result<Self> {
        let user_name = args.name.unwrap_or_else(|| {
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "user".to_string())
        });

        let auth_token = if let Some(ref path) = args.auth_token_file {
            let token = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("Failed to read auth token file: {}", e))?
                .trim()
                .to_string();
            Some(token)
        } else {
            None
        };

        let log_file = args.log_file.unwrap_or_else(|| PathBuf::from("river-tui.log"));

        Ok(Self {
            gateway_url: args.gateway_url,
            listen_port: args.listen_port,
            user_name,
            channel: args.channel,
            auth_token,
            log_file,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_defaults() {
        let args = Args::parse_from(["river-tui"]);
        assert_eq!(args.gateway_url, "http://127.0.0.1:3000");
        assert_eq!(args.listen_port, 0);
        assert_eq!(args.channel, "terminal");
        assert!(args.name.is_none());
    }

    #[test]
    fn test_args_custom() {
        let args = Args::parse_from([
            "river-tui",
            "--name", "cassie",
            "--channel", "dev",
            "--listen-port", "8082",
        ]);
        assert_eq!(args.name, Some("cassie".to_string()));
        assert_eq!(args.channel, "dev");
        assert_eq!(args.listen_port, 8082);
    }
}
```

- [ ] **Step 2: Write state.rs**

Create `crates/river-tui/src/state.rs`:

```rust
//! Shared state between TUI and HTTP server

use chrono::{DateTime, Local};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// A single line in the chat display
#[derive(Debug, Clone)]
pub struct ChatLine {
    pub timestamp: DateTime<Local>,
    pub sender: String,
    pub content: String,
    pub is_agent: bool,
}

/// Shared state between the TUI task and HTTP server task
#[derive(Clone)]
pub struct SharedState {
    /// Message buffer — append-only
    pub messages: Arc<Mutex<Vec<ChatLine>>>,
    /// Notify the TUI to re-render when a new message arrives
    pub notify: Arc<Notify>,
    /// Gateway connection status
    pub gateway_connected: Arc<std::sync::atomic::AtomicBool>,
    /// HTTP server status
    pub server_healthy: Arc<std::sync::atomic::AtomicBool>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            gateway_connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            server_healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    /// Push a message and notify the TUI to re-render
    pub fn push_message(&self, line: ChatLine) {
        self.messages.lock().unwrap().push(line);
        self.notify.notify_one();
    }

    /// Get a snapshot of all messages
    pub fn get_messages(&self) -> Vec<ChatLine> {
        self.messages.lock().unwrap().clone()
    }

    pub fn is_gateway_connected(&self) -> bool {
        self.gateway_connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_gateway_connected(&self, connected: bool) {
        self.gateway_connected.store(connected, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_server_healthy(&self) -> bool {
        self.server_healthy.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_server_healthy(&self, healthy: bool) {
        self.server_healthy.store(healthy, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_get_messages() {
        let state = SharedState::new();
        state.push_message(ChatLine {
            timestamp: Local::now(),
            sender: "user".into(),
            content: "hello".into(),
            is_agent: false,
        });
        let msgs = state.get_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello");
    }

    #[test]
    fn test_gateway_connected_default() {
        let state = SharedState::new();
        assert!(!state.is_gateway_connected());
    }

    #[test]
    fn test_server_healthy_default() {
        let state = SharedState::new();
        assert!(state.is_server_healthy());
    }
}
```

- [ ] **Step 3: Verify it compiles and tests pass**

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-tui/
git commit -m "feat(tui): config and shared state modules"
```

---

### Task 3: Gateway Client

**Files:**
- Create: `crates/river-tui/src/gateway.rs`

- [ ] **Step 1: Write gateway.rs**

Create `crates/river-tui/src/gateway.rs`:

```rust
//! HTTP client for river-gateway communication

use reqwest::Client;
use river_adapter::{AdapterInfo, RegisterRequest};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

/// Message sent to gateway /incoming endpoint.
/// Matches the gateway's IncomingMessage struct (routes.rs:102).
#[derive(Debug, Serialize)]
pub struct IncomingMessage {
    pub adapter: String,
    pub event_type: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// Author for incoming messages.
/// Matches the gateway's Author struct (routes.rs:126) — id and name only.
#[derive(Debug, Serialize)]
pub struct Author {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct IncomingResponse {
    pub status: String,
}

/// HTTP client for river-gateway
pub struct GatewayClient {
    client: Client,
    base_url: String,
    auth_token: Option<String>,
}

impl GatewayClient {
    pub fn new(base_url: String, auth_token: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, base_url, auth_token }
    }

    /// Send a user message to the gateway
    pub async fn send_incoming(&self, msg: IncomingMessage) -> Result<IncomingResponse, String> {
        let url = format!("{}/incoming", self.base_url);
        let mut req = self.client.post(&url).json(&msg);

        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response = req
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("gateway returned status {}", response.status()));
        }

        response
            .json()
            .await
            .map_err(|e| format!("parse error: {}", e))
    }

    /// Check if gateway is reachable
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/health", self.base_url);
        self.client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Register this adapter with the gateway
    pub async fn register(&self, listen_port: u16) -> Result<(), String> {
        let url = format!("{}/adapters/register", self.base_url);
        let info = AdapterInfo {
            name: "tui".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            url: format!("http://127.0.0.1:{}", listen_port),
            features: HashSet::new(),
            metadata: serde_json::json!({}),
        };

        let mut req = self.client
            .post(&url)
            .json(&RegisterRequest { adapter: info });

        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response: river_adapter::RegisterResponse = req
            .send()
            .await
            .map_err(|e| format!("registration request failed: {}", e))?
            .json()
            .await
            .map_err(|e| format!("failed to parse registration response: {}", e))?;

        if response.accepted {
            Ok(())
        } else {
            Err(response.error.unwrap_or_else(|| "registration rejected".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message_serialization() {
        let msg = IncomingMessage {
            adapter: "tui".into(),
            event_type: "MessageCreate".into(),
            channel: "terminal".into(),
            author: Author {
                id: "local-user".into(),
                name: "cassie".into(),
            },
            content: "hello".into(),
            message_id: Some("msg-1".into()),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"adapter\":\"tui\""));
        assert!(json.contains("\"name\":\"cassie\""));
        // Verify no is_bot field
        assert!(!json.contains("is_bot"));
    }

    #[test]
    fn test_incoming_message_no_message_id() {
        let msg = IncomingMessage {
            adapter: "tui".into(),
            event_type: "MessageCreate".into(),
            channel: "terminal".into(),
            author: Author {
                id: "local-user".into(),
                name: "cassie".into(),
            },
            content: "hello".into(),
            message_id: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        // skip_serializing_if means message_id should be absent
        assert!(!json.contains("message_id"));
    }

    #[test]
    fn test_gateway_client_creation() {
        let client = GatewayClient::new("http://localhost:3000".into(), None);
        assert_eq!(client.base_url, "http://localhost:3000");
    }
}
```

- [ ] **Step 2: Verify it compiles and tests pass**

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-tui/src/gateway.rs
git commit -m "feat(tui): gateway client — send incoming, health check, registration"
```

---

### Task 4: HTTP Server (Inbound from Gateway)

**Files:**
- Create: `crates/river-tui/src/server.rs`

- [ ] **Step 1: Write server.rs**

Create `crates/river-tui/src/server.rs`:

```rust
//! HTTP server — receives outbound messages from the gateway

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use river_adapter::{SendRequest, SendResponse};
use chrono::Local;

use crate::state::{ChatLine, SharedState};

/// Health check response
#[derive(serde::Serialize)]
struct HealthResponse {
    healthy: bool,
}

/// Create the HTTP router
pub fn create_router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/send", post(handle_send))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { healthy: true })
}

async fn handle_send(
    State(state): State<SharedState>,
    Json(req): Json<SendRequest>,
) -> Result<Json<SendResponse>, StatusCode> {
    let line = ChatLine {
        timestamp: Local::now(),
        sender: "agent".into(),
        content: req.content,
        is_agent: true,
    };

    state.push_message(line);

    let msg_id = format!("tui-{}", chrono::Utc::now().timestamp_millis());

    Ok(Json(SendResponse {
        success: true,
        message_id: Some(msg_id),
        error: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_check() {
        let state = SharedState::new();
        let app = create_router(state);
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_handle_send() {
        let state = SharedState::new();
        let app = create_router(state.clone());

        let body = serde_json::json!({
            "channel": "terminal",
            "content": "Hello from agent!"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/send")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let msgs = state.get_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello from agent!");
        assert!(msgs[0].is_agent);
    }
}
```

- [ ] **Step 2: Verify it compiles and tests pass**

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-tui/src/server.rs
git commit -m "feat(tui): HTTP server — receives /send from gateway"
```

---

### Task 5: TUI Rendering and Input

**Files:**
- Create: `crates/river-tui/src/tui.rs`

- [ ] **Step 1: Write tui.rs**

Create `crates/river-tui/src/tui.rs`:

```rust
//! Ratatui terminal interface

use crate::gateway::{Author, GatewayClient, IncomingMessage};
use crate::state::{ChatLine, SharedState};
use chrono::Local;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use std::io::stdout;
use std::sync::Arc;

/// Run the TUI event loop. Ensures terminal cleanup on all exit paths.
pub async fn run(
    state: SharedState,
    gateway: Arc<GatewayClient>,
    user_name: String,
    channel: String,
) -> anyhow::Result<()> {
    // Set up terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    // Run the inner loop, capturing the result
    let result = run_inner(state, gateway, user_name, channel).await;

    // Always restore terminal, even on error/panic
    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);

    result
}

async fn run_inner(
    state: SharedState,
    gateway: Arc<GatewayClient>,
    user_name: String,
    channel: String,
) -> anyhow::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut input = String::new();
    let mut scroll_offset: u16 = 0;
    let mut follow_tail = true;

    loop {
        // Draw
        let messages = state.get_messages();
        let connected = state.is_gateway_connected();
        let server_ok = state.is_server_healthy();

        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),    // message log
                    Constraint::Length(1),  // status bar
                    Constraint::Length(3),  // input
                ])
                .split(frame.area());

            // --- Message log ---
            let msg_lines: Vec<Line> = messages.iter().map(|m| {
                let time = m.timestamp.format("%H:%M").to_string();
                let name_color = if m.is_agent { Color::Cyan } else { Color::Green };
                Line::from(vec![
                    Span::styled(format!("{} ", time), Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}: ", m.sender), Style::default().fg(name_color)),
                    Span::raw(&m.content),
                ])
            }).collect();

            let msg_widget = Paragraph::new(msg_lines.clone())
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false });

            // Auto-scroll (counts messages, not wrapped lines — known v1 limitation)
            let inner_height = chunks[0].height.saturating_sub(2);
            let total_lines = msg_lines.len() as u16;
            if follow_tail && total_lines > inner_height {
                scroll_offset = total_lines.saturating_sub(inner_height);
            }

            let msg_widget = msg_widget.scroll((scroll_offset, 0));
            frame.render_widget(msg_widget, chunks[0]);

            // --- Status bar ---
            let gw_indicator = if connected { "●" } else { "○" };
            let gw_color = if connected { Color::Green } else { Color::Red };
            let gw_text = if connected { "connected" } else { "disconnected" };

            let mut status_spans = vec![
                Span::raw(" [tui "),
                Span::styled(gw_indicator, Style::default().fg(gw_color)),
                Span::raw(format!(" gateway: {}", gw_text)),
            ];

            if !server_ok {
                status_spans.push(Span::styled(" | server: down", Style::default().fg(Color::Red)));
            }

            status_spans.push(Span::raw("]"));

            let status_widget = Paragraph::new(Line::from(status_spans))
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(status_widget, chunks[1]);

            // --- Input ---
            let input_widget = Paragraph::new(format!("> {}_", input))
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(input_widget, chunks[2]);
        })?;

        // Wait for either a crossterm event or a notify signal
        tokio::select! {
            // Check for terminal input events
            poll_result = tokio::task::spawn_blocking(|| {
                event::poll(std::time::Duration::from_millis(100)).unwrap_or(false)
            }) => {
                if !poll_result.unwrap_or(false) {
                    continue;
                }

                let evt = tokio::task::block_in_place(|| event::read())?;

                match evt {
                    Event::Key(key) => {
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                            (KeyCode::Enter, _) if !input.is_empty() => {
                                let content = std::mem::take(&mut input);
                                follow_tail = true;

                                // Add to local display
                                state.push_message(ChatLine {
                                    timestamp: Local::now(),
                                    sender: user_name.clone(),
                                    content: content.clone(),
                                    is_agent: false,
                                });

                                // Send to gateway
                                let gw = gateway.clone();
                                let ch = channel.clone();
                                let name = user_name.clone();
                                tokio::spawn(async move {
                                    let msg = IncomingMessage {
                                        adapter: "tui".into(),
                                        event_type: "MessageCreate".into(),
                                        channel: ch,
                                        author: Author {
                                            id: "local-user".into(),
                                            name,
                                        },
                                        content,
                                        message_id: Some(format!("tui-{}", chrono::Utc::now().timestamp_millis())),
                                    };
                                    if let Err(e) = gw.send_incoming(msg).await {
                                        tracing::error!(error = %e, "Failed to send message to gateway");
                                    }
                                });
                            }
                            (KeyCode::Char(c), _) => {
                                input.push(c);
                            }
                            (KeyCode::Backspace, _) => { input.pop(); }
                            (KeyCode::Up, _) => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(1);
                            }
                            (KeyCode::Down, _) => {
                                scroll_offset = scroll_offset.saturating_add(1);
                                // Re-enable follow if scrolled past the end
                                let total = state.get_messages().len() as u16;
                                if scroll_offset >= total {
                                    follow_tail = true;
                                }
                            }
                            (KeyCode::PageUp, _) => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(10);
                            }
                            (KeyCode::PageDown, _) => {
                                scroll_offset = scroll_offset.saturating_add(10);
                                let total = state.get_messages().len() as u16;
                                if scroll_offset >= total {
                                    follow_tail = true;
                                }
                            }
                            _ => {}
                        }
                    }
                    Event::Resize(_, _) => {} // re-render on next loop
                    _ => {}
                }
            }
            // Wake up when a new message arrives from the gateway
            _ = state.notify.notified() => {
                // New message arrived — the next loop iteration will re-render
                follow_tail = true;
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd /home/cassie/river-engine && cargo check -p river-tui
```

Expected: compiles clean.

- [ ] **Step 3: Commit**

```bash
git add crates/river-tui/src/tui.rs
git commit -m "feat(tui): ratatui rendering and crossterm input loop with cleanup guard"
```

---

### Task 6: Main Entry Point

**Files:**
- Modify: `crates/river-tui/src/main.rs`

- [ ] **Step 1: Write the full main.rs**

Replace `crates/river-tui/src/main.rs`:

```rust
use clap::Parser;
use river_tui::config::{Args, TuiConfig};
use river_tui::gateway::GatewayClient;
use river_tui::server::create_router;
use river_tui::state::SharedState;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = TuiConfig::from_args(args)?;

    // Log to file, not stdout — stdout is owned by ratatui
    let log_file = std::fs::File::create(&config.log_file)?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    tracing::info!("Starting River TUI Adapter");
    tracing::info!("Gateway: {}", config.gateway_url);
    tracing::info!("User: {}", config.user_name);
    tracing::info!("Channel: {}", config.channel);

    let state = SharedState::new();
    let gateway = Arc::new(GatewayClient::new(
        config.gateway_url.clone(),
        config.auth_token.clone(),
    ));

    // Spawn HTTP server — bind to configured port (0 = OS-assigned)
    let http_state = state.clone();
    let listener = tokio::net::TcpListener::bind(
        format!("127.0.0.1:{}", config.listen_port)
    ).await?;
    let actual_port = listener.local_addr()?.port();
    tracing::info!("HTTP server listening on 127.0.0.1:{}", actual_port);

    let server_state = state.clone();
    tokio::spawn(async move {
        let app = create_router(http_state);
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "HTTP server failed");
            server_state.set_server_healthy(false);
        }
    });

    // Register with gateway (non-blocking, non-fatal)
    let gw_for_register = gateway.clone();
    tokio::spawn(async move {
        for attempt in 1..=3 {
            match gw_for_register.register(actual_port).await {
                Ok(()) => {
                    tracing::info!("Registered with gateway on attempt {}", attempt);
                    return;
                }
                Err(e) => {
                    tracing::warn!("Failed to register with gateway (attempt {}): {}", attempt, e);
                    tokio::time::sleep(std::time::Duration::from_secs(5 * attempt as u64)).await;
                }
            }
        }
        tracing::warn!("Failed to register with gateway after 3 attempts (continuing anyway)");
    });

    // Spawn gateway health check loop
    let gw_for_health = gateway.clone();
    let state_for_health = state.clone();
    tokio::spawn(async move {
        loop {
            let reachable = gw_for_health.health_check().await;
            state_for_health.set_gateway_connected(reachable);
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });

    // Run TUI (blocks until Ctrl-C, terminal cleanup is guaranteed)
    river_tui::tui::run(state, gateway, config.user_name, config.channel).await?;

    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd /home/cassie/river-engine && cargo check -p river-tui
```

- [ ] **Step 3: Build the binary**

```bash
cd /home/cassie/river-engine && cargo build -p river-tui
```

Expected: produces `target/debug/river-tui`.

- [ ] **Step 4: Commit**

```bash
git add crates/river-tui/src/main.rs
git commit -m "feat(tui): main entry — file logging, server error handling, cleanup guard"
```

---

### Task 7: Integration Test

**Files:** none (manual testing)

- [ ] **Step 1: Run the TUI without a gateway**

```bash
cd /home/cassie/river-engine && cargo run -p river-tui
```

Expected: terminal enters alternate screen, shows the chat layout with empty message log, status bar showing `gateway: disconnected`, and input prompt. Ctrl-C exits cleanly and restores the terminal. Check `river-tui.log` for log output (not on screen).

- [ ] **Step 2: Run with a gateway (if one is running)**

```bash
cd /home/cassie/river-engine && cargo run -p river-tui -- --gateway-url http://127.0.0.1:3000 --name cassie
```

Expected: TUI starts, status bar shows `gateway: connected` after health check (up to 30s), typing a message + Enter sends it to the gateway.

- [ ] **Step 3: Run the full test suite**

```bash
cd /home/cassie/river-engine && cargo test
```

Expected: all existing tests pass, plus the new river-tui tests.

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix(tui): adjustments from integration testing"
```

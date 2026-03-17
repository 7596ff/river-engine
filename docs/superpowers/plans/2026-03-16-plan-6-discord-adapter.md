# Discord Adapter Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone Discord adapter using Twilight that routes messages between Discord and river-gateway.

**Architecture:** Separate `river-discord` crate running as its own binary. Connects to Discord via Twilight websocket, forwards messages to gateway via HTTP POST to `/incoming`. Receives outbound messages via HTTP server that gateway calls. Channel management via slash commands and admin API.

**Tech Stack:** Twilight (gateway, http, model), Axum (HTTP server), Reqwest (HTTP client), Tokio, Clap, Tracing

**Spec:** `docs/superpowers/specs/2026-03-16-discord-adapter-design.md`

---

## File Structure

```
crates/river-discord/
├── Cargo.toml
├── src/
│   ├── main.rs          # CLI entry, startup, shutdown
│   ├── lib.rs           # Public exports
│   ├── config.rs        # DiscordConfig, CLI args
│   ├── channels.rs      # ChannelState (RwLock<HashSet>), persistence
│   ├── client.rs        # DiscordClient wrapper around Twilight
│   ├── handler.rs       # Event handler (messages, reactions)
│   ├── gateway.rs       # GatewayClient (HTTP to river-gateway)
│   ├── outbound.rs      # HTTP server (POST /send, admin API)
│   └── commands.rs      # Slash command registration and handling
```

---

## Chunk 1: Project Setup and Configuration

### Task 1: Create crate and add dependencies

**Files:**
- Create: `crates/river-discord/Cargo.toml`
- Modify: `Cargo.toml` (workspace - add twilight deps)

- [ ] **Step 1: Add Twilight dependencies to workspace**

Add to `Cargo.toml` workspace dependencies:

```toml
twilight-gateway = "0.16"
twilight-http = "0.16"
twilight-model = "0.16"
twilight-util = { version = "0.16", features = ["builder"] }
```

- [ ] **Step 2: Create river-discord Cargo.toml**

Create `crates/river-discord/Cargo.toml`:

```toml
[package]
name = "river-discord"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "River Engine Discord adapter using Twilight"

[[bin]]
name = "river-discord"
path = "src/main.rs"

[dependencies]
river-core = { path = "../river-core" }
tokio.workspace = true
axum.workspace = true
reqwest.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
anyhow.workspace = true
twilight-gateway.workspace = true
twilight-http.workspace = true
twilight-model.workspace = true
twilight-util.workspace = true

[dev-dependencies]
tempfile = "3.10"
tower = { workspace = true, features = ["util"] }
```

- [ ] **Step 3: Create minimal lib.rs**

Create `crates/river-discord/src/lib.rs`:

```rust
//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod config;
```

- [ ] **Step 4: Create minimal main.rs**

Create `crates/river-discord/src/main.rs`:

```rust
fn main() {
    println!("river-discord");
}
```

- [ ] **Step 5: Verify compilation**

Run: `cargo build -p river-discord`
Expected: Compiles successfully

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/river-discord/
git commit -m "feat(discord): create river-discord crate with dependencies"
```

---

### Task 2: Create configuration types

**Files:**
- Create: `crates/river-discord/src/config.rs`

- [ ] **Step 1: Create config module**

Create `crates/river-discord/src/config.rs`:

```rust
//! Configuration types

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-discord")]
#[command(about = "River Engine Discord Adapter")]
pub struct Args {
    /// Discord bot token file
    #[arg(long)]
    pub token_file: PathBuf,

    /// River gateway URL
    #[arg(long, default_value = "http://localhost:3000")]
    pub gateway_url: String,

    /// Port for adapter HTTP server
    #[arg(long, default_value = "3002")]
    pub listen_port: u16,

    /// Initial channel IDs (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub channels: Vec<u64>,

    /// State file for channel persistence
    #[arg(long)]
    pub state_file: Option<PathBuf>,

    /// Guild ID for slash command registration
    #[arg(long)]
    pub guild_id: u64,
}

/// Runtime configuration
#[derive(Debug, Clone)]
pub struct DiscordConfig {
    pub token: String,
    pub gateway_url: String,
    pub listen_port: u16,
    pub initial_channels: Vec<u64>,
    pub state_file: Option<PathBuf>,
    pub guild_id: u64,
}

impl DiscordConfig {
    /// Load configuration from CLI args
    pub fn from_args(args: Args) -> anyhow::Result<Self> {
        let token = std::fs::read_to_string(&args.token_file)
            .map_err(|e| anyhow::anyhow!("Failed to read token file: {}", e))?
            .trim()
            .to_string();

        if token.is_empty() {
            anyhow::bail!("Token file is empty");
        }

        Ok(Self {
            token,
            gateway_url: args.gateway_url,
            listen_port: args.listen_port,
            initial_channels: args.channels,
            state_file: args.state_file,
            guild_id: args.guild_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_defaults() {
        // Verify clap parsing works with minimal args
        let args = Args::parse_from([
            "river-discord",
            "--token-file", "/tmp/token",
            "--guild-id", "123456",
        ]);
        assert_eq!(args.gateway_url, "http://localhost:3000");
        assert_eq!(args.listen_port, 3002);
        assert!(args.channels.is_empty());
    }

    #[test]
    fn test_args_with_channels() {
        let args = Args::parse_from([
            "river-discord",
            "--token-file", "/tmp/token",
            "--guild-id", "123456",
            "--channels", "111,222,333",
        ]);
        assert_eq!(args.channels, vec![111, 222, 333]);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-discord config`
Expected: 2 tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-discord/src/config.rs
git commit -m "feat(discord): add configuration types"
```

---

### Task 3: Create channel state management

**Files:**
- Create: `crates/river-discord/src/channels.rs`
- Modify: `crates/river-discord/src/lib.rs`

- [ ] **Step 1: Create channel state module**

Create `crates/river-discord/src/channels.rs`:

```rust
//! Channel state management

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Persisted channel state format
#[derive(Debug, Serialize, Deserialize)]
struct PersistedState {
    version: u32,
    channels: Vec<u64>,
}

/// Thread-safe channel state
pub struct ChannelState {
    channels: RwLock<HashSet<u64>>,
    state_file: Option<std::path::PathBuf>,
}

impl ChannelState {
    /// Create new channel state with initial channels
    pub fn new(initial_channels: Vec<u64>, state_file: Option<std::path::PathBuf>) -> Arc<Self> {
        let channels: HashSet<u64> = initial_channels.into_iter().collect();
        Arc::new(Self {
            channels: RwLock::new(channels),
            state_file,
        })
    }

    /// Load state from file, falling back to initial channels
    pub async fn load(
        initial_channels: Vec<u64>,
        state_file: Option<std::path::PathBuf>,
    ) -> Arc<Self> {
        let channels = if let Some(ref path) = state_file {
            Self::load_from_file(path).unwrap_or_else(|| initial_channels.into_iter().collect())
        } else {
            initial_channels.into_iter().collect()
        };

        Arc::new(Self {
            channels: RwLock::new(channels),
            state_file,
        })
    }

    fn load_from_file(path: &Path) -> Option<HashSet<u64>> {
        let content = std::fs::read_to_string(path).ok()?;
        let state: PersistedState = serde_json::from_str(&content).ok()?;
        if state.version != 1 {
            tracing::warn!("Unknown state file version, ignoring");
            return None;
        }
        Some(state.channels.into_iter().collect())
    }

    /// Check if a channel is being listened to
    pub async fn contains(&self, channel_id: u64) -> bool {
        self.channels.read().await.contains(&channel_id)
    }

    /// Add a channel to the listen set
    pub async fn add(&self, channel_id: u64) -> bool {
        let mut channels = self.channels.write().await;
        let added = channels.insert(channel_id);
        if added {
            drop(channels);
            self.persist().await;
        }
        added
    }

    /// Remove a channel from the listen set
    pub async fn remove(&self, channel_id: u64) -> bool {
        let mut channels = self.channels.write().await;
        let removed = channels.remove(&channel_id);
        if removed {
            drop(channels);
            self.persist().await;
        }
        removed
    }

    /// Get all channel IDs
    pub async fn list(&self) -> Vec<u64> {
        self.channels.read().await.iter().copied().collect()
    }

    /// Get channel count
    pub async fn count(&self) -> usize {
        self.channels.read().await.len()
    }

    /// Persist state to file (atomic write)
    async fn persist(&self) {
        let Some(ref path) = self.state_file else {
            return;
        };

        let channels: Vec<u64> = self.channels.read().await.iter().copied().collect();
        let state = PersistedState {
            version: 1,
            channels,
        };

        let content = match serde_json::to_string_pretty(&state) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to serialize state: {}", e);
                return;
            }
        };

        // Atomic write: write to temp file then rename
        let temp_path = path.with_extension("tmp");
        if let Err(e) = std::fs::write(&temp_path, &content) {
            tracing::error!("Failed to write state file: {}", e);
            return;
        }
        if let Err(e) = std::fs::rename(&temp_path, path) {
            tracing::error!("Failed to rename state file: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_channel_state_basic() {
        let state = ChannelState::new(vec![1, 2, 3], None);

        assert!(state.contains(1).await);
        assert!(state.contains(2).await);
        assert!(!state.contains(99).await);
        assert_eq!(state.count().await, 3);
    }

    #[tokio::test]
    async fn test_channel_state_add_remove() {
        let state = ChannelState::new(vec![], None);

        assert!(state.add(100).await);
        assert!(state.contains(100).await);
        assert!(!state.add(100).await); // already exists

        assert!(state.remove(100).await);
        assert!(!state.contains(100).await);
        assert!(!state.remove(100).await); // already removed
    }

    #[tokio::test]
    async fn test_channel_state_persistence() {
        let dir = tempdir().unwrap();
        let state_file = dir.path().join("channels.json");

        // Create and populate state
        {
            let state = ChannelState::new(vec![], Some(state_file.clone()));
            state.add(111).await;
            state.add(222).await;
        }

        // Load state from file
        let state = ChannelState::load(vec![], Some(state_file)).await;
        assert!(state.contains(111).await);
        assert!(state.contains(222).await);
        assert_eq!(state.count().await, 2);
    }

    #[tokio::test]
    async fn test_channel_state_list() {
        let state = ChannelState::new(vec![5, 3, 1], None);
        let mut list = state.list().await;
        list.sort();
        assert_eq!(list, vec![1, 3, 5]);
    }
}
```

- [ ] **Step 2: Update lib.rs**

Update `crates/river-discord/src/lib.rs`:

```rust
//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod channels;
pub mod config;

pub use channels::ChannelState;
pub use config::{Args, DiscordConfig};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-discord channels`
Expected: 4 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-discord/src/
git commit -m "feat(discord): add channel state management with persistence"
```

---

## Chunk 2: Gateway Client and Event Handling

### Task 4: Create gateway HTTP client

**Files:**
- Create: `crates/river-discord/src/gateway.rs`
- Modify: `crates/river-discord/src/lib.rs`

- [ ] **Step 1: Create gateway client module**

Create `crates/river-discord/src/gateway.rs`:

```rust
//! HTTP client for river-gateway communication

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Message author info
#[derive(Debug, Serialize)]
pub struct Author {
    pub id: String,
    pub name: String,
}

/// Metadata for incoming events
#[derive(Debug, Serialize, Default)]
pub struct EventMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

/// Incoming event sent to gateway
#[derive(Debug, Serialize)]
pub struct IncomingEvent {
    pub adapter: &'static str,
    pub event_type: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    pub message_id: String,
    pub metadata: EventMetadata,
}

/// Response from gateway /incoming endpoint
#[derive(Debug, Deserialize)]
pub struct IncomingResponse {
    pub status: String,
    pub channel: String,
}

/// HTTP client for river-gateway
pub struct GatewayClient {
    client: Client,
    base_url: String,
}

impl GatewayClient {
    /// Create a new gateway client
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, base_url }
    }

    /// Send an incoming event to the gateway
    pub async fn send_incoming(&self, event: IncomingEvent) -> Result<IncomingResponse, GatewayError> {
        let url = format!("{}/incoming", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(&event)
            .send()
            .await
            .map_err(|e| GatewayError::Request(e.to_string()))?;

        if !response.status().is_success() {
            return Err(GatewayError::Response(format!(
                "Gateway returned status {}",
                response.status()
            )));
        }

        response
            .json()
            .await
            .map_err(|e| GatewayError::Parse(e.to_string()))
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
}

/// Gateway communication errors
#[derive(Debug)]
pub enum GatewayError {
    Request(String),
    Response(String),
    Parse(String),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatewayError::Request(e) => write!(f, "request failed: {}", e),
            GatewayError::Response(e) => write!(f, "bad response: {}", e),
            GatewayError::Parse(e) => write!(f, "parse error: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_event_serialization() {
        let event = IncomingEvent {
            adapter: "discord",
            event_type: "message".to_string(),
            channel: "123456".to_string(),
            author: Author {
                id: "user123".to_string(),
                name: "TestUser".to_string(),
            },
            content: "Hello world".to_string(),
            message_id: "msg789".to_string(),
            metadata: EventMetadata {
                guild_id: Some("guild1".to_string()),
                thread_id: None,
                reply_to: None,
            },
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"adapter\":\"discord\""));
        assert!(json.contains("\"event_type\":\"message\""));
        assert!(json.contains("\"guild_id\":\"guild1\""));
        // thread_id should be skipped (None)
        assert!(!json.contains("thread_id"));
    }

    #[test]
    fn test_gateway_client_creation() {
        let client = GatewayClient::new("http://localhost:3000".to_string());
        assert_eq!(client.base_url, "http://localhost:3000");
    }
}
```

- [ ] **Step 2: Update lib.rs**

Update `crates/river-discord/src/lib.rs`:

```rust
//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod channels;
pub mod config;
pub mod gateway;

pub use channels::ChannelState;
pub use config::{Args, DiscordConfig};
pub use gateway::{GatewayClient, IncomingEvent, Author, EventMetadata};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-discord gateway`
Expected: 2 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-discord/src/
git commit -m "feat(discord): add gateway HTTP client"
```

---

### Task 5: Create Discord event handler

**Files:**
- Create: `crates/river-discord/src/handler.rs`
- Modify: `crates/river-discord/src/lib.rs`

- [ ] **Step 1: Create event handler module**

Create `crates/river-discord/src/handler.rs`:

```rust
//! Discord event handling

use crate::channels::ChannelState;
use crate::gateway::{Author, EventMetadata, GatewayClient, IncomingEvent};
use std::sync::Arc;
use twilight_model::gateway::payload::incoming::{MessageCreate, ReactionAdd};

/// Handles Discord events and forwards to gateway
pub struct EventHandler {
    channels: Arc<ChannelState>,
    gateway: Arc<GatewayClient>,
}

impl EventHandler {
    /// Create a new event handler
    pub fn new(channels: Arc<ChannelState>, gateway: Arc<GatewayClient>) -> Self {
        Self { channels, gateway }
    }

    /// Handle a message create event
    pub async fn handle_message(&self, msg: Box<MessageCreate>) {
        // Ignore bot messages
        if msg.author.bot {
            return;
        }

        let channel_id = msg.channel_id.get();

        // Check if we're listening to this channel
        if !self.channels.contains(channel_id).await {
            return;
        }

        // Build the event
        let event = IncomingEvent {
            adapter: "discord",
            event_type: "message".to_string(),
            channel: channel_id.to_string(),
            author: Author {
                id: msg.author.id.get().to_string(),
                name: msg.author.name.clone(),
            },
            content: msg.content.clone(),
            message_id: msg.id.get().to_string(),
            metadata: EventMetadata {
                guild_id: msg.guild_id.map(|id| id.get().to_string()),
                thread_id: None, // TODO: detect thread context
                reply_to: msg.referenced_message.as_ref().map(|m| m.id.get().to_string()),
            },
        };

        // Send to gateway
        if let Err(e) = self.gateway.send_incoming(event).await {
            tracing::error!("Failed to forward message to gateway: {}", e);
        } else {
            tracing::info!("Forwarded message to gateway");
        }
    }

    /// Handle a reaction add event
    pub async fn handle_reaction(&self, reaction: Box<ReactionAdd>) {
        let channel_id = reaction.channel_id.get();

        // Check if we're listening to this channel
        if !self.channels.contains(channel_id).await {
            return;
        }

        // Get user info
        let (user_id, user_name) = match &reaction.member {
            Some(member) => {
                let name = member.nick.as_ref()
                    .or(member.user.as_ref().map(|u| &u.name))
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string());
                let id = member.user.as_ref()
                    .map(|u| u.id.get().to_string())
                    .unwrap_or_else(|| reaction.user_id.get().to_string());
                (id, name)
            }
            None => (reaction.user_id.get().to_string(), "Unknown".to_string()),
        };

        // Get emoji string
        let emoji = match &reaction.emoji {
            twilight_model::channel::message::ReactionType::Custom { id, name, .. } => {
                name.clone().unwrap_or_else(|| format!("<:emoji:{}>", id))
            }
            twilight_model::channel::message::ReactionType::Unicode { name } => name.clone(),
        };

        let event = IncomingEvent {
            adapter: "discord",
            event_type: "reaction_add".to_string(),
            channel: channel_id.to_string(),
            author: Author {
                id: user_id,
                name: user_name,
            },
            content: emoji,
            message_id: reaction.message_id.get().to_string(),
            metadata: EventMetadata {
                guild_id: reaction.guild_id.map(|id| id.get().to_string()),
                thread_id: None,
                reply_to: None,
            },
        };

        if let Err(e) = self.gateway.send_incoming(event).await {
            tracing::error!("Failed to forward reaction to gateway: {}", e);
        } else {
            tracing::info!("Forwarded reaction to gateway");
        }
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require mocking Twilight types
    // These are covered by manual testing with live Discord
}
```

- [ ] **Step 2: Update lib.rs**

Update `crates/river-discord/src/lib.rs`:

```rust
//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod channels;
pub mod config;
pub mod gateway;
pub mod handler;

pub use channels::ChannelState;
pub use config::{Args, DiscordConfig};
pub use gateway::{Author, EventMetadata, GatewayClient, IncomingEvent};
pub use handler::EventHandler;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build -p river-discord`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/river-discord/src/
git commit -m "feat(discord): add event handler for messages and reactions"
```

---

## Chunk 3: Outbound HTTP Server

### Task 6: Create outbound message types

**Files:**
- Create: `crates/river-discord/src/outbound.rs`
- Modify: `crates/river-discord/src/lib.rs`

- [ ] **Step 1: Create outbound types and validation**

Create `crates/river-discord/src/outbound.rs`:

```rust
//! HTTP server for outbound messages and admin API

use axum::{
    extract::State,
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::channels::ChannelState;

/// Send message request from gateway
#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub channel: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub create_thread: Option<String>,
    #[serde(default)]
    pub reaction: Option<String>,
}

impl SendRequest {
    /// Validate the request
    pub fn validate(&self) -> Result<(), &'static str> {
        // Must have content or reaction
        if self.content.is_none() && self.reaction.is_none() {
            return Err("must provide content or reaction");
        }

        // content and reaction are mutually exclusive
        if self.content.is_some() && self.reaction.is_some() {
            return Err("content and reaction are mutually exclusive");
        }

        // reply_to and create_thread are mutually exclusive
        if self.reply_to.is_some() && self.create_thread.is_some() {
            return Err("reply_to and create_thread are mutually exclusive");
        }

        Ok(())
    }
}

/// Send message response
#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Add channel request
#[derive(Debug, Deserialize)]
pub struct AddChannelRequest {
    pub channel_id: String,
}

/// Channel operation response
#[derive(Debug, Serialize)]
pub struct ChannelResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// List channels response
#[derive(Debug, Serialize)]
pub struct ListChannelsResponse {
    pub channels: Vec<String>,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub discord: &'static str,
    pub gateway: &'static str,
    pub channel_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_request_validation_valid_content() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_send_request_validation_valid_reaction() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: None,
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: Some("👍".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_send_request_validation_no_content_or_reaction() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: None,
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: None,
        };
        assert_eq!(req.validate().unwrap_err(), "must provide content or reaction");
    }

    #[test]
    fn test_send_request_validation_both_content_and_reaction() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: Some("👍".to_string()),
        };
        assert_eq!(req.validate().unwrap_err(), "content and reaction are mutually exclusive");
    }

    #[test]
    fn test_send_request_validation_reply_and_thread() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: Some("msg1".to_string()),
            thread_id: None,
            create_thread: Some("New Thread".to_string()),
            reaction: None,
        };
        assert_eq!(req.validate().unwrap_err(), "reply_to and create_thread are mutually exclusive");
    }
}
```

- [ ] **Step 2: Update lib.rs**

Update `crates/river-discord/src/lib.rs`:

```rust
//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod channels;
pub mod config;
pub mod gateway;
pub mod handler;
pub mod outbound;

pub use channels::ChannelState;
pub use config::{Args, DiscordConfig};
pub use gateway::{Author, EventMetadata, GatewayClient, IncomingEvent};
pub use handler::EventHandler;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-discord outbound`
Expected: 5 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-discord/src/
git commit -m "feat(discord): add outbound message types with validation"
```

---

### Task 7: Create HTTP server routes

**Files:**
- Modify: `crates/river-discord/src/outbound.rs`

- [ ] **Step 1: Add shared state and route handlers**

Add to `crates/river-discord/src/outbound.rs` (before the tests module):

```rust
/// Shared application state for HTTP server
pub struct AppState {
    pub channels: Arc<ChannelState>,
    pub discord_connected: std::sync::atomic::AtomicBool,
    pub gateway_reachable: std::sync::atomic::AtomicBool,
}

impl AppState {
    pub fn new(channels: Arc<ChannelState>) -> Arc<Self> {
        Arc::new(Self {
            channels,
            discord_connected: std::sync::atomic::AtomicBool::new(false),
            gateway_reachable: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn set_discord_connected(&self, connected: bool) {
        self.discord_connected.store(connected, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_gateway_reachable(&self, reachable: bool) {
        self.gateway_reachable.store(reachable, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Create the HTTP router
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/send", post(handle_send))
        .route("/channels", get(list_channels))
        .route("/channels", post(add_channel))
        .route("/channels/{id}", delete(remove_channel))
        .with_state(state)
}

async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let discord = if state.discord_connected.load(std::sync::atomic::Ordering::Relaxed) {
        "connected"
    } else {
        "disconnected"
    };
    let gateway = if state.gateway_reachable.load(std::sync::atomic::Ordering::Relaxed) {
        "reachable"
    } else {
        "unreachable"
    };

    Json(HealthResponse {
        status: "ok",
        discord,
        gateway,
        channel_count: state.channels.count().await,
    })
}

async fn handle_send(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<SendRequest>,
) -> Result<Json<SendResponse>, (StatusCode, Json<SendResponse>)> {
    // Validate request
    if let Err(e) = req.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some(format!("validation error: {}", e)),
            }),
        ));
    }

    // TODO: Actually send to Discord via Twilight HTTP client
    // For now, return a placeholder response
    Ok(Json(SendResponse {
        success: true,
        message_id: Some("placeholder".to_string()),
        error: None,
    }))
}

async fn list_channels(State(state): State<Arc<AppState>>) -> Json<ListChannelsResponse> {
    let channels = state.channels.list().await;
    Json(ListChannelsResponse {
        channels: channels.into_iter().map(|c| c.to_string()).collect(),
    })
}

async fn add_channel(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddChannelRequest>,
) -> Result<Json<ChannelResponse>, (StatusCode, Json<ChannelResponse>)> {
    let channel_id: u64 = req.channel_id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ChannelResponse {
                success: false,
                error: Some("invalid channel id".to_string()),
            }),
        )
    })?;

    state.channels.add(channel_id).await;
    tracing::info!("Channel added");

    Ok(Json(ChannelResponse {
        success: true,
        error: None,
    }))
}

async fn remove_channel(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ChannelResponse>, (StatusCode, Json<ChannelResponse>)> {
    let channel_id: u64 = id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ChannelResponse {
                success: false,
                error: Some("invalid channel id".to_string()),
            }),
        )
    })?;

    let removed = state.channels.remove(channel_id).await;
    if removed {
        tracing::info!("Channel removed");
        Ok(Json(ChannelResponse {
            success: true,
            error: None,
        }))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ChannelResponse {
                success: false,
                error: Some("channel not in listen set".to_string()),
            }),
        ))
    }
}
```

- [ ] **Step 2: Add router tests**

Add to the tests module in `crates/river-discord/src/outbound.rs`:

```rust
    #[tokio::test]
    async fn test_health_check() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![1, 2, 3], None);
        let state = AppState::new(channels);
        state.set_discord_connected(true);
        state.set_gateway_reachable(true);

        let app = create_router(state);
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_channels() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![111, 222], None);
        let state = AppState::new(channels);

        let app = create_router(state);
        let response = app
            .oneshot(Request::builder().uri("/channels").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_add_channel() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![], None);
        let state = AppState::new(channels.clone());

        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"channel_id": "999"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(channels.contains(999).await);
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-discord outbound`
Expected: 8 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-discord/
git commit -m "feat(discord): add HTTP server routes for outbound and admin API"
```

---

## Chunk 4: Discord Client and Slash Commands

### Task 8: Create Discord client wrapper

**Files:**
- Create: `crates/river-discord/src/client.rs`
- Modify: `crates/river-discord/src/lib.rs`

- [ ] **Step 1: Create Discord client module**

Create `crates/river-discord/src/client.rs`:

```rust
//! Twilight Discord client wrapper

use std::sync::Arc;
use twilight_gateway::{Event, Intents, Shard, ShardId};
use twilight_http::Client as HttpClient;
use twilight_model::id::{marker::GuildMarker, Id};

/// Discord client wrapping Twilight components
pub struct DiscordClient {
    pub http: Arc<HttpClient>,
    pub shard: Shard,
    pub guild_id: Id<GuildMarker>,
}

impl DiscordClient {
    /// Create a new Discord client
    pub async fn new(token: &str, guild_id: u64) -> anyhow::Result<Self> {
        let http = Arc::new(HttpClient::new(token.to_string()));

        let intents = Intents::GUILDS
            | Intents::GUILD_MESSAGES
            | Intents::GUILD_MESSAGE_REACTIONS
            | Intents::MESSAGE_CONTENT
            | Intents::DIRECT_MESSAGES;

        let shard = Shard::new(ShardId::ONE, token.to_string(), intents);

        Ok(Self {
            http,
            shard,
            guild_id: Id::new(guild_id),
        })
    }

    /// Receive the next event from Discord
    pub async fn next_event(&mut self) -> Option<Event> {
        self.shard.next_event().await.ok()
    }

    /// Send a message to a channel
    pub async fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        reply_to: Option<u64>,
    ) -> anyhow::Result<u64> {
        use twilight_model::id::{marker::ChannelMarker, marker::MessageMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);

        let mut request = self.http.create_message(channel).content(content)?;

        if let Some(msg_id) = reply_to {
            let msg: Id<MessageMarker> = Id::new(msg_id);
            request = request.reply(msg);
        }

        let response = request.await?;
        let message = response.model().await?;

        Ok(message.id.get())
    }

    /// Add a reaction to a message
    pub async fn add_reaction(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> anyhow::Result<()> {
        use twilight_http::request::channel::reaction::RequestReactionType;
        use twilight_model::id::{marker::ChannelMarker, marker::MessageMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);
        let message: Id<MessageMarker> = Id::new(message_id);

        let reaction = RequestReactionType::Unicode { name: emoji };

        self.http
            .create_reaction(channel, message, &reaction)
            .await?;

        Ok(())
    }

    /// Check if connected to Discord
    pub fn is_connected(&self) -> bool {
        // Shard status check
        self.shard.status().is_connected()
    }
}
```

- [ ] **Step 2: Update lib.rs**

Update `crates/river-discord/src/lib.rs`:

```rust
//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod channels;
pub mod client;
pub mod config;
pub mod gateway;
pub mod handler;
pub mod outbound;

pub use channels::ChannelState;
pub use client::DiscordClient;
pub use config::{Args, DiscordConfig};
pub use gateway::{Author, EventMetadata, GatewayClient, IncomingEvent};
pub use handler::EventHandler;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build -p river-discord`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/river-discord/src/
git commit -m "feat(discord): add Twilight Discord client wrapper"
```

---

### Task 9: Create slash commands

**Files:**
- Create: `crates/river-discord/src/commands.rs`
- Modify: `crates/river-discord/src/lib.rs`

- [ ] **Step 1: Create slash commands module**

Create `crates/river-discord/src/commands.rs`:

```rust
//! Slash command registration and handling

use crate::channels::ChannelState;
use std::sync::Arc;
use twilight_http::Client as HttpClient;
use twilight_model::application::command::{Command, CommandType};
use twilight_model::application::interaction::application_command::CommandOptionValue;
use twilight_model::application::interaction::Interaction;
use twilight_model::channel::message::MessageFlags;
use twilight_model::http::interaction::{InteractionResponse, InteractionResponseType, InteractionResponseData};
use twilight_model::id::{marker::GuildMarker, Id};
use twilight_util::builder::command::{CommandBuilder, ChannelBuilder};

/// Register slash commands for a guild
pub async fn register_commands(
    http: &HttpClient,
    application_id: Id<twilight_model::id::marker::ApplicationMarker>,
    guild_id: Id<GuildMarker>,
) -> anyhow::Result<()> {
    let commands = vec![
        CommandBuilder::new("listen", "Add a channel to the listen set", CommandType::ChatInput)
            .option(ChannelBuilder::new("channel", "The channel to listen to").required(true))
            .default_member_permissions(twilight_model::guild::Permissions::MANAGE_CHANNELS)
            .build(),
        CommandBuilder::new("unlisten", "Remove a channel from the listen set", CommandType::ChatInput)
            .option(ChannelBuilder::new("channel", "The channel to stop listening to").required(true))
            .default_member_permissions(twilight_model::guild::Permissions::MANAGE_CHANNELS)
            .build(),
        CommandBuilder::new("channels", "List all channels being listened to", CommandType::ChatInput)
            .default_member_permissions(twilight_model::guild::Permissions::MANAGE_CHANNELS)
            .build(),
    ];

    http.set_guild_commands(application_id, guild_id, &commands)
        .await?;

    tracing::info!("Registered slash commands");
    Ok(())
}

/// Handle an interaction
pub async fn handle_interaction(
    http: &HttpClient,
    interaction: Interaction,
    channels: Arc<ChannelState>,
) -> anyhow::Result<()> {
    let Interaction::ApplicationCommand(command) = interaction else {
        return Ok(());
    };

    let response_content = match command.data.name.as_str() {
        "listen" => {
            let channel_id = command.data.options.iter()
                .find(|o| o.name == "channel")
                .and_then(|o| match &o.value {
                    CommandOptionValue::Channel(id) => Some(id.get()),
                    _ => None,
                });

            if let Some(id) = channel_id {
                channels.add(id).await;
                format!("Now listening to <#{}>", id)
            } else {
                "Invalid channel".to_string()
            }
        }
        "unlisten" => {
            let channel_id = command.data.options.iter()
                .find(|o| o.name == "channel")
                .and_then(|o| match &o.value {
                    CommandOptionValue::Channel(id) => Some(id.get()),
                    _ => None,
                });

            if let Some(id) = channel_id {
                if channels.remove(id).await {
                    format!("Stopped listening to <#{}>", id)
                } else {
                    format!("<#{}> was not in the listen set", id)
                }
            } else {
                "Invalid channel".to_string()
            }
        }
        "channels" => {
            let list = channels.list().await;
            if list.is_empty() {
                "Not listening to any channels".to_string()
            } else {
                let channel_mentions: Vec<String> = list.iter().map(|id| format!("<#{}>", id)).collect();
                format!("Listening to: {}", channel_mentions.join(", "))
            }
        }
        _ => "Unknown command".to_string(),
    };

    // Send ephemeral response
    let response = InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(InteractionResponseData {
            content: Some(response_content),
            flags: Some(MessageFlags::EPHEMERAL),
            ..Default::default()
        }),
    };

    http.interaction(command.application_id)
        .create_response(command.id, &command.token, &response)
        .await?;

    Ok(())
}
```

- [ ] **Step 2: Update lib.rs**

Update `crates/river-discord/src/lib.rs`:

```rust
//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod channels;
pub mod client;
pub mod commands;
pub mod config;
pub mod gateway;
pub mod handler;
pub mod outbound;

pub use channels::ChannelState;
pub use client::DiscordClient;
pub use config::{Args, DiscordConfig};
pub use gateway::{Author, EventMetadata, GatewayClient, IncomingEvent};
pub use handler::EventHandler;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build -p river-discord`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/river-discord/
git commit -m "feat(discord): add slash command registration and handling"
```

---

## Chunk 5: Main Entry Point and Integration

### Task 10: Create main entry point

**Files:**
- Modify: `crates/river-discord/src/main.rs`

- [ ] **Step 1: Implement main.rs**

Replace `crates/river-discord/src/main.rs`:

```rust
use clap::Parser;
use river_discord::{
    channels::ChannelState,
    client::DiscordClient,
    commands::{handle_interaction, register_commands},
    config::{Args, DiscordConfig},
    gateway::GatewayClient,
    handler::EventHandler,
    outbound::{create_router, AppState},
};
use std::net::SocketAddr;
use std::sync::Arc;
use twilight_gateway::Event;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let config = DiscordConfig::from_args(args)?;

    tracing::info!("Starting River Discord Adapter");

    // Load channel state
    let channels = ChannelState::load(
        config.initial_channels.clone(),
        config.state_file.clone(),
    ).await;
    tracing::info!("Loaded channel state");

    // Create gateway client
    let gateway_client = Arc::new(GatewayClient::new(config.gateway_url.clone()));

    // Create Discord client
    let mut discord = DiscordClient::new(&config.token, config.guild_id).await?;
    tracing::info!("Connected to Discord");

    // Get application ID for slash commands
    let app_info = discord.http.current_user_application().await?.model().await?;
    let application_id = app_info.id;

    // Register slash commands
    register_commands(&discord.http, application_id, discord.guild_id).await?;

    // Create event handler
    let event_handler = EventHandler::new(channels.clone(), gateway_client.clone());

    // Create HTTP server state
    let http_state = AppState::new(channels.clone());

    // Spawn HTTP server
    let http_state_clone = http_state.clone();
    let listen_port = config.listen_port;
    tokio::spawn(async move {
        let app = create_router(http_state_clone);
        let addr = SocketAddr::from(([127, 0, 0, 1], listen_port));
        tracing::info!("HTTP server listening on {}", addr);
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    // Spawn gateway health check loop
    let gateway_for_health = gateway_client.clone();
    let state_for_health = http_state.clone();
    tokio::spawn(async move {
        loop {
            let reachable = gateway_for_health.health_check().await;
            state_for_health.set_gateway_reachable(reachable);
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });

    // Main event loop
    tracing::info!("Entering event loop");
    loop {
        // Update connection status
        http_state.set_discord_connected(discord.is_connected());

        // Wait for next event
        let Some(event) = discord.next_event().await else {
            tracing::warn!("Discord connection closed, attempting reconnect...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        };

        match event {
            Event::Ready(_) => {
                tracing::info!("Discord ready");
                http_state.set_discord_connected(true);
            }
            Event::MessageCreate(msg) => {
                event_handler.handle_message(msg).await;
            }
            Event::ReactionAdd(reaction) => {
                event_handler.handle_reaction(reaction).await;
            }
            Event::InteractionCreate(interaction) => {
                let http = discord.http.clone();
                let channels = channels.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_interaction(&http, interaction.0, channels).await {
                        tracing::error!("Failed to handle interaction: {}", e);
                    }
                });
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 2: Build and verify**

Run: `cargo build -p river-discord`
Expected: Compiles successfully

- [ ] **Step 3: Check CLI help**

Run: `cargo run -p river-discord -- --help`
Expected: Shows all CLI flags

- [ ] **Step 4: Commit**

```bash
git add crates/river-discord/src/main.rs
git commit -m "feat(discord): add main entry point with event loop"
```

---

### Task 11: Implement outbound Discord sending

**Files:**
- Modify: `crates/river-discord/src/outbound.rs`
- Modify: `crates/river-discord/src/client.rs`

- [ ] **Step 1: Add DiscordClient to AppState**

Update `AppState` in `crates/river-discord/src/outbound.rs`:

```rust
use crate::client::DiscordClient;
use tokio::sync::RwLock;

/// Shared application state for HTTP server
pub struct AppState {
    pub channels: Arc<ChannelState>,
    pub discord: Arc<RwLock<Option<Arc<DiscordClient>>>>,
    pub discord_connected: std::sync::atomic::AtomicBool,
    pub gateway_reachable: std::sync::atomic::AtomicBool,
}

impl AppState {
    pub fn new(channels: Arc<ChannelState>) -> Arc<Self> {
        Arc::new(Self {
            channels,
            discord: Arc::new(RwLock::new(None)),
            discord_connected: std::sync::atomic::AtomicBool::new(false),
            gateway_reachable: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn set_discord(&self, client: Arc<DiscordClient>) {
        // Note: This is a sync wrapper for simplicity
        let discord = self.discord.clone();
        tokio::spawn(async move {
            *discord.write().await = Some(client);
        });
    }

    pub fn set_discord_connected(&self, connected: bool) {
        self.discord_connected.store(connected, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_gateway_reachable(&self, reachable: bool) {
        self.gateway_reachable.store(reachable, std::sync::atomic::Ordering::Relaxed);
    }
}
```

- [ ] **Step 2: Update handle_send to actually send to Discord**

Update `handle_send` in `crates/river-discord/src/outbound.rs`:

```rust
async fn handle_send(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendRequest>,
) -> Result<Json<SendResponse>, (StatusCode, Json<SendResponse>)> {
    // Validate request
    if let Err(e) = req.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some(format!("validation error: {}", e)),
            }),
        ));
    }

    // Get Discord client
    let discord_guard = state.discord.read().await;
    let Some(ref discord) = *discord_guard else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some("discord client not initialized".to_string()),
            }),
        ));
    };

    // Parse channel ID
    let channel_id: u64 = req.channel.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some("invalid channel id".to_string()),
            }),
        )
    })?;

    // Handle reaction
    if let Some(emoji) = &req.reaction {
        let message_id: u64 = req.reply_to.as_ref()
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(SendResponse {
                        success: false,
                        message_id: None,
                        error: Some("reply_to required for reactions".to_string()),
                    }),
                )
            })?
            .parse()
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(SendResponse {
                        success: false,
                        message_id: None,
                        error: Some("invalid message id".to_string()),
                    }),
                )
            })?;

        discord.add_reaction(channel_id, message_id, emoji).await.map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(SendResponse {
                    success: false,
                    message_id: None,
                    error: Some(format!("discord api error: {}", e)),
                }),
            )
        })?;

        return Ok(Json(SendResponse {
            success: true,
            message_id: None,
            error: None,
        }));
    }

    // Handle message
    let content = req.content.as_ref().unwrap();
    let reply_to = req.reply_to.as_ref().and_then(|s| s.parse().ok());

    let message_id = discord.send_message(channel_id, content, reply_to).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some(format!("discord api error: {}", e)),
            }),
        )
    })?;

    tracing::info!("Sent message to Discord");

    Ok(Json(SendResponse {
        success: true,
        message_id: Some(message_id.to_string()),
        error: None,
    }))
}
```

- [ ] **Step 3: Update main.rs to set Discord client on AppState**

Add after creating the Discord client in `main.rs`:

```rust
    // Set Discord client on HTTP state (for outbound messages)
    http_state.set_discord(Arc::new(discord));
```

Note: This requires refactoring `discord` to be `Arc<DiscordClient>` and making the event loop work with a cloned Arc. The full refactor:

```rust
    // Create Discord client
    let discord = Arc::new(DiscordClient::new(&config.token, config.guild_id).await?);
    tracing::info!("Connected to Discord");

    // ... rest of setup ...

    // Set Discord client on HTTP state (for outbound messages)
    {
        let mut discord_ref = http_state.discord.write().await;
        *discord_ref = Some(discord.clone());
    }

    // Main event loop needs mutable access to shard
    // This requires redesigning - the shard needs to be separate from the http client
```

Actually, the design needs adjustment. The `Shard` requires mutable access for `next_event()`, so it can't be shared via Arc. Let's restructure:

- [ ] **Step 3 (revised): Restructure DiscordClient for shared HTTP access**

Update `crates/river-discord/src/client.rs`:

```rust
//! Twilight Discord client wrapper

use std::sync::Arc;
use twilight_gateway::{Event, Intents, Shard, ShardId};
use twilight_http::Client as HttpClient;
use twilight_model::id::{marker::GuildMarker, Id};

/// Shared Discord HTTP client (for sending messages)
#[derive(Clone)]
pub struct DiscordSender {
    pub http: Arc<HttpClient>,
    pub guild_id: Id<GuildMarker>,
}

impl DiscordSender {
    /// Send a message to a channel
    pub async fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        reply_to: Option<u64>,
    ) -> anyhow::Result<u64> {
        use twilight_model::id::{marker::ChannelMarker, marker::MessageMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);

        let mut request = self.http.create_message(channel).content(content)?;

        if let Some(msg_id) = reply_to {
            let msg: Id<MessageMarker> = Id::new(msg_id);
            request = request.reply(msg);
        }

        let response = request.await?;
        let message = response.model().await?;

        Ok(message.id.get())
    }

    /// Add a reaction to a message
    pub async fn add_reaction(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> anyhow::Result<()> {
        use twilight_http::request::channel::reaction::RequestReactionType;
        use twilight_model::id::{marker::ChannelMarker, marker::MessageMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);
        let message: Id<MessageMarker> = Id::new(message_id);

        let reaction = RequestReactionType::Unicode { name: emoji };

        self.http
            .create_reaction(channel, message, &reaction)
            .await?;

        Ok(())
    }
}

/// Discord client with gateway shard (for receiving events)
pub struct DiscordClient {
    pub sender: DiscordSender,
    pub shard: Shard,
}

impl DiscordClient {
    /// Create a new Discord client
    pub async fn new(token: &str, guild_id: u64) -> anyhow::Result<Self> {
        let http = Arc::new(HttpClient::new(token.to_string()));

        let intents = Intents::GUILDS
            | Intents::GUILD_MESSAGES
            | Intents::GUILD_MESSAGE_REACTIONS
            | Intents::MESSAGE_CONTENT
            | Intents::DIRECT_MESSAGES;

        let shard = Shard::new(ShardId::ONE, token.to_string(), intents);

        Ok(Self {
            sender: DiscordSender {
                http,
                guild_id: Id::new(guild_id),
            },
            shard,
        })
    }

    /// Get the sender (can be cloned and shared)
    pub fn sender(&self) -> DiscordSender {
        self.sender.clone()
    }

    /// Receive the next event from Discord
    pub async fn next_event(&mut self) -> Option<Event> {
        self.shard.next_event().await.ok()
    }

    /// Check if connected to Discord
    pub fn is_connected(&self) -> bool {
        self.shard.status().is_connected()
    }
}
```

- [ ] **Step 4: Update outbound.rs to use DiscordSender**

Update `AppState` in `crates/river-discord/src/outbound.rs`:

```rust
use crate::client::DiscordSender;

/// Shared application state for HTTP server
pub struct AppState {
    pub channels: Arc<ChannelState>,
    pub discord: Arc<RwLock<Option<DiscordSender>>>,
    pub discord_connected: std::sync::atomic::AtomicBool,
    pub gateway_reachable: std::sync::atomic::AtomicBool,
}

impl AppState {
    pub fn new(channels: Arc<ChannelState>) -> Arc<Self> {
        Arc::new(Self {
            channels,
            discord: Arc::new(RwLock::new(None)),
            discord_connected: std::sync::atomic::AtomicBool::new(false),
            gateway_reachable: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub async fn set_discord(&self, sender: DiscordSender) {
        *self.discord.write().await = Some(sender);
    }

    // ... rest unchanged
}
```

- [ ] **Step 5: Update main.rs with new structure**

Update main.rs to use `DiscordSender`:

```rust
    // Set Discord sender on HTTP state (for outbound messages)
    http_state.set_discord(discord.sender()).await;
```

- [ ] **Step 6: Update lib.rs exports**

```rust
pub use client::{DiscordClient, DiscordSender};
```

- [ ] **Step 7: Build and verify**

Run: `cargo build -p river-discord`
Expected: Compiles successfully

- [ ] **Step 8: Commit**

```bash
git add crates/river-discord/src/
git commit -m "feat(discord): implement outbound message sending to Discord"
```

---

### Task 12: Run all tests and update STATUS.md

**Files:**
- Modify: `docs/superpowers/STATUS.md`

- [ ] **Step 1: Run all tests**

Run: `cargo test`
Expected: All tests pass, note the count

- [ ] **Step 2: Build release binary**

Run: `cargo build --release -p river-discord`
Expected: Build succeeds

- [ ] **Step 3: Verify CLI help**

Run: `./target/release/river-discord --help`
Expected: Shows all flags

- [ ] **Step 4: Update STATUS.md**

Add to `docs/superpowers/STATUS.md` under Completed:

```markdown
### Plan 6: Discord Adapter ✅
- Twilight-based Discord adapter
- Channel management via slash commands and admin API
- Message and reaction forwarding to gateway
- Outbound message sending from agent
- Dynamic channel add/remove at runtime
- State persistence to file
- XX tests passing (update with actual count)
- Binary: `river-discord --token-file /path --gateway-url http://localhost:3000 --guild-id 123`
```

Update "Next Up" section.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/STATUS.md
git commit -m "docs: update STATUS.md with Plan 6 completion"
```

---

## Summary

This plan implements the Discord adapter in 12 tasks across 5 chunks:

1. **Chunk 1: Project Setup** (Tasks 1-3)
   - Create crate, dependencies, config, channel state

2. **Chunk 2: Gateway Client** (Tasks 4-5)
   - HTTP client for river-gateway, event handler

3. **Chunk 3: Outbound Server** (Tasks 6-7)
   - HTTP server types, routes, admin API

4. **Chunk 4: Discord Client** (Tasks 8-9)
   - Twilight client wrapper, slash commands

5. **Chunk 5: Integration** (Tasks 10-12)
   - Main entry point, outbound sending, final tests

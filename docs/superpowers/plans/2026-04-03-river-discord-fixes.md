# river-discord Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix critical gaps in river-discord including Adapter trait implementation, reconnection handling, test coverage, and event forwarding latency.

**Architecture:** The Discord adapter uses twilight for gateway/HTTP communication and exposes an HTTP API for worker integration. Events flow from Discord gateway through a channel to the worker endpoint. The adapter must implement the `Adapter` trait from river-adapter and handle reconnection gracefully.

**Tech Stack:** Rust, twilight-gateway, twilight-http, twilight-model, axum, tokio, async_trait, reqwest

---

## File Structure

```
crates/river-discord/
  src/
    discord.rs      # DiscordClient + Adapter trait impl (MODIFY)
    http.rs         # HTTP endpoints (MODIFY)
    main.rs         # Entry point, event loop (MODIFY)
  tests/
    event_conversion.rs  # Event conversion tests (CREATE)
    emoji.rs             # Emoji parsing tests (CREATE)
    mod.rs               # Test module (CREATE)
  Cargo.toml        # Add async-trait, dev deps (MODIFY)
```

---

## Task 1: Add async-trait dependency

Add the async-trait crate needed for implementing the Adapter trait.

- [ ] **Step 1.1:** Update Cargo.toml to add async-trait dependency

**File:** `/home/cassie/river-engine/crates/river-discord/Cargo.toml`

Add after line 24 (after `chrono = { workspace = true }`):
```toml
async-trait = { workspace = true }
```

- [ ] **Step 1.2:** Verify the dependency is in workspace Cargo.toml

```bash
grep -q "async-trait" /home/cassie/river-engine/Cargo.toml && echo "OK" || echo "Need to add to workspace"
```

- [ ] **Step 1.3:** Run cargo check to verify

```bash
cd /home/cassie/river-engine && cargo check -p river-discord
```

**Commit:** `feat(river-discord): add async-trait dependency for Adapter trait`

---

## Task 2: Implement Adapter trait on DiscordClient

Implement the required `Adapter` trait from river-adapter on `DiscordClient`.

- [ ] **Step 2.1:** Add async_trait import and AdapterError to discord.rs

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Replace lines 1-16 with:
```rust
//! Discord gateway client using twilight.

use crate::DiscordConfig;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use river_adapter::{
    Adapter, AdapterError, Attachment, Author, ErrorCode, EventMetadata, FeatureId, InboundEvent,
    OutboundRequest, OutboundResponse, ResponseData, ResponseError,
};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use twilight_gateway::{Event, Intents, Shard, ShardId, StreamExt};
use twilight_http::Client as HttpClient;
use twilight_model::channel::message::{EmojiReactionType, MessageType};
use twilight_model::id::marker::{ChannelMarker, MessageMarker};
use twilight_model::id::Id;
```

- [ ] **Step 2.2:** Add adapter_name field to DiscordClient struct

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Replace the DiscordClient struct (lines 18-22) with:
```rust
/// Discord client wrapping twilight gateway and HTTP.
pub struct DiscordClient {
    http: Arc<HttpClient>,
    event_rx: Arc<RwLock<mpsc::Receiver<InboundEvent>>>,
    connected: Arc<RwLock<bool>>,
    adapter_name: String,
}
```

- [ ] **Step 2.3:** Update DiscordClient::new to accept adapter_name parameter

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Replace the `new` function signature and body (lines 24-80) with:
```rust
impl DiscordClient {
    /// Create a new Discord client.
    pub async fn new(
        config: DiscordConfig,
        adapter_name: String,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let intents = Intents::from_bits_truncate(
            config.intents.unwrap_or(
                Intents::GUILD_MESSAGES.bits()
                    | Intents::MESSAGE_CONTENT.bits()
                    | Intents::GUILD_MESSAGE_REACTIONS.bits()
                    | Intents::GUILD_MESSAGE_TYPING.bits()
                    | Intents::DIRECT_MESSAGES.bits(),
            ),
        );

        let mut shard = Shard::new(ShardId::ONE, config.token.clone(), intents);
        let http = Arc::new(HttpClient::new(config.token));
        let (event_tx, event_rx) = mpsc::channel::<InboundEvent>(256);
        let connected = Arc::new(RwLock::new(true));

        // Spawn gateway event loop
        let connected_clone = connected.clone();
        let adapter_name_clone = adapter_name.clone();

        tokio::spawn(async move {
            tracing::info!("Starting Discord gateway event loop");

            while let Some(event) = shard.next_event(twilight_gateway::EventTypeFlags::all()).await
            {
                match event {
                    Ok(event) => {
                        if let Some(inbound) = convert_event(&adapter_name_clone, event) {
                            if event_tx.send(inbound).await.is_err() {
                                tracing::warn!("Event channel closed");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Gateway error: {:?}", e);
                        // Mark as disconnected on error
                        let mut c = connected_clone.write().await;
                        *c = false;
                        break;
                    }
                }
            }

            tracing::info!("Gateway event loop ended");
        });

        Ok(Self {
            http,
            event_rx: Arc::new(RwLock::new(event_rx)),
            connected,
            adapter_name,
        })
    }
```

- [ ] **Step 2.4:** Rename existing execute method to execute_impl

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Change the method name from `pub async fn execute(` to `async fn execute_impl(` (around line 96).

- [ ] **Step 2.5:** Add Adapter trait implementation after DiscordClient impl block

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Add after the closing brace of `impl DiscordClient` (after `is_healthy` method, around line 341):
```rust

#[async_trait]
impl Adapter for DiscordClient {
    fn adapter_type(&self) -> &str {
        &self.adapter_name
    }

    fn features(&self) -> Vec<FeatureId> {
        supported_features()
    }

    async fn start(&self, _worker_endpoint: String) -> Result<(), AdapterError> {
        // Event forwarding is started in new(), this is a no-op
        // The worker_endpoint is provided during registration
        Ok(())
    }

    async fn execute(&self, request: OutboundRequest) -> Result<OutboundResponse, AdapterError> {
        Ok(self.execute_impl(request).await)
    }

    async fn health(&self) -> Result<(), AdapterError> {
        if self.is_healthy().await {
            Ok(())
        } else {
            Err(AdapterError::Connection("websocket disconnected".into()))
        }
    }
}
```

- [ ] **Step 2.6:** Update main.rs to pass adapter_name to DiscordClient::new

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Replace line 145:
```rust
    let discord = Arc::new(DiscordClient::new(discord_config.clone()).await?);
```
with:
```rust
    let discord = Arc::new(DiscordClient::new(discord_config.clone(), args.adapter_type.clone()).await?);
```

- [ ] **Step 2.7:** Run cargo check to verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-discord
```

**Commit:** `feat(river-discord): implement Adapter trait on DiscordClient`

---

## Task 3: Fix /start endpoint semantic conflict

The /start endpoint always fails because worker_endpoint is set during registration. Remove the endpoint since registration provides the worker endpoint.

- [ ] **Step 3.1:** Remove /start route from router

**File:** `/home/cassie/river-engine/crates/river-discord/src/http.rs`

Replace lines 16-22:
```rust
/// Create the HTTP router.
pub fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/start", post(start))
        .route("/execute", post(execute))
        .route("/health", get(health))
        .with_state(state)
}
```
with:
```rust
/// Create the HTTP router.
pub fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/execute", post(execute))
        .route("/health", get(health))
        .with_state(state)
}
```

- [ ] **Step 3.2:** Remove StartRequest, StartResponse structs and start handler

**File:** `/home/cassie/river-engine/crates/river-discord/src/http.rs`

Remove lines 24-60 (StartRequest struct through the start handler function).

- [ ] **Step 3.3:** Remove unused post import

**File:** `/home/cassie/river-engine/crates/river-discord/src/http.rs`

Replace line 8:
```rust
    routing::{get, post},
```
with:
```rust
    routing::{get, post},
```

Note: Keep `post` as it's still used by `/execute`.

- [ ] **Step 3.4:** Remove worker_endpoint from AdapterState since it's only used by /start

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Replace lines 49-62:
```rust
/// Shared adapter state (without the DiscordClient to maintain Sync).
pub struct AdapterState {
    pub worker_endpoint: Option<String>,
    pub config: Option<DiscordConfig>,
}

impl AdapterState {
    fn new() -> Self {
        Self {
            worker_endpoint: None,
            config: None,
        }
    }
}
```
with:
```rust
/// Shared adapter state (without the DiscordClient to maintain Sync).
pub struct AdapterState {
    pub config: Option<DiscordConfig>,
}

impl AdapterState {
    fn new() -> Self {
        Self {
            config: None,
        }
    }
}
```

- [ ] **Step 3.5:** Update state initialization to remove worker_endpoint

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Replace lines 138-142:
```rust
    {
        let mut s = state.write().await;
        s.config = Some(discord_config.clone());
        s.worker_endpoint = Some(registration.worker_endpoint.clone());
    }
```
with:
```rust
    {
        let mut s = state.write().await;
        s.config = Some(discord_config.clone());
    }
```

- [ ] **Step 3.6:** Run cargo check to verify

```bash
cd /home/cassie/river-engine && cargo check -p river-discord
```

**Commit:** `fix(river-discord): remove /start endpoint that always failed`

---

## Task 4: Handle MessageUpdate with None content

twilight's MessageUpdate has `content: Option<String>` but EventMetadata expects `String`. Handle the None case.

- [ ] **Step 4.1:** Update MessageUpdate event conversion to handle None content

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Find the MessageUpdate match arm (around lines 386-397) and replace:
```rust
        Event::MessageUpdate(msg) => Some(InboundEvent {
            adapter: adapter_name.into(),
            metadata: EventMetadata::MessageUpdate {
                channel: msg.channel_id.to_string(),
                message_id: msg.id.to_string(),
                content: msg.content.clone(),
                timestamp: msg
                    .edited_timestamp
                    .map(format_timestamp)
                    .unwrap_or_default(),
            },
        }),
```
with:
```rust
        Event::MessageUpdate(msg) => {
            // Skip if no content (partial updates without content)
            let content = match msg.content.clone() {
                Some(c) => c,
                None => return None,
            };
            Some(InboundEvent {
                adapter: adapter_name.into(),
                metadata: EventMetadata::MessageUpdate {
                    channel: msg.channel_id.to_string(),
                    message_id: msg.id.to_string(),
                    content,
                    timestamp: msg
                        .edited_timestamp
                        .map(format_timestamp)
                        .unwrap_or_default(),
                },
            })
        }
```

- [ ] **Step 4.2:** Run cargo check to verify

```bash
cd /home/cassie/river-engine && cargo check -p river-discord
```

**Commit:** `fix(river-discord): handle MessageUpdate with None content`

---

## Task 5: Add rate limit detection

Detect Discord 429 responses and return `AdapterError::RateLimited` with retry_after_ms.

- [ ] **Step 5.1:** Create helper function to check for rate limit errors

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Add after the `error_response` function (around line 516):
```rust

/// Check if error is a rate limit and extract retry_after if so.
fn check_rate_limit(err: &twilight_http::Error) -> Option<u64> {
    if let twilight_http::error::ErrorType::Response { status, .. } = err.kind() {
        if status.get() == 429 {
            // Default to 1 second if we can't parse retry_after
            // In practice, twilight handles rate limits internally,
            // but we expose this for the adapter protocol
            return Some(1000);
        }
    }
    None
}
```

- [ ] **Step 5.2:** Update SendMessage error handling to detect rate limits

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

In the `execute_impl` method, find the SendMessage error handling (around line 129):
```rust
                    Err(e) => error_response(ErrorCode::PlatformError, &e.to_string()),
```
Replace with:
```rust
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
```

- [ ] **Step 5.3:** Add rate limited error response helper

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Add after the `error_response` function:
```rust

/// Create a rate-limited error response.
fn error_response_rate_limited(retry_after_ms: u64) -> OutboundResponse {
    OutboundResponse {
        ok: false,
        data: None,
        error: Some(ResponseError {
            code: ErrorCode::RateLimited,
            message: format!("rate limited, retry after {}ms", retry_after_ms),
        }),
    }
}
```

- [ ] **Step 5.4:** Update all other API call error handlers similarly

Update error handling for EditMessage, DeleteMessage, AddReaction, RemoveReaction, TypingIndicator, and ReadHistory to use the same pattern:
```rust
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
```

- [ ] **Step 5.5:** Run cargo check to verify

```bash
cd /home/cassie/river-engine && cargo check -p river-discord
```

**Commit:** `feat(river-discord): detect rate limits and return proper error code`

---

## Task 6: Implement reconnection with exponential backoff

Add automatic reconnection when the gateway disconnects.

- [ ] **Step 6.1:** Refactor DiscordClient to support reconnection

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Replace the entire `new` method implementation with a version that includes reconnection logic:
```rust
impl DiscordClient {
    /// Create a new Discord client.
    pub async fn new(
        config: DiscordConfig,
        adapter_name: String,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let intents = Intents::from_bits_truncate(
            config.intents.unwrap_or(
                Intents::GUILD_MESSAGES.bits()
                    | Intents::MESSAGE_CONTENT.bits()
                    | Intents::GUILD_MESSAGE_REACTIONS.bits()
                    | Intents::GUILD_MESSAGE_TYPING.bits()
                    | Intents::DIRECT_MESSAGES.bits(),
            ),
        );

        let http = Arc::new(HttpClient::new(config.token.clone()));
        let (event_tx, event_rx) = mpsc::channel::<InboundEvent>(256);
        let connected = Arc::new(RwLock::new(true));

        // Spawn gateway event loop with reconnection
        let connected_clone = connected.clone();
        let adapter_name_clone = adapter_name.clone();
        let token = config.token.clone();

        tokio::spawn(async move {
            tracing::info!("Starting Discord gateway event loop");

            let mut backoff = std::time::Duration::from_secs(1);
            let max_backoff = std::time::Duration::from_secs(60);
            let mut disconnect_time: Option<std::time::Instant> = None;

            loop {
                // Create a new shard for each connection attempt
                let mut shard = Shard::new(ShardId::ONE, token.clone(), intents);

                // If we're reconnecting, emit ConnectionRestored
                if let Some(disconnected_at) = disconnect_time.take() {
                    let downtime_secs = disconnected_at.elapsed().as_secs();
                    let event = InboundEvent {
                        adapter: adapter_name_clone.clone(),
                        metadata: EventMetadata::ConnectionRestored {
                            downtime_seconds: downtime_secs,
                        },
                    };
                    if event_tx.send(event).await.is_err() {
                        tracing::warn!("Event channel closed during reconnect");
                        break;
                    }
                    tracing::info!("Gateway reconnected after {}s downtime", downtime_secs);
                }

                // Reset backoff on successful connection
                backoff = std::time::Duration::from_secs(1);
                {
                    let mut c = connected_clone.write().await;
                    *c = true;
                }

                // Process events until error
                while let Some(event) = shard.next_event(twilight_gateway::EventTypeFlags::all()).await {
                    match event {
                        Ok(event) => {
                            if let Some(inbound) = convert_event(&adapter_name_clone, event) {
                                if event_tx.send(inbound).await.is_err() {
                                    tracing::warn!("Event channel closed");
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Gateway error: {:?}", e);
                            break;
                        }
                    }
                }

                // Mark as disconnected
                {
                    let mut c = connected_clone.write().await;
                    *c = false;
                }

                // Record disconnect time for downtime calculation
                disconnect_time = Some(std::time::Instant::now());

                // Send ConnectionLost event
                let event = InboundEvent {
                    adapter: adapter_name_clone.clone(),
                    metadata: EventMetadata::ConnectionLost {
                        reason: "gateway disconnected".into(),
                        reconnecting: true,
                    },
                };
                if event_tx.send(event).await.is_err() {
                    tracing::warn!("Event channel closed");
                    break;
                }

                // Exponential backoff before reconnect
                tracing::info!("Reconnecting in {:?}", backoff);
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, max_backoff);
            }

            tracing::info!("Gateway event loop ended");
        });

        Ok(Self {
            http,
            event_rx: Arc::new(RwLock::new(event_rx)),
            connected,
            adapter_name,
        })
    }
```

- [ ] **Step 6.2:** Run cargo check to verify

```bash
cd /home/cassie/river-engine && cargo check -p river-discord
```

**Commit:** `feat(river-discord): add reconnection with exponential backoff`

---

## Task 7: Remove event polling latency

Forward events directly from the gateway event loop instead of queuing and polling every 100ms.

- [ ] **Step 7.1:** Update main.rs to use direct event forwarding

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Replace the event forwarding loop (lines 153-175):
```rust
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
```
with:
```rust
    // Start event forwarding loop (no polling delay)
    let discord_clone = discord.clone();
    let worker_endpoint = registration.worker_endpoint.clone();
    let event_task = tokio::spawn(async move {
        let http_client = reqwest::Client::new();
        loop {
            // Wait for events without polling delay
            if let Some(event) = discord_clone.recv_event().await {
                if let Err(e) = http_client
                    .post(format!("{}/notify", worker_endpoint))
                    .json(&event)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                {
                    tracing::warn!("Failed to forward event to worker: {}", e);
                }
            } else {
                // Channel closed, exit
                break;
            }
        }
    });
```

- [ ] **Step 7.2:** Add recv_event method to DiscordClient

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Replace the `poll_events` method (around lines 83-93):
```rust
    /// Poll for new events from the gateway.
    pub async fn poll_events(&self) -> Vec<InboundEvent> {
        let mut events = Vec::new();
        let mut rx = self.event_rx.write().await;

        // Drain available events without blocking
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        events
    }
```
with:
```rust
    /// Receive next event from the gateway (blocking).
    pub async fn recv_event(&self) -> Option<InboundEvent> {
        let mut rx = self.event_rx.write().await;
        rx.recv().await
    }

    /// Poll for new events from the gateway (non-blocking, for compatibility).
    #[allow(dead_code)]
    pub async fn poll_events(&self) -> Vec<InboundEvent> {
        let mut events = Vec::new();
        let mut rx = self.event_rx.write().await;

        // Drain available events without blocking
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        events
    }
```

- [ ] **Step 7.3:** Run cargo check to verify

```bash
cd /home/cassie/river-engine && cargo check -p river-discord
```

**Commit:** `perf(river-discord): remove 100ms polling latency in event forwarding`

---

## Task 8: Graceful shutdown instead of abort

Replace task abort with graceful shutdown using cancellation.

- [ ] **Step 8.1:** Add tokio_util dependency for CancellationToken

**File:** `/home/cassie/river-engine/crates/river-discord/Cargo.toml`

Add after the chrono line:
```toml
tokio-util = { version = "0.7", features = ["rt"] }
```

- [ ] **Step 8.2:** Update main.rs to use CancellationToken

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Add import at the top (after line 17):
```rust
use tokio_util::sync::CancellationToken;
```

- [ ] **Step 8.3:** Create cancellation token and pass to tasks

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Add after creating discord client (around line 145):
```rust
    // Create cancellation token for graceful shutdown
    let cancel_token = CancellationToken::new();
```

- [ ] **Step 8.4:** Update event forwarding loop to respect cancellation

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Update the event_task to use cancellation:
```rust
    // Start event forwarding loop (no polling delay)
    let discord_clone = discord.clone();
    let worker_endpoint = registration.worker_endpoint.clone();
    let cancel_clone = cancel_token.clone();
    let event_task = tokio::spawn(async move {
        let http_client = reqwest::Client::new();
        loop {
            tokio::select! {
                _ = cancel_clone.cancelled() => {
                    tracing::info!("Event forwarding task cancelled");
                    break;
                }
                event = discord_clone.recv_event() => {
                    if let Some(event) = event {
                        if let Err(e) = http_client
                            .post(format!("{}/notify", worker_endpoint))
                            .json(&event)
                            .timeout(Duration::from_secs(5))
                            .send()
                            .await
                        {
                            tracing::warn!("Failed to forward event to worker: {}", e);
                        }
                    } else {
                        // Channel closed, exit
                        break;
                    }
                }
            }
        }
    });
```

- [ ] **Step 8.5:** Update shutdown handling to use cancellation

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Replace the shutdown section (around lines 186-192):
```rust
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down Discord adapter");

    event_task.abort();
    server.abort();

    Ok(())
```
with:
```rust
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down Discord adapter gracefully");

    // Signal cancellation
    cancel_token.cancel();

    // Wait for tasks to finish (with timeout)
    let shutdown_timeout = Duration::from_secs(5);
    tokio::select! {
        _ = event_task => {
            tracing::info!("Event task stopped");
        }
        _ = tokio::time::sleep(shutdown_timeout) => {
            tracing::warn!("Event task did not stop in time, aborting");
        }
    }

    // Server doesn't need graceful shutdown, just stop accepting connections
    server.abort();

    Ok(())
```

- [ ] **Step 8.6:** Run cargo check to verify

```bash
cd /home/cassie/river-engine && cargo check -p river-discord
```

**Commit:** `fix(river-discord): graceful shutdown instead of task abort`

---

## Task 9: Create test module structure

Set up the test directory and module structure.

- [ ] **Step 9.1:** Create tests directory

```bash
mkdir -p /home/cassie/river-engine/crates/river-discord/tests
```

- [ ] **Step 9.2:** Create tests/mod.rs

**File:** `/home/cassie/river-engine/crates/river-discord/tests/mod.rs`

```rust
//! Integration tests for river-discord.

mod emoji;
mod event_conversion;
```

- [ ] **Step 9.3:** Update Cargo.toml with test dependencies

**File:** `/home/cassie/river-engine/crates/river-discord/Cargo.toml`

Add at the end:
```toml

[dev-dependencies]
tokio = { workspace = true, features = ["rt-multi-thread", "macros"] }
```

**Commit:** `test(river-discord): create test module structure`

---

## Task 10: Add emoji parsing tests

Test the emoji parsing and formatting functions.

- [ ] **Step 10.1:** Make emoji functions public for testing

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Change line (around 476):
```rust
fn format_emoji(emoji: &EmojiReactionType) -> String {
```
to:
```rust
pub fn format_emoji(emoji: &EmojiReactionType) -> String {
```

And change line (around 487):
```rust
fn parse_emoji(emoji: &str) -> twilight_http::request::channel::reaction::RequestReactionType<'_> {
```
to:
```rust
pub fn parse_emoji(emoji: &str) -> twilight_http::request::channel::reaction::RequestReactionType<'_> {
```

- [ ] **Step 10.2:** Export emoji functions from lib or make discord module public

**File:** `/home/cassie/river-engine/crates/river-discord/src/main.rs`

Add after `mod http;` (line 7):
```rust
pub use discord::{format_emoji, parse_emoji};
```

Actually, since this is a binary crate, we need unit tests instead. Add tests to discord.rs.

- [ ] **Step 10.3:** Add unit tests to discord.rs

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Add at the end of the file:
```rust

#[cfg(test)]
mod tests {
    use super::*;
    use twilight_model::id::Id;

    #[test]
    fn test_parse_unicode_emoji() {
        let emoji = parse_emoji("\u{1F44D}"); // thumbs up
        match emoji {
            twilight_http::request::channel::reaction::RequestReactionType::Unicode { name } => {
                assert_eq!(name, "\u{1F44D}");
            }
            _ => panic!("Expected unicode emoji"),
        }
    }

    #[test]
    fn test_parse_custom_emoji() {
        let emoji = parse_emoji("<:rust:123456789>");
        match emoji {
            twilight_http::request::channel::reaction::RequestReactionType::Custom { id, name } => {
                assert_eq!(id, Id::new(123456789));
                assert_eq!(name, Some("rust"));
            }
            _ => panic!("Expected custom emoji"),
        }
    }

    #[test]
    fn test_parse_animated_custom_emoji() {
        let emoji = parse_emoji("<a:dance:987654321>");
        match emoji {
            twilight_http::request::channel::reaction::RequestReactionType::Custom { id, name } => {
                assert_eq!(id, Id::new(987654321));
                assert_eq!(name, Some("dance"));
            }
            _ => panic!("Expected custom emoji"),
        }
    }

    #[test]
    fn test_format_unicode_emoji() {
        let emoji = EmojiReactionType::Unicode {
            name: "\u{1F44D}".into(),
        };
        assert_eq!(format_emoji(&emoji), "\u{1F44D}");
    }

    #[test]
    fn test_format_custom_emoji() {
        let emoji = EmojiReactionType::Custom {
            animated: false,
            id: Id::new(123456789),
            name: Some("rust".into()),
        };
        assert_eq!(format_emoji(&emoji), "<:rust:123456789>");
    }

    #[test]
    fn test_format_custom_emoji_without_name() {
        let emoji = EmojiReactionType::Custom {
            animated: false,
            id: Id::new(123456789),
            name: None,
        };
        assert_eq!(format_emoji(&emoji), "<:emoji:123456789>");
    }
}
```

- [ ] **Step 10.4:** Run tests

```bash
cd /home/cassie/river-engine && cargo test -p river-discord
```

**Commit:** `test(river-discord): add emoji parsing unit tests`

---

## Task 11: Add event conversion tests

Test the event conversion from twilight events to InboundEvents.

- [ ] **Step 11.1:** Add event conversion tests to discord.rs tests module

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Add to the `#[cfg(test)] mod tests` block:
```rust

    #[test]
    fn test_convert_message_create_skips_bot() {
        use twilight_model::channel::message::Message;
        use twilight_model::user::User;
        use twilight_model::util::Timestamp;

        // Create a minimal bot message
        let timestamp = Timestamp::from_secs(1704067200).unwrap();
        let user = User {
            id: Id::new(1),
            name: "TestBot".into(),
            bot: true,
            discriminator: 0,
            avatar: None,
            accent_color: None,
            banner: None,
            email: None,
            flags: None,
            global_name: None,
            locale: None,
            mfa_enabled: None,
            premium_type: None,
            public_flags: None,
            system: None,
            verified: None,
        };

        let msg = Box::new(twilight_model::gateway::payload::incoming::MessageCreate(
            Message {
                id: Id::new(123),
                channel_id: Id::new(456),
                author: user,
                content: "Hello from bot".into(),
                timestamp,
                edited_timestamp: None,
                tts: false,
                mention_everyone: false,
                mentions: vec![],
                mention_roles: vec![],
                attachments: vec![],
                embeds: vec![],
                pinned: false,
                kind: MessageType::Regular,
                activity: None,
                application: None,
                application_id: None,
                flags: None,
                guild_id: None,
                interaction: None,
                interaction_metadata: None,
                member: None,
                mention_channels: vec![],
                message_snapshots: vec![],
                nonce: None,
                reactions: vec![],
                reference: None,
                referenced_message: None,
                role_subscription_data: None,
                sticker_items: vec![],
                thread: None,
                webhook_id: None,
                poll: None,
                call: None,
            },
        ));

        let event = Event::MessageCreate(msg);
        let result = convert_event("discord", event);
        assert!(result.is_none(), "Should skip bot messages");
    }

    #[test]
    fn test_convert_message_delete() {
        use twilight_model::gateway::payload::incoming::MessageDelete;

        let msg = MessageDelete {
            channel_id: Id::new(456),
            id: Id::new(123),
            guild_id: None,
        };

        let event = Event::MessageDelete(msg);
        let result = convert_event("discord", event);
        assert!(result.is_some());

        let inbound = result.unwrap();
        assert_eq!(inbound.adapter, "discord");
        match inbound.metadata {
            EventMetadata::MessageDelete {
                channel,
                message_id,
            } => {
                assert_eq!(channel, "456");
                assert_eq!(message_id, "123");
            }
            _ => panic!("Expected MessageDelete event"),
        }
    }

    #[test]
    fn test_convert_typing_start() {
        use twilight_model::gateway::payload::incoming::TypingStart;

        let typing = TypingStart {
            channel_id: Id::new(456),
            user_id: Id::new(789),
            guild_id: None,
            member: None,
            timestamp: 1704067200,
        };

        let event = Event::TypingStart(Box::new(typing));
        let result = convert_event("discord", event);
        assert!(result.is_some());

        let inbound = result.unwrap();
        match inbound.metadata {
            EventMetadata::TypingStart { channel, user_id } => {
                assert_eq!(channel, "456");
                assert_eq!(user_id, "789");
            }
            _ => panic!("Expected TypingStart event"),
        }
    }

    #[test]
    fn test_convert_gateway_close() {
        use twilight_gateway::CloseFrame;

        let close = Some(CloseFrame {
            code: 4000,
            reason: "unknown error".into(),
        });

        let event = Event::GatewayClose(close);
        let result = convert_event("discord", event);
        assert!(result.is_some());

        let inbound = result.unwrap();
        match inbound.metadata {
            EventMetadata::ConnectionLost {
                reason,
                reconnecting,
            } => {
                assert!(reason.contains("4000"));
                assert!(reason.contains("unknown error"));
                assert!(reconnecting);
            }
            _ => panic!("Expected ConnectionLost event"),
        }
    }
```

- [ ] **Step 11.2:** Run tests

```bash
cd /home/cassie/river-engine && cargo test -p river-discord
```

**Commit:** `test(river-discord): add event conversion unit tests`

---

## Task 12: Add timestamp formatting test

Test the timestamp formatting function.

- [ ] **Step 12.1:** Add timestamp test to discord.rs tests module

**File:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

Add to the `#[cfg(test)] mod tests` block:
```rust

    #[test]
    fn test_format_timestamp() {
        use twilight_model::util::Timestamp;

        // 2024-01-01 00:00:00 UTC
        let ts = Timestamp::from_secs(1704067200).unwrap();
        let formatted = format_timestamp(ts);
        assert!(formatted.starts_with("2024-01-01"));
        assert!(formatted.contains("T"));
        assert!(formatted.ends_with("Z") || formatted.contains("+"));
    }
```

- [ ] **Step 12.2:** Run tests

```bash
cd /home/cassie/river-engine && cargo test -p river-discord
```

**Commit:** `test(river-discord): add timestamp formatting test`

---

## Task 13: Clean up unused imports and code

Remove any dead code warnings and clean up unused code.

- [ ] **Step 13.1:** Run clippy to find issues

```bash
cd /home/cassie/river-engine && cargo clippy -p river-discord -- -W clippy::all
```

- [ ] **Step 13.2:** Fix any clippy warnings

Address warnings like:
- Unused imports
- Dead code
- Missing documentation

- [ ] **Step 13.3:** Run final cargo check and test

```bash
cd /home/cassie/river-engine && cargo check -p river-discord && cargo test -p river-discord
```

**Commit:** `chore(river-discord): clean up warnings and unused code`

---

## Task 14: Remove unused tests directory files

Since we're using unit tests in discord.rs instead of integration tests, remove the tests directory if it was created.

- [ ] **Step 14.1:** Remove tests directory if empty/unused

```bash
rm -rf /home/cassie/river-engine/crates/river-discord/tests
```

**Commit:** (no commit needed if directory wasn't created)

---

## Verification Checklist

After completing all tasks, verify:

- [ ] `cargo check -p river-discord` passes
- [ ] `cargo test -p river-discord` passes with all tests
- [ ] `cargo clippy -p river-discord` has no warnings
- [ ] Adapter trait is implemented on DiscordClient
- [ ] /start endpoint is removed
- [ ] Reconnection logic with exponential backoff works
- [ ] ConnectionRestored event is emitted after reconnect
- [ ] Event forwarding has no polling delay
- [ ] Rate limit detection returns RateLimited error code
- [ ] MessageUpdate handles None content by skipping
- [ ] Emoji parsing tests pass
- [ ] Event conversion tests pass
- [ ] Graceful shutdown works

---

## Summary of Changes

| File | Changes |
|------|---------|
| `Cargo.toml` | Add async-trait, tokio-util dependencies |
| `src/discord.rs` | Implement Adapter trait, add reconnection, rate limit detection, fix MessageUpdate, add tests |
| `src/http.rs` | Remove /start endpoint |
| `src/main.rs` | Direct event forwarding, graceful shutdown, remove AdapterState.worker_endpoint |

Total estimated time: 2-3 hours

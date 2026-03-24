# Adapter Framework

> **⚠️ SUPERSEDED:** This plan has been merged into Phase 0 and Phase 0.5:
> - **Phase 0** (`2026-03-23-plan-phase0-extract-crates.md`) — Creates river-adapter crate (Tasks 1-2 from here)
> - **Phase 0.5** (`2026-03-23-plan-phase0.5-discord-refactor.md`) — Gateway integration + Discord refactor (Tasks 3-5 from here)
>
> **Do not execute this plan.** Use Phase 0 and Phase 0.5 instead.

---

**Original goal:** Create the `river-adapter` crate with shared types, trait, feature flags, and OpenAPI generation. Refactor `river-discord` to use shared types and self-register with the gateway.

**Architecture:** New `river-adapter` crate defines the contract between gateway and adapters. Gateway stores adapters in a registry keyed by name. Adapters are external processes communicating via HTTP. Discord is the reference implementation.

**Tech Stack:** serde, utoipa (OpenAPI), async-trait, reqwest

**Depends on:** Nothing (can proceed in parallel with I/You phases)

---

## File Structure

**New files:**
- `crates/river-adapter/Cargo.toml`
- `crates/river-adapter/src/lib.rs` — exports, OpenAPI doc generation
- `crates/river-adapter/src/types.rs` — IncomingEvent, SendRequest, SendResponse, Author
- `crates/river-adapter/src/capabilities.rs` — Feature enum
- `crates/river-adapter/src/registration.rs` — AdapterInfo, RegisterRequest/Response
- `crates/river-adapter/src/traits.rs` — Adapter trait
- `crates/river-adapter/src/error.rs` — AdapterError
- `crates/river-adapter/src/http.rs` — HttpAdapter implementation
- `crates/river-adapter/openapi.json` — generated, committed

**Modified files:**
- `Cargo.toml` — add river-adapter to workspace
- `crates/river-gateway/Cargo.toml` — add river-adapter dep
- `crates/river-gateway/src/tools/communication.rs` — migrate to river-adapter types
- `crates/river-gateway/src/api/routes.rs` — add adapter registration endpoint
- `crates/river-discord/Cargo.toml` — add river-adapter dep
- `crates/river-discord/src/` — use shared types, self-register

---

## Task 1: Create river-adapter Crate (Types)

- [ ] **Step 1: Create crate**

```bash
mkdir -p crates/river-adapter/src
```

Write `crates/river-adapter/Cargo.toml`:
```toml
[package]
name = "river-adapter"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
chrono.workspace = true
thiserror.workspace = true
async-trait = "0.1"
reqwest.workspace = true

[dev-dependencies]
tokio.workspace = true
```

- [ ] **Step 2: Create types.rs**

```rust
//! Core message types for adapter communication

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Incoming event from adapter to gateway
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingEvent {
    pub adapter: String,
    pub event_type: EventType,
    pub channel: String,
    pub channel_name: Option<String>,
    pub author: Author,
    pub content: String,
    pub message_id: String,
    pub timestamp: DateTime<Utc>,
    /// Native platform structure (opaque to gateway)
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventType {
    MessageCreate,
    MessageUpdate,
    MessageDelete,
    ReactionAdd,
    ReactionRemove,
    Identify(String),
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub is_bot: bool,
}

/// Outgoing message from gateway to adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRequest {
    pub channel: String,
    pub content: String,
    #[serde(default)]
    pub options: SendOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SendOptions {
    pub reply_to: Option<String>,
    pub thread_id: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResponse {
    pub success: bool,
    pub message_id: Option<String>,
    pub error: Option<String>,
}
```

- [ ] **Step 3: Create capabilities.rs**

```rust
//! Feature flags for adapter capabilities

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Feature {
    ReadHistory,
    Reactions,
    Threads,
    Attachments,
    Embeds,
    TypingIndicator,
    EditMessage,
    DeleteMessage,
    Custom(String),
}
```

- [ ] **Step 4: Create registration.rs**

```rust
//! Adapter registration types

use crate::capabilities::Feature;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub name: String,
    pub version: String,
    pub url: String,
    pub features: HashSet<Feature>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub adapter: AdapterInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub accepted: bool,
    pub error: Option<String>,
}
```

- [ ] **Step 5: Create error.rs**

```rust
//! Adapter error types

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Adapter not found: {0}")]
    NotFound(String),
    #[error("Feature not supported: {0:?}")]
    FeatureNotSupported(crate::capabilities::Feature),
    #[error("Adapter error: {0}")]
    Other(String),
}
```

- [ ] **Step 6: Create lib.rs**

```rust
//! River Adapter — shared types for communication adapters

pub mod types;
pub mod capabilities;
pub mod registration;
pub mod traits;
pub mod error;
pub mod http;

pub use types::{IncomingEvent, EventType, Author, SendRequest, SendOptions, SendResponse};
pub use capabilities::Feature;
pub use registration::{AdapterInfo, RegisterRequest, RegisterResponse};
pub use traits::Adapter;
pub use error::AdapterError;
pub use http::HttpAdapter;
```

- [ ] **Step 7: Verify compilation**

```bash
cargo check -p river-adapter
```

- [ ] **Step 8: Commit**

```bash
git add crates/river-adapter/
git commit -m "feat: create river-adapter crate with shared types"
```

---

## Task 2: Adapter Trait and HttpAdapter

- [ ] **Step 1: Create traits.rs**

```rust
//! Adapter trait for gateway-side abstraction

use crate::{AdapterError, Feature, IncomingEvent, SendRequest, SendResponse};
use async_trait::async_trait;

#[async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &str;
    fn supports(&self, feature: &Feature) -> bool;
    async fn send(&self, request: SendRequest) -> Result<SendResponse, AdapterError>;
    async fn read_history(&self, channel: &str, limit: usize) -> Result<Vec<IncomingEvent>, AdapterError>;
    async fn health(&self) -> Result<bool, AdapterError>;
}
```

- [ ] **Step 2: Create http.rs**

```rust
//! HTTP-based adapter client

use crate::{Adapter, AdapterError, AdapterInfo, Feature, IncomingEvent, SendRequest, SendResponse};
use async_trait::async_trait;
use std::collections::HashSet;

pub struct HttpAdapter {
    pub info: AdapterInfo,
    client: reqwest::Client,
}

impl HttpAdapter {
    pub fn new(info: AdapterInfo) -> Self {
        Self {
            info,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Adapter for HttpAdapter {
    fn name(&self) -> &str {
        &self.info.name
    }

    fn supports(&self, feature: &Feature) -> bool {
        self.info.features.contains(feature)
    }

    async fn send(&self, request: SendRequest) -> Result<SendResponse, AdapterError> {
        let url = format!("{}/send", self.info.url);
        let response = self.client.post(&url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    async fn read_history(&self, channel: &str, limit: usize) -> Result<Vec<IncomingEvent>, AdapterError> {
        if !self.supports(&Feature::ReadHistory) {
            return Err(AdapterError::FeatureNotSupported(Feature::ReadHistory));
        }
        let url = format!("{}/history/{}?limit={}", self.info.url, channel, limit);
        let response = self.client.get(&url)
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    async fn health(&self) -> Result<bool, AdapterError> {
        let url = format!("{}/health", self.info.url);
        let response: serde_json::Value = self.client.get(&url)
            .send()
            .await?
            .json()
            .await?;
        Ok(response.get("healthy").and_then(|v| v.as_bool()).unwrap_or(false))
    }
}
```

- [ ] **Step 3: Write tests**

Test: MockAdapter implements trait, feature checking works.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(adapter): add Adapter trait and HttpAdapter implementation"
```

---

## Task 3: Gateway Integration

- [ ] **Step 1: Add river-adapter dependency to gateway**

```toml
river-adapter = { path = "../river-adapter" }
```

- [ ] **Step 2: Add adapter registration endpoint**

In `api/routes.rs`, add:
```rust
async fn register_adapter(
    State(state): State<Arc<AppState>>,
    Json(request): Json<river_adapter::RegisterRequest>,
) -> Json<river_adapter::RegisterResponse> {
    // Store adapter in registry
    // ...
}
```

Add route: `POST /adapters/register`

- [ ] **Step 3: Update AdapterRegistry in communication tools**

Replace the simple `HashMap<String, AdapterConfig>` with `HashMap<String, Box<dyn Adapter>>`. Or: keep both registries during migration, new one for river-adapter types.

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p river-gateway
```

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(gateway): add adapter registration endpoint"
```

---

## Task 4: Discord Migration

- [ ] **Step 1: Add river-adapter dependency to discord**

```toml
river-adapter = { path = "../river-adapter" }
```

- [ ] **Step 2: Create event mapping function**

```rust
fn discord_message_to_event(msg: &twilight_model::channel::Message) -> river_adapter::IncomingEvent {
    river_adapter::IncomingEvent {
        adapter: "discord".into(),
        event_type: river_adapter::EventType::MessageCreate,
        channel: msg.channel_id.to_string(),
        channel_name: None, // populated later
        author: river_adapter::Author {
            id: msg.author.id.to_string(),
            name: msg.author.name.clone(),
            is_bot: msg.author.bot,
        },
        content: msg.content.clone(),
        message_id: msg.id.to_string(),
        timestamp: msg.timestamp.into(),
        metadata: serde_json::json!({
            "guild_id": msg.guild_id,
        }),
    }
}
```

- [ ] **Step 3: Self-register on startup**

After Discord connects:
```rust
let info = river_adapter::AdapterInfo {
    name: "discord".into(),
    version: env!("CARGO_PKG_VERSION").into(),
    url: format!("http://localhost:{}", config.port),
    features: HashSet::from([
        Feature::ReadHistory,
        Feature::Reactions,
        Feature::Threads,
        Feature::Attachments,
        Feature::Embeds,
        Feature::EditMessage,
        Feature::DeleteMessage,
        Feature::TypingIndicator,
    ]),
    metadata: serde_json::json!({ "bot_id": bot_id }),
};

reqwest::Client::new()
    .post(format!("{}/adapters/register", gateway_url))
    .json(&river_adapter::RegisterRequest { adapter: info })
    .send()
    .await?;
```

- [ ] **Step 4: Use shared types for sending**

Migrate outbound message sending to use `river_adapter::SendRequest` / `SendResponse`.

- [ ] **Step 5: Run all tests**

```bash
cargo test
```

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(discord): migrate to river-adapter types with self-registration"
```

---

## Task 5: OpenAPI Generation

- [ ] **Step 1: Add utoipa to river-adapter**

```toml
utoipa = { version = "5", features = ["chrono"] }
```

- [ ] **Step 2: Add #[derive(ToSchema)] to all types**

Add `utoipa::ToSchema` to: `IncomingEvent`, `EventType`, `Author`, `SendRequest`, `SendOptions`, `SendResponse`, `AdapterInfo`, `Feature`, `RegisterRequest`, `RegisterResponse`.

- [ ] **Step 3: Generate openapi.json**

Add a build script or test that generates `openapi.json`:

```rust
#[test]
fn generate_openapi() {
    let doc = AdapterApiDoc::openapi();
    let json = doc.to_pretty_json().unwrap();
    std::fs::write("openapi.json", json).unwrap();
}
```

- [ ] **Step 4: Commit generated spec**

```bash
git add crates/river-adapter/openapi.json
git commit -m "docs(adapter): generate OpenAPI spec from Rust types"
```

---

## Summary

The adapter framework plan:
1. **river-adapter crate** — shared types, trait, feature flags
2. **HttpAdapter** — gateway-side client for external adapters
3. **Gateway integration** — registration endpoint, updated registry
4. **Discord migration** — use shared types, self-register
5. **OpenAPI generation** — spec from Rust types

Total: 5 tasks, ~25 steps. Fully independent of I/You phases.

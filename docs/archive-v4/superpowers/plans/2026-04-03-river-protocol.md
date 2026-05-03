# river-protocol Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a foundational crate consolidating all shared wire protocol types, eliminating duplication across crates.

**Architecture:** New `river-protocol` crate with zero river-* dependencies. All identity types move from `river-adapter`, duplicated registry types consolidated, registration protocols unified. Other crates depend on `river-protocol` for shared types.

**Tech Stack:** Rust, serde, utoipa

---

## File Structure

```
crates/river-protocol/
├── Cargo.toml
└── src/
    ├── lib.rs           # Re-exports all public types
    ├── identity.rs      # Side, Baton, Channel, Author, Attachment, Ground
    ├── registry.rs      # ProcessEntry, Registry
    ├── model.rs         # ModelConfig
    └── registration.rs  # Worker/Adapter registration types
```

**Files to modify:**
- `Cargo.toml` (workspace members)
- `crates/river-adapter/Cargo.toml` (add river-protocol dep)
- `crates/river-adapter/src/lib.rs` (re-export from river-protocol)
- `crates/river-adapter/src/author.rs` (remove, types moved)
- `crates/river-context/Cargo.toml` (change dep from river-adapter to river-protocol)
- `crates/river-context/src/lib.rs` (update re-exports)
- `crates/river-worker/Cargo.toml` (add river-protocol dep)
- `crates/river-worker/src/state.rs` (remove duplicated types)
- `crates/river-worker/src/config.rs` (remove types moved to protocol)
- `crates/river-orchestrator/Cargo.toml` (add river-protocol dep)
- `crates/river-orchestrator/src/registry.rs` (remove duplicated types)
- `crates/river-discord/Cargo.toml` (add river-protocol dep)
- `crates/river-discord/src/main.rs` (remove local registration types)
- `crates/river-tui/Cargo.toml` (add river-protocol dep)
- `crates/river-tui/src/main.rs` (remove local registration types)

---

### Task 1: Create river-protocol crate with identity types

**Files:**
- Create: `crates/river-protocol/Cargo.toml`
- Create: `crates/river-protocol/src/lib.rs`
- Create: `crates/river-protocol/src/identity.rs`
- Modify: `Cargo.toml` (workspace already includes `crates/*`)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "river-protocol"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Shared protocol types for River Engine"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
utoipa = { workspace = true }
```

- [ ] **Step 2: Create identity.rs with types from river-adapter**

```rust
//! Identity types for River Engine entities.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Message author information.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Author {
    /// Unique identifier for the author.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether this is a bot account.
    pub bot: bool,
}

/// Communication channel identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct Channel {
    /// Adapter type (e.g., "discord", "slack").
    pub adapter: String,
    /// Channel identifier.
    pub id: String,
    /// Human-readable channel name.
    pub name: Option<String>,
}

/// File attachment metadata.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Attachment {
    /// Unique identifier.
    pub id: String,
    /// Original filename.
    pub filename: String,
    /// URL to download the attachment.
    pub url: String,
    /// File size in bytes.
    pub size: u64,
    /// MIME content type.
    pub content_type: Option<String>,
}

/// Worker role (actor or spectator).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Baton {
    /// Actor: handles external communication
    Actor,
    /// Spectator: manages memory and reviews
    Spectator,
}

/// Fixed position in the dyad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// Get the opposite side.
    pub fn opposite(&self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Human operator information.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Ground {
    /// Human operator name.
    pub name: String,
    /// Human operator platform ID.
    pub id: String,
    /// Channel for reaching the human.
    pub channel: Channel,
}
```

- [ ] **Step 3: Create lib.rs with exports**

```rust
//! River Protocol - Shared types for River Engine.
//!
//! This crate provides foundational types used across all River Engine crates.
//! It has no dependencies on other river-* crates.

mod identity;

pub use identity::{Attachment, Author, Baton, Channel, Ground, Side};
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p river-protocol`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add crates/river-protocol
git commit -m "feat: create river-protocol crate with identity types"
```

---

### Task 2: Add registry types to river-protocol

**Files:**
- Create: `crates/river-protocol/src/registry.rs`
- Modify: `crates/river-protocol/src/lib.rs`

- [ ] **Step 1: Create registry.rs**

```rust
//! Registry types for process discovery.

use crate::{Baton, Ground, Side};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Process entry in the registry.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessEntry {
    Worker {
        endpoint: String,
        dyad: String,
        side: Side,
        baton: Baton,
        model: String,
        ground: Ground,
    },
    Adapter {
        endpoint: String,
        #[serde(rename = "adapter_type")]
        adapter_type: String,
        dyad: String,
        features: Vec<u16>,
    },
    EmbedService {
        endpoint: String,
        name: String,
    },
}

impl ProcessEntry {
    /// Get the endpoint for this process.
    pub fn endpoint(&self) -> &str {
        match self {
            ProcessEntry::Worker { endpoint, .. } => endpoint,
            ProcessEntry::Adapter { endpoint, .. } => endpoint,
            ProcessEntry::EmbedService { endpoint, .. } => endpoint,
        }
    }
}

/// The full registry sent to all processes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Registry {
    pub processes: Vec<ProcessEntry>,
}

impl Registry {
    /// Find embed service endpoint.
    pub fn embed_endpoint(&self) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::EmbedService { endpoint, .. } => Some(endpoint.as_str()),
            _ => None,
        })
    }

    /// Find adapter endpoint by type.
    pub fn adapter_endpoint(&self, adapter_type: &str) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::Adapter {
                endpoint,
                adapter_type: t,
                ..
            } if t == adapter_type => Some(endpoint.as_str()),
            _ => None,
        })
    }

    /// Find worker endpoint by dyad and side.
    pub fn worker_endpoint(&self, dyad: &str, side: &Side) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::Worker {
                endpoint,
                dyad: d,
                side: s,
                ..
            } if d == dyad && s == side => Some(endpoint.as_str()),
            _ => None,
        })
    }
}
```

- [ ] **Step 2: Update lib.rs to export registry types**

```rust
//! River Protocol - Shared types for River Engine.
//!
//! This crate provides foundational types used across all River Engine crates.
//! It has no dependencies on other river-* crates.

mod identity;
mod registry;

pub use identity::{Attachment, Author, Baton, Channel, Ground, Side};
pub use registry::{ProcessEntry, Registry};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p river-protocol`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/river-protocol/src/registry.rs crates/river-protocol/src/lib.rs
git commit -m "feat(protocol): add ProcessEntry and Registry types"
```

---

### Task 3: Add model and registration types to river-protocol

**Files:**
- Create: `crates/river-protocol/src/model.rs`
- Create: `crates/river-protocol/src/registration.rs`
- Modify: `crates/river-protocol/src/lib.rs`

- [ ] **Step 1: Create model.rs**

```rust
//! Model configuration types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Model configuration from orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelConfig {
    /// LLM API endpoint URL.
    pub endpoint: String,
    /// Model name/identifier.
    pub name: String,
    /// API key for authentication.
    pub api_key: String,
    /// Maximum context window size in tokens.
    pub context_limit: usize,
}
```

- [ ] **Step 2: Create registration.rs**

```rust
//! Registration protocol types for workers and adapters.

use crate::{Baton, Ground, ModelConfig, Side};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// === Worker Registration ===

/// Worker identity for registration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkerRegistration {
    pub dyad: String,
    pub side: Side,
}

/// Worker registration request to orchestrator.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkerRegistrationRequest {
    pub endpoint: String,
    pub worker: WorkerRegistration,
}

/// Worker registration response from orchestrator.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WorkerRegistrationResponse {
    pub accepted: bool,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub model: ModelConfig,
    pub ground: Ground,
    pub workspace: String,
    pub initial_message: Option<String>,
    pub start_sleeping: bool,
}

// === Adapter Registration ===

/// Adapter identity for registration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdapterRegistration {
    #[serde(rename = "type")]
    pub adapter_type: String,
    pub dyad: String,
    pub features: Vec<u16>,
}

/// Adapter registration request to orchestrator.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdapterRegistrationRequest {
    pub endpoint: String,
    pub adapter: AdapterRegistration,
}

/// Adapter registration response from orchestrator.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AdapterRegistrationResponse {
    pub accepted: bool,
    /// Adapter-specific configuration (e.g., Discord token).
    pub config: serde_json::Value,
    pub worker_endpoint: String,
}
```

- [ ] **Step 3: Update lib.rs with all exports**

```rust
//! River Protocol - Shared types for River Engine.
//!
//! This crate provides foundational types used across all River Engine crates.
//! It has no dependencies on other river-* crates.

mod identity;
mod model;
mod registration;
mod registry;

pub use identity::{Attachment, Author, Baton, Channel, Ground, Side};
pub use model::ModelConfig;
pub use registration::{
    AdapterRegistration, AdapterRegistrationRequest, AdapterRegistrationResponse,
    WorkerRegistration, WorkerRegistrationRequest, WorkerRegistrationResponse,
};
pub use registry::{ProcessEntry, Registry};
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p river-protocol`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add crates/river-protocol/src/model.rs crates/river-protocol/src/registration.rs crates/river-protocol/src/lib.rs
git commit -m "feat(protocol): add ModelConfig and registration types"
```

---

### Task 4: Update river-adapter to use river-protocol

**Files:**
- Modify: `crates/river-adapter/Cargo.toml`
- Modify: `crates/river-adapter/src/lib.rs`
- Delete: `crates/river-adapter/src/author.rs`

- [ ] **Step 1: Add river-protocol dependency to Cargo.toml**

Add to `[dependencies]` section:
```toml
river-protocol = { path = "../river-protocol" }
```

- [ ] **Step 2: Update lib.rs to re-export from river-protocol**

Replace the author module import and exports:

```rust
//! River Adapter - Types-only library for adapter ↔ worker communication.
//!
//! This crate defines the interface between Workers and adapter binaries.
//! It exports types, traits, and enums — no HTTP infrastructure.
//!
//! # Feature System
//!
//! Two enums work together:
//! - [`FeatureId`]: Lightweight enum for registration and capability checks
//! - [`OutboundRequest`]: Data-carrying enum with typed payloads
//!
//! # Usage
//!
//! ```rust
//! use river_adapter::{FeatureId, OutboundRequest, Adapter, InboundEvent, EventMetadata, Author};
//!
//! // Check if a feature is required
//! assert!(FeatureId::SendMessage.is_required());
//! assert!(FeatureId::ReceiveMessage.is_required());
//! assert!(!FeatureId::EditMessage.is_required());
//!
//! // Get the feature ID for a request
//! let request = OutboundRequest::SendMessage {
//!     channel: "general".into(),
//!     content: "Hello!".into(),
//!     reply_to: None,
//! };
//! assert_eq!(request.feature_id(), FeatureId::SendMessage);
//! ```

mod error;
mod event;
mod feature;
mod response;
mod traits;

// Re-export identity types from river-protocol
pub use river_protocol::{Attachment, Author, Baton, Channel, Ground, Side};

pub use error::AdapterError;
pub use event::{EventMetadata, EventType, InboundEvent};
pub use feature::{FeatureId, OutboundRequest};
pub use response::{ErrorCode, HistoryMessage, OutboundResponse, ResponseData, ResponseError};
pub use traits::Adapter;

use utoipa::OpenApi;

/// OpenAPI documentation for adapter types.
#[derive(OpenApi)]
#[openapi(components(schemas(
    // Feature system
    FeatureId,
    OutboundRequest,
    // Inbound events
    InboundEvent,
    EventMetadata,
    EventType,
    // Responses
    OutboundResponse,
    ResponseData,
    ResponseError,
    ErrorCode,
    HistoryMessage,
    // Supporting (from river-protocol)
    Author,
    Channel,
    Attachment,
    Baton,
    Side,
    Ground,
)))]
pub struct AdapterApiDoc;

/// Generate OpenAPI JSON specification.
pub fn openapi_json() -> String {
    AdapterApiDoc::openapi()
        .to_pretty_json()
        .expect("failed to generate OpenAPI JSON")
}
```

- [ ] **Step 3: Delete author.rs**

```bash
rm crates/river-adapter/src/author.rs
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p river-adapter`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add crates/river-adapter/Cargo.toml crates/river-adapter/src/lib.rs
git rm crates/river-adapter/src/author.rs
git commit -m "refactor(adapter): use identity types from river-protocol"
```

---

### Task 5: Update river-context to use river-protocol

**Files:**
- Modify: `crates/river-context/Cargo.toml`
- Modify: `crates/river-context/src/lib.rs`

- [ ] **Step 1: Add river-protocol dependency to Cargo.toml**

Add to `[dependencies]` section:
```toml
river-protocol = { path = "../river-protocol" }
```

- [ ] **Step 2: Update lib.rs re-exports**

Change the re-export at the bottom from:
```rust
// Re-export types from river-adapter
pub use river_adapter::{Author, Channel};
```

To:
```rust
// Re-export types from river-protocol
pub use river_protocol::{Author, Channel};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p river-context`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/river-context/Cargo.toml crates/river-context/src/lib.rs
git commit -m "refactor(context): use types from river-protocol"
```

---

### Task 6: Update river-worker to use river-protocol

**Files:**
- Modify: `crates/river-worker/Cargo.toml`
- Modify: `crates/river-worker/src/state.rs`
- Modify: `crates/river-worker/src/config.rs`

- [ ] **Step 1: Add river-protocol dependency to Cargo.toml**

Add to `[dependencies]` section:
```toml
river-protocol = { path = "../river-protocol" }
```

- [ ] **Step 2: Update state.rs to import from river-protocol**

Replace the local ProcessEntry and Registry with imports:

```rust
//! Worker state.

use crate::config::WorkerConfig;
use river_adapter::{Baton, Channel, Ground, Side};
use river_context::Flash;
use river_protocol::{ModelConfig, ProcessEntry, Registry, WorkerRegistrationResponse};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Notification about new messages.
#[derive(Debug, Clone)]
pub struct Notification {
    pub channel: Channel,
    pub count: usize,
    pub since_id: Option<String>,
}

/// Worker state.
#[derive(Debug)]
pub struct WorkerState {
    // Identity
    pub dyad: String,
    pub side: Side,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub ground: Ground,
    pub workspace: PathBuf,

    // Communication
    pub current_channel: Channel,
    pub watch_list: HashSet<String>, // Channel keys: "adapter:id"

    // Registry
    pub registry: Registry,

    // Model
    pub model_config: ModelConfig,
    pub token_count: usize,
    pub context_limit: usize,

    // Loop control
    pub sleeping: bool,
    pub sleep_until: Option<Instant>,
    pub pending_notifications: Vec<Notification>,
    pub pending_flashes: Vec<Flash>,

    // Role switching
    pub switch_pending: bool,

    // Initial context (loaded from files at startup)
    pub role_content: Option<String>,
    pub identity_content: Option<String>,
    pub initial_message: Option<String>,
}

impl WorkerState {
    /// Create initial state from config and registration.
    pub fn new(config: &WorkerConfig, registration: WorkerRegistrationResponse) -> Self {
        Self {
            dyad: config.dyad.clone(),
            side: config.side.clone(),
            baton: registration.baton,
            partner_endpoint: registration.partner_endpoint,
            ground: registration.ground.clone(),
            workspace: PathBuf::from(&registration.workspace),
            current_channel: registration.ground.channel.clone(),
            watch_list: HashSet::new(),
            registry: Registry::default(),
            model_config: registration.model,
            token_count: 0,
            context_limit: 0, // Will be set from model config
            sleeping: registration.start_sleeping,
            sleep_until: None,
            pending_notifications: Vec::new(),
            pending_flashes: Vec::new(),
            switch_pending: false,
            role_content: None,
            identity_content: None,
            initial_message: None,
        }
    }

    /// Get channel key for watch list.
    pub fn channel_key(channel: &Channel) -> String {
        format!("{}:{}", channel.adapter, channel.id)
    }

    /// Check if a channel is in the watch list.
    pub fn is_watched(&self, channel: &Channel) -> bool {
        self.watch_list.contains(&Self::channel_key(channel))
    }

    /// Add channel to watch list.
    pub fn watch(&mut self, channel: &Channel) {
        self.watch_list.insert(Self::channel_key(channel));
    }

    /// Remove channel from watch list.
    pub fn unwatch(&mut self, channel: &Channel) {
        self.watch_list.remove(&Self::channel_key(channel));
    }

    /// Get partner side.
    pub fn partner_side(&self) -> Side {
        self.side.opposite()
    }
}

/// Thread-safe state wrapper.
pub type SharedState = Arc<RwLock<WorkerState>>;

pub fn new_shared_state(config: &WorkerConfig, registration: WorkerRegistrationResponse) -> SharedState {
    let state = WorkerState::new(config, registration);
    Arc::new(RwLock::new(state))
}
```

- [ ] **Step 3: Update config.rs to use river-protocol types**

```rust
//! Worker configuration.

use river_adapter::Side;
use river_protocol::{WorkerRegistration, WorkerRegistrationRequest, WorkerRegistrationResponse};
use std::path::PathBuf;

/// Worker config from CLI args.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub orchestrator_endpoint: String,
    pub dyad: String,
    pub side: Side,
    pub port: u16,
}

// Re-export registration types for convenience
pub use river_protocol::ModelConfig;
pub type RegistrationResponse = WorkerRegistrationResponse;
pub type RegistrationRequest = WorkerRegistrationRequest;

impl WorkerConfig {
    pub fn workspace_path(&self, registration: &RegistrationResponse) -> PathBuf {
        PathBuf::from(&registration.workspace)
    }

    pub fn context_path(&self, registration: &RegistrationResponse) -> PathBuf {
        let workspace = self.workspace_path(registration);
        let side_str = match self.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        workspace.join(side_str).join("context.jsonl")
    }

    pub fn identity_path(&self, registration: &RegistrationResponse) -> PathBuf {
        let workspace = self.workspace_path(registration);
        let side_str = match self.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        workspace.join(side_str).join("identity.md")
    }

    pub fn role_path(&self, registration: &RegistrationResponse) -> PathBuf {
        use river_adapter::Baton;
        let workspace = self.workspace_path(registration);
        let role_str = match registration.baton {
            Baton::Actor => "actor",
            Baton::Spectator => "spectator",
        };
        workspace.join("roles").join(format!("{}.md", role_str))
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p river-worker`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add crates/river-worker/Cargo.toml crates/river-worker/src/state.rs crates/river-worker/src/config.rs
git commit -m "refactor(worker): use types from river-protocol"
```

---

### Task 7: Update river-orchestrator to use river-protocol

**Files:**
- Modify: `crates/river-orchestrator/Cargo.toml`
- Modify: `crates/river-orchestrator/src/registry.rs`

- [ ] **Step 1: Add river-protocol dependency to Cargo.toml**

Add to `[dependencies]` section:
```toml
river-protocol = { path = "../river-protocol" }
```

- [ ] **Step 2: Update registry.rs to import shared types**

Replace the local ProcessEntry and Registry definitions, keep RegistryState and push logic:

```rust
//! Registry state and push mechanism.

use river_protocol::{Baton, Ground, ProcessEntry, Registry, Side};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Key for identifying a worker.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct WorkerKey {
    pub dyad: String,
    pub side: Side,
}

/// Key for identifying an adapter.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct AdapterKey {
    pub dyad: String,
    pub adapter_type: String,
}

/// Internal registry state.
#[derive(Debug, Default)]
pub struct RegistryState {
    workers: HashMap<WorkerKey, ProcessEntry>,
    adapters: HashMap<AdapterKey, ProcessEntry>,
    embed_services: HashMap<String, ProcessEntry>,
}

impl RegistryState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or update a worker.
    pub fn register_worker(
        &mut self,
        dyad: String,
        side: Side,
        endpoint: String,
        baton: Baton,
        model: String,
        ground: Ground,
    ) {
        let key = WorkerKey {
            dyad: dyad.clone(),
            side: side.clone(),
        };
        let entry = ProcessEntry::Worker {
            endpoint,
            dyad,
            side,
            baton,
            model,
            ground,
        };
        self.workers.insert(key, entry);
    }

    /// Register or update an adapter.
    pub fn register_adapter(
        &mut self,
        dyad: String,
        adapter_type: String,
        endpoint: String,
        features: Vec<u16>,
    ) {
        let key = AdapterKey {
            dyad: dyad.clone(),
            adapter_type: adapter_type.clone(),
        };
        let entry = ProcessEntry::Adapter {
            endpoint,
            adapter_type,
            dyad,
            features,
        };
        self.adapters.insert(key, entry);
    }

    /// Register or update an embed service.
    pub fn register_embed(&mut self, name: String, endpoint: String) {
        let entry = ProcessEntry::EmbedService {
            endpoint,
            name: name.clone(),
        };
        self.embed_services.insert(name, entry);
    }

    /// Update a worker's baton.
    pub fn update_worker_baton(&mut self, dyad: &str, side: &Side, new_baton: Baton) -> bool {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        if let Some(ProcessEntry::Worker { baton, .. }) = self.workers.get_mut(&key) {
            *baton = new_baton;
            true
        } else {
            false
        }
    }

    /// Update a worker's model.
    pub fn update_worker_model(&mut self, dyad: &str, side: &Side, new_model: String) -> bool {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        if let Some(ProcessEntry::Worker { model, .. }) = self.workers.get_mut(&key) {
            *model = new_model;
            true
        } else {
            false
        }
    }

    /// Remove a worker from registry.
    pub fn remove_worker(&mut self, dyad: &str, side: &Side) {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        self.workers.remove(&key);
    }

    /// Remove an adapter from registry.
    pub fn remove_adapter(&mut self, dyad: &str, adapter_type: &str) {
        let key = AdapterKey {
            dyad: dyad.to_string(),
            adapter_type: adapter_type.to_string(),
        };
        self.adapters.remove(&key);
    }

    /// Remove an embed service from registry.
    pub fn remove_embed(&mut self, name: &str) {
        self.embed_services.remove(name);
    }

    /// Get worker endpoint.
    pub fn get_worker_endpoint(&self, dyad: &str, side: &Side) -> Option<String> {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        self.workers.get(&key).map(|e| e.endpoint().to_string())
    }

    /// Get partner worker endpoint.
    pub fn get_partner_endpoint(&self, dyad: &str, side: &Side) -> Option<String> {
        let partner_side = side.opposite();
        self.get_worker_endpoint(dyad, &partner_side)
    }

    /// Get embed service endpoint.
    pub fn get_embed_endpoint(&self, name: &str) -> Option<String> {
        self.embed_services.get(name).map(|e| e.endpoint().to_string())
    }

    /// Build the registry snapshot for pushing.
    pub fn build_registry(&self) -> Registry {
        let mut processes = Vec::new();
        processes.extend(self.workers.values().cloned());
        processes.extend(self.adapters.values().cloned());
        processes.extend(self.embed_services.values().cloned());
        Registry { processes }
    }

    /// Get all endpoints for pushing.
    pub fn all_endpoints(&self) -> Vec<String> {
        let mut endpoints = Vec::new();
        for entry in self.workers.values() {
            endpoints.push(entry.endpoint().to_string());
        }
        for entry in self.adapters.values() {
            endpoints.push(entry.endpoint().to_string());
        }
        for entry in self.embed_services.values() {
            endpoints.push(entry.endpoint().to_string());
        }
        endpoints
    }

    /// Get worker count.
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Get adapter count.
    pub fn adapter_count(&self) -> usize {
        self.adapters.len()
    }

    /// Get embed service count.
    pub fn embed_count(&self) -> usize {
        self.embed_services.len()
    }
}

/// Push registry to all endpoints.
pub async fn push_registry(
    client: &reqwest::Client,
    registry: &Registry,
    endpoints: &[String],
) {
    for endpoint in endpoints {
        let url = format!("{}/registry", endpoint);
        let registry_clone = registry.clone();
        let client_clone = client.clone();

        // Fire and forget - don't wait for responses
        tokio::spawn(async move {
            if let Err(e) = client_clone
                .post(&url)
                .json(&registry_clone)
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
            {
                tracing::warn!("Failed to push registry to {}: {}", url, e);
            }
        });
    }
}

/// Thread-safe registry wrapper.
pub type SharedRegistry = Arc<RwLock<RegistryState>>;

pub fn new_shared_registry() -> SharedRegistry {
    Arc::new(RwLock::new(RegistryState::new()))
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p river-orchestrator`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/Cargo.toml crates/river-orchestrator/src/registry.rs
git commit -m "refactor(orchestrator): use types from river-protocol"
```

---

### Task 8: Update river-discord to use river-protocol

**Files:**
- Modify: `crates/river-discord/Cargo.toml`
- Modify: `crates/river-discord/src/main.rs`

- [ ] **Step 1: Add river-protocol dependency to Cargo.toml**

Add to `[dependencies]` section:
```toml
river-protocol = { path = "../river-protocol" }
```

- [ ] **Step 2: Update main.rs to use river-protocol registration types**

Remove local registration type definitions and import from river-protocol. Replace:

```rust
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
    config: DiscordConfig,
    worker_endpoint: String,
}
```

With imports and keep only DiscordConfig:

```rust
use river_protocol::{AdapterRegistration, AdapterRegistrationRequest, AdapterRegistrationResponse};

/// Discord-specific config from orchestrator.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub guild_id: Option<u64>,
    pub intents: Option<u64>,
}
```

Then update the registration code to use `AdapterRegistrationRequest` instead of `RegistrationRequest`, and parse `AdapterRegistrationResponse` then extract config:

```rust
let reg_request = AdapterRegistrationRequest {
    endpoint: adapter_endpoint.clone(),
    adapter: AdapterRegistration {
        adapter_type: args.adapter_type.clone(),
        dyad: args.dyad.clone(),
        features: features.iter().map(|f| *f as u16).collect(),
    },
};
```

And for the response, parse the config field:
```rust
let registration: AdapterRegistrationResponse = response.json().await?;
let discord_config: DiscordConfig = serde_json::from_value(registration.config)?;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p river-discord`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/river-discord/Cargo.toml crates/river-discord/src/main.rs
git commit -m "refactor(discord): use registration types from river-protocol"
```

---

### Task 9: Update river-tui to use river-protocol

**Files:**
- Modify: `crates/river-tui/Cargo.toml`
- Modify: `crates/river-tui/src/main.rs`

- [ ] **Step 1: Add river-protocol dependency to Cargo.toml**

Add to `[dependencies]` section:
```toml
river-protocol = { path = "../river-protocol" }
```

- [ ] **Step 2: Update main.rs to use river-protocol registration types**

Remove local registration type definitions and import from river-protocol. Replace:

```rust
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
    #[allow(dead_code)]
    config: serde_json::Value,
    worker_endpoint: String,
}
```

With:

```rust
use river_protocol::{AdapterRegistration, AdapterRegistrationRequest, AdapterRegistrationResponse};
```

Then update the registration code:

```rust
let reg_request = AdapterRegistrationRequest {
    endpoint: adapter_endpoint.clone(),
    adapter: AdapterRegistration {
        adapter_type: args.adapter_type.clone(),
        dyad: args.dyad.clone(),
        features: features.iter().map(|f| *f as u16).collect(),
    },
};
```

And for response handling:
```rust
let registration: AdapterRegistrationResponse = response.json().await?;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p river-tui`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/river-tui/Cargo.toml crates/river-tui/src/main.rs
git commit -m "refactor(tui): use registration types from river-protocol"
```

---

### Task 10: Verify full workspace builds and test

**Files:** None (verification only)

- [ ] **Step 1: Build entire workspace**

Run: `cargo build --workspace`
Expected: All crates compile successfully

- [ ] **Step 2: Run tests**

Run: `cargo test --workspace`
Expected: All tests pass

- [ ] **Step 3: Check for unused dependencies**

Run: `cargo +nightly udeps --workspace` (if available, otherwise skip)
Expected: No unused dependencies in river-protocol

- [ ] **Step 4: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix: resolve any remaining build issues"
```

(Only if fixes were needed)

---

## Self-Review Checklist

**Spec coverage:**
- ✓ Create river-protocol crate (Task 1-3)
- ✓ Move identity types from river-adapter (Task 1, 4)
- ✓ Consolidate ProcessEntry/Registry (Task 2, 6, 7)
- ✓ Consolidate registration types (Task 3, 6, 8, 9)
- ✓ Update river-adapter to depend on river-protocol (Task 4)
- ✓ Update river-context (Task 5)
- ✓ Update river-worker (Task 6)
- ✓ Update river-orchestrator (Task 7)
- ✓ Update river-discord (Task 8)
- ✓ Update river-tui (Task 9)

**Placeholder scan:** No TBD/TODO items. All code blocks complete.

**Type consistency:** ProcessEntry, Registry, ModelConfig, WorkerRegistrationResponse used consistently across tasks.

# Phase 0: Extract Crates

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `river-adapter` and extract `river-tools` and `river-db` as standalone crates so the gateway can be restructured without breaking tools, database access, or adapter communication.

**Architecture:** Three new workspace crates. Gateway depends on them. All existing tests continue to pass. river-adapter is new code from the design spec; river-tools and river-db are pure extractions.

**Tech Stack:** Rust workspace crates, serde, async-trait, thiserror

---

## Overview

| Crate | Type | Contents |
|-------|------|----------|
| `river-adapter` | **New** | Adapter types, trait, HttpAdapter, feature flags |
| `river-db` | Extract | Database layer (schema, messages, memories, contexts) |
| `river-tools` | Extract | Tool system (registry, executor, 8 tool modules) |

---

## Dependency Audit (for extractions)

### Tools That Have Gateway Dependencies

| Tool File | Gateway Dependencies | Extraction? |
|-----------|---------------------|-------------|
| `communication.rs` | `conversations::*`, `tokio::sync::mpsc` | ❌ Stays in gateway |
| `memory.rs` | `db::*`, `memory::*` | ❌ Stays in gateway |
| `subagent.rs` | `loop::ModelClient`, `subagent::*` | ❌ Stays in gateway |
| `sync.rs` | `conversations::*`, `db::Database` | ❌ Stays in gateway |
| `scheduling.rs` | None (self-contained) | ✅ Extract |
| `model.rs` | `reqwest` only | ✅ Extract |
| `file.rs` | Filesystem only | ✅ Extract |
| `shell.rs` | Command only | ✅ Extract |
| `web.rs` | `reqwest` only | ✅ Extract |
| `logging.rs` | `std::fs` only | ✅ Extract |

---

## Task 1: Create river-adapter Crate

- [ ] **Step 1: Create crate directory**

```bash
mkdir -p crates/river-adapter/src
```

- [ ] **Step 2: Write Cargo.toml**

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

- [ ] **Step 3: Add to workspace**

In root `Cargo.toml`, add to workspace members:
```toml
members = [
    # ... existing ...
    "crates/river-adapter",
]
```

- [ ] **Step 4: Create types.rs**

Write `crates/river-adapter/src/types.rs`:
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

- [ ] **Step 5: Create capabilities.rs**

Write `crates/river-adapter/src/capabilities.rs`:
```rust
//! Feature flags for adapter capabilities

use serde::{Deserialize, Serialize};

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

- [ ] **Step 6: Create registration.rs**

Write `crates/river-adapter/src/registration.rs`:
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

- [ ] **Step 7: Create error.rs**

Write `crates/river-adapter/src/error.rs`:
```rust
//! Adapter error types

use crate::capabilities::Feature;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Adapter not found: {0}")]
    NotFound(String),

    #[error("Feature not supported: {0:?}")]
    FeatureNotSupported(Feature),

    #[error("Adapter error: {0}")]
    Other(String),
}
```

- [ ] **Step 8: Create traits.rs**

Write `crates/river-adapter/src/traits.rs`:
```rust
//! Adapter trait for gateway-side abstraction

use crate::{AdapterError, Feature, IncomingEvent, SendRequest, SendResponse};
use async_trait::async_trait;

#[async_trait]
pub trait Adapter: Send + Sync {
    /// Adapter name
    fn name(&self) -> &str;

    /// Check if feature is supported
    fn supports(&self, feature: &Feature) -> bool;

    /// Send a message
    async fn send(&self, request: SendRequest) -> Result<SendResponse, AdapterError>;

    /// Read channel history (if supported)
    async fn read_history(&self, channel: &str, limit: usize) -> Result<Vec<IncomingEvent>, AdapterError>;

    /// Health check
    async fn health(&self) -> Result<bool, AdapterError>;
}
```

- [ ] **Step 9: Create http.rs**

Write `crates/river-adapter/src/http.rs`:
```rust
//! HTTP-based adapter client

use crate::{Adapter, AdapterError, AdapterInfo, Feature, IncomingEvent, SendRequest, SendResponse};
use async_trait::async_trait;

/// Gateway-side client for external adapters
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
        let response = self.client
            .post(&url)
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
        let response = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    async fn health(&self) -> Result<bool, AdapterError> {
        let url = format!("{}/health", self.info.url);
        let response: serde_json::Value = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;
        Ok(response.get("healthy").and_then(|v| v.as_bool()).unwrap_or(false))
    }
}
```

- [ ] **Step 10: Create lib.rs**

Write `crates/river-adapter/src/lib.rs`:
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

- [ ] **Step 11: Write basic tests**

Write `crates/river-adapter/src/lib.rs` tests section:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_event_type_serialization() {
        let event_type = EventType::MessageCreate;
        let json = serde_json::to_string(&event_type).unwrap();
        assert!(json.contains("MessageCreate"));
    }

    #[test]
    fn test_adapter_info_creation() {
        let info = AdapterInfo {
            name: "test".into(),
            version: "1.0.0".into(),
            url: "http://localhost:3000".into(),
            features: HashSet::from([Feature::ReadHistory, Feature::Reactions]),
            metadata: serde_json::json!({}),
        };
        assert_eq!(info.name, "test");
        assert!(info.features.contains(&Feature::ReadHistory));
    }

    #[test]
    fn test_send_request_serialization() {
        let request = SendRequest {
            channel: "123".into(),
            content: "Hello".into(),
            options: SendOptions::default(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"channel\":\"123\""));
        assert!(json.contains("\"content\":\"Hello\""));
    }
}
```

- [ ] **Step 12: Verify compilation**

```bash
cargo check -p river-adapter
cargo test -p river-adapter
```

- [ ] **Step 13: Commit**

```bash
git add crates/river-adapter/ Cargo.toml
git commit -m "feat: create river-adapter crate with shared types and trait"
```

---

## Task 2: Create river-db Crate

- [ ] **Step 1: Create crate directory and Cargo.toml**

```bash
mkdir -p crates/river-db/src
```

Write `crates/river-db/Cargo.toml`:
```toml
[package]
name = "river-db"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
river-core = { path = "../river-core" }
rusqlite.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
chrono.workspace = true
```

- [ ] **Step 2: Add to workspace**

In root `Cargo.toml`, add to workspace members:
```toml
"crates/river-db",
```

- [ ] **Step 3: Copy db module files**

```bash
cp crates/river-gateway/src/db/schema.rs crates/river-db/src/
cp crates/river-gateway/src/db/messages.rs crates/river-db/src/
cp crates/river-gateway/src/db/memories.rs crates/river-db/src/
cp crates/river-gateway/src/db/contexts.rs crates/river-db/src/
```

- [ ] **Step 4: Write river-db/src/lib.rs**

```rust
//! River Database — SQLite storage layer

pub mod schema;
pub mod messages;
pub mod memories;
pub mod contexts;

pub use schema::Database;
pub use messages::{Message, MessageRole};
pub use memories::Memory;
pub use contexts::ContextRecord;
```

- [ ] **Step 5: Fix import paths in copied files**

In each copied file, replace `use crate::` and `use super::` references:
- `crate::db::Database` → `crate::schema::Database` or `super::schema::Database`
- Any `river_core::` imports should stay as-is

- [ ] **Step 6: Verify river-db compiles**

```bash
cargo check -p river-db
cargo test -p river-db
```

- [ ] **Step 7: Commit**

```bash
git add crates/river-db/ Cargo.toml
git commit -m "feat: extract river-db crate from gateway"
```

---

## Task 3: Wire Gateway to river-db

- [ ] **Step 1: Add river-db dependency to gateway**

In `crates/river-gateway/Cargo.toml`:
```toml
river-db = { path = "../river-db" }
```

- [ ] **Step 2: Update gateway's db/mod.rs to re-export**

Replace `crates/river-gateway/src/db/mod.rs` contents:
```rust
//! Database layer — re-exported from river-db
pub use river_db::*;
```

- [ ] **Step 3: Remove old db source files from gateway**

```bash
rm crates/river-gateway/src/db/schema.rs
rm crates/river-gateway/src/db/messages.rs
rm crates/river-gateway/src/db/memories.rs
rm crates/river-gateway/src/db/contexts.rs
```

- [ ] **Step 4: Fix any compilation errors**

```bash
cargo check -p river-gateway
```

Fix any remaining import path issues. Common fixes:
- `crate::db::Database` still works (through re-export)
- Ensure `MessageRole`, `Message`, `Memory` are accessible

- [ ] **Step 5: Run all gateway tests**

```bash
cargo test -p river-gateway
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(gateway): use river-db crate, remove inline db modules"
```

---

## Task 4: Create river-tools Crate (Core)

- [ ] **Step 1: Create crate directory and Cargo.toml**

```bash
mkdir -p crates/river-tools/src
```

Write `crates/river-tools/Cargo.toml`:
```toml
[package]
name = "river-tools"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
river-core = { path = "../river-core" }
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
reqwest.workspace = true
tokio.workspace = true
glob.workspace = true
regex.workspace = true
chrono.workspace = true

[dev-dependencies]
tempfile = "3.10"
```

- [ ] **Step 2: Add to workspace**

In root `Cargo.toml`, add to workspace members:
```toml
"crates/river-tools",
```

- [ ] **Step 3: Copy registry and executor**

```bash
cp crates/river-gateway/src/tools/registry.rs crates/river-tools/src/
cp crates/river-gateway/src/tools/executor.rs crates/river-tools/src/
```

- [ ] **Step 4: Write river-tools/src/lib.rs with just registry + executor**

```rust
//! River Tools — Tool system for agent capabilities

pub mod registry;
pub mod executor;

pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::{ToolExecutor, ToolCall, ToolCallResponse};
```

- [ ] **Step 5: Fix executor imports**

In `crates/river-tools/src/executor.rs`:
- Remove `use crate::metrics::AgentMetrics;` and the metrics field/logic (metrics stays in gateway)
- Change `use super::{ToolRegistry, ToolResult, ToolSchema};` to `use crate::registry::{ToolRegistry, ToolResult, ToolSchema};`

The executor in river-tools is the pure version. Gateway wraps it with metrics.

- [ ] **Step 6: Fix registry imports**

In `crates/river-tools/src/registry.rs`:
- `use river_core::RiverError;` stays as-is

- [ ] **Step 7: Verify compilation**

```bash
cargo check -p river-tools
cargo test -p river-tools
```

- [ ] **Step 8: Commit**

```bash
git add crates/river-tools/ Cargo.toml
git commit -m "feat: extract river-tools crate (registry + executor)"
```

---

## Task 5: Copy Self-Contained Tools to river-tools

- [ ] **Step 1: Copy tool files**

```bash
cp crates/river-gateway/src/tools/file.rs crates/river-tools/src/
cp crates/river-gateway/src/tools/shell.rs crates/river-tools/src/
cp crates/river-gateway/src/tools/web.rs crates/river-tools/src/
cp crates/river-gateway/src/tools/logging.rs crates/river-tools/src/
cp crates/river-gateway/src/tools/model.rs crates/river-tools/src/
cp crates/river-gateway/src/tools/scheduling.rs crates/river-tools/src/
```

- [ ] **Step 2: Update river-tools/src/lib.rs with all modules**

```rust
//! River Tools — Tool system for agent capabilities

pub mod registry;
pub mod executor;
pub mod file;
pub mod shell;
pub mod web;
pub mod logging;
pub mod model;
pub mod scheduling;

pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::{ToolExecutor, ToolCall, ToolCallResponse};
pub use file::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool};
pub use shell::BashTool;
pub use web::{WebFetchTool, WebSearchTool};
pub use model::{ModelManagerConfig, ModelManagerState, RequestModelTool, ReleaseModelTool, SwitchModelTool};
pub use scheduling::{ContextRotation, HeartbeatScheduler, RotateContextTool, ScheduleHeartbeatTool};
pub use logging::LogReadTool;
```

- [ ] **Step 3: Fix all imports in copied files**

Each file: replace `use super::{Tool, ToolResult};` with `use crate::registry::{Tool, ToolResult};`
Replace `use crate::tools::` with `use crate::`

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p river-tools
cargo test -p river-tools
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tools): add file, shell, web, logging, model, scheduling tools"
```

---

## Task 6: Wire Gateway to river-tools

- [ ] **Step 1: Add river-tools dependency to gateway**

In `crates/river-gateway/Cargo.toml`:
```toml
river-tools = { path = "../river-tools" }
```

- [ ] **Step 2: Update gateway's tools/mod.rs**

Replace with re-exports from river-tools + local gateway-specific tools:

```rust
//! Tool system — re-exports from river-tools + gateway-specific tools

// Gateway-specific tools (depend on gateway internals)
mod communication;
mod memory;
mod subagent;
mod sync;

// Re-export everything from river-tools
pub use river_tools::{
    Tool, ToolRegistry, ToolSchema, ToolResult,
    ToolExecutor, ToolCall, ToolCallResponse,
    ReadTool, WriteTool, EditTool, GlobTool, GrepTool,
    BashTool,
    WebFetchTool, WebSearchTool,
    ModelManagerConfig, ModelManagerState, RequestModelTool, ReleaseModelTool, SwitchModelTool,
    ContextRotation, HeartbeatScheduler, RotateContextTool, ScheduleHeartbeatTool,
    LogReadTool,
};

// Re-export gateway-specific tools
pub use communication::{
    AdapterConfig, AdapterRegistry, SendMessageTool, ListAdaptersTool, ContextStatusTool,
    ReadChannelTool
};
pub use sync::SyncConversationTool;
pub use memory::{EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool};
pub use subagent::{
    SpawnSubagentTool, ListSubagentsTool, SubagentStatusTool, StopSubagentTool,
    InternalSendTool, InternalReceiveTool, WaitForSubagentTool
};
```

- [ ] **Step 3: Remove old tool files from gateway that moved**

```bash
rm crates/river-gateway/src/tools/registry.rs
rm crates/river-gateway/src/tools/executor.rs
rm crates/river-gateway/src/tools/file.rs
rm crates/river-gateway/src/tools/shell.rs
rm crates/river-gateway/src/tools/web.rs
rm crates/river-gateway/src/tools/logging.rs
rm crates/river-gateway/src/tools/model.rs
rm crates/river-gateway/src/tools/scheduling.rs
```

- [ ] **Step 4: Update gateway tools that import from super**

In `communication.rs`, `memory.rs`, `subagent.rs`, `sync.rs`:
Replace `use super::{Tool, ToolResult};` with `use river_tools::{Tool, ToolResult};`

- [ ] **Step 5: Handle the metrics wrapper**

The gateway's `ToolExecutor` usage had `.with_metrics()`. Create a thin wrapper in `crates/river-gateway/src/state.rs` or inline in server:

```rust
// In state.rs, the ToolExecutor from river-tools is used directly.
// Metrics tracking moves to the agent loop (increment on tool call).
```

Or add a `MetricsToolExecutor` wrapper in the gateway that delegates to `river_tools::ToolExecutor`.

- [ ] **Step 6: Fix compilation**

```bash
cargo check -p river-gateway
```

Iterate on import fixes until it compiles.

- [ ] **Step 7: Run all tests**

```bash
cargo test -p river-gateway
```

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(gateway): use river-tools crate, keep gateway-specific tools local"
```

---

## Task 7: Final Verification

- [ ] **Step 1: Check river-discord compilation**

```bash
cargo check -p river-discord
```

river-discord should not depend on gateway tools directly. If it does, fix imports.

- [ ] **Step 2: Run all workspace tests**

```bash
cargo test
```

- [ ] **Step 3: Build all binaries**

```bash
cargo build
```

- [ ] **Step 4: Verify crate structure**

```bash
ls crates/
# Should show: river-adapter, river-core, river-db, river-discord, river-gateway, river-migrate, river-orchestrator, river-tools
```

- [ ] **Step 5: Commit final state**

```bash
git add -A
git commit -m "test: verify all crates compile after extraction"
```

---

## Summary

Phase 0 creates/extracts three crates:

| Crate | Type | Lines (est.) | Contents |
|-------|------|--------------|----------|
| `river-adapter` | New | ~300 | Types, trait, HttpAdapter, feature flags |
| `river-db` | Extract | ~900 | Schema, messages, memories, contexts |
| `river-tools` | Extract | ~3,400 | Registry, executor, 8 tool modules |

Gateway-specific tools (`communication`, `memory`, `subagent`, `sync`) stay in gateway because they depend on gateway internals.

**Total: 7 tasks, ~50 steps. Creates 3 crates, no functional changes.**

---

## Related Documents

- `docs/specs/adapter-framework-design.md` — Adapter types and trait design
- `docs/superpowers/plans/2026-03-23-plan-phase0.5-discord-refactor.md` — Next phase (uses river-adapter)
- `docs/specs/gateway-restructure-meta-plan.md` — Overall restructure plan

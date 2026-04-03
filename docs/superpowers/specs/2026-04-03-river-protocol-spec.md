# river-protocol — Design Spec

> Foundational crate for all shared wire protocol types
>
> Authors: Cass, Claude
> Date: 2026-04-03

## Overview

`river-protocol` is a new foundational crate that consolidates all shared types used across River Engine crates. It has no river-* dependencies and serves as the single source of truth for identity types, registry types, model configuration, and registration protocols.

## Goals

1. **Eliminate type duplication** — `ProcessEntry` and `Registry` are currently duplicated between `river-worker` and `river-orchestrator`
2. **Consolidate registration protocols** — Worker and adapter registration types are defined separately in multiple crates
3. **Clean dependency graph** — `river-adapter` should only be for building adapters, not a shared types library

## Dependency Graph

```
river-snowflake (standalone)

river-protocol (standalone - ALL shared types)
        │
        ├── river-adapter (adapter traits/features)
        ├── river-context (context assembly)
        ├── river-orchestrator
        ├── river-worker
        ├── river-discord
        └── river-tui
```

`river-protocol` has zero river-* dependencies. All other crates depend on it for shared types.

## Module Structure

```
river-protocol/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── identity.rs      # Baton, Side, Ground, Channel, Author, Attachment
    ├── registry.rs      # ProcessEntry, Registry
    ├── model.rs         # ModelConfig
    └── registration.rs  # Worker/Adapter registration request/response types
```

## Types

### identity.rs

Moved from `river-adapter/src/author.rs`:

```rust
/// Which side of the dyad.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

/// Current role assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Baton {
    Actor,
    Spectator,
}

/// Ground state for a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ground {
    pub channel: Channel,
    pub adapter: String,
}

/// Channel identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Channel {
    pub adapter: String,
    pub id: String,
    pub name: Option<String>,
}

/// Message author.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub bot: bool,
}

/// File attachment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub url: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}
```

### registry.rs

Consolidated from `river-worker/src/state.rs` and `river-orchestrator/src/registry.rs`:

```rust
/// Process entry in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
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

/// The full registry sent to all processes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    pub processes: Vec<ProcessEntry>,
}
```

Note: Using `#[serde(tag = "type")]` for explicit discrimination instead of `#[serde(untagged)]` which is fragile.

### model.rs

Moved from `river-worker/src/config.rs`:

```rust
/// Model configuration from orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,
    pub context_limit: usize,
}
```

### registration.rs

Consolidated from `river-worker/src/config.rs`, `river-discord/src/main.rs`, and `river-tui/src/main.rs`:

```rust
// === Worker Registration ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegistration {
    pub dyad: String,
    pub side: Side,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerRegistrationRequest {
    pub endpoint: String,
    pub worker: WorkerRegistration,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterRegistration {
    #[serde(rename = "type")]
    pub adapter_type: String,
    pub dyad: String,
    pub features: Vec<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdapterRegistrationRequest {
    pub endpoint: String,
    pub adapter: AdapterRegistration,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdapterRegistrationResponse {
    pub accepted: bool,
    pub config: serde_json::Value,  // Adapter-specific config
    pub worker_endpoint: String,
}
```

## Public Exports

```rust
// lib.rs
pub use identity::{Attachment, Author, Baton, Channel, Ground, Side};
pub use model::ModelConfig;
pub use registry::{ProcessEntry, Registry};
pub use registration::{
    AdapterRegistration, AdapterRegistrationRequest, AdapterRegistrationResponse,
    WorkerRegistration, WorkerRegistrationRequest, WorkerRegistrationResponse,
};
```

## Migration Plan

### 1. Create river-protocol crate

New crate with the module structure above.

### 2. Update river-adapter

- Remove identity types from `src/author.rs`
- Add dependency on `river-protocol`
- Re-export identity types for backward compatibility (temporary)

### 3. Update river-orchestrator

- Remove local `ProcessEntry` and `Registry` definitions
- Import from `river-protocol`

### 4. Update river-worker

- Remove local `ProcessEntry`, `Registry`, `ModelConfig`, registration types
- Import from `river-protocol`

### 5. Update river-discord

- Remove local registration types
- Import from `river-protocol`

### 6. Update river-tui

- Remove local registration types
- Import from `river-protocol`

### 7. Update river-context

- Add dependency on `river-protocol` for `Channel` type if needed

### 8. Clean up re-exports

- Remove temporary re-exports from `river-adapter` once all consumers migrated

## Dependencies

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

## Testing

Unit tests for serde round-trips on all types, especially `ProcessEntry` with the new tagged enum format.

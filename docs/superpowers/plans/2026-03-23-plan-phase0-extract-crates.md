# Phase 0: Extract Crates

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `river-tools` and `river-db` as standalone crates so the gateway can be restructured without breaking tools or database access.

**Architecture:** Two new workspace crates. Gateway depends on them. All existing tests continue to pass. No functional changes — pure extraction.

**Tech Stack:** Rust workspace crates, no new dependencies

---

## Dependency Audit

Before extracting, we must understand what tools depend on from gateway internals.

### Tools That Have Gateway Dependencies

| Tool File | Lines | Gateway Dependencies |
|-----------|-------|---------------------|
| `communication.rs` | 672 | `conversations::{Author, Message, WriteOp}`, `tokio::sync::mpsc` (for write ops) |
| `memory.rs` | 334 | `db::{Database, Memory}`, `memory::{EmbeddingClient, MemorySearcher}` |
| `subagent.rs` | 696 | `loop::ModelClient`, `subagent::*` |
| `scheduling.rs` | 369 | None (self-contained atomics) |
| `model.rs` | 422 | `reqwest` for orchestrator HTTP calls |
| `sync.rs` | 217 | `conversations::*`, `db::Database` |
| `file.rs` | 857 | None (filesystem only) |
| `shell.rs` | 304 | None (Command only) |
| `web.rs` | 401 | `reqwest` only |
| `logging.rs` | 233 | `std::fs` only |

### Extraction Strategy

**`river-tools`** gets: `registry.rs`, `executor.rs`, `file.rs`, `shell.rs`, `web.rs`, `logging.rs`, `model.rs`, `scheduling.rs` (the self-contained ones).

**Tools that stay in gateway** (for now): `communication.rs`, `memory.rs`, `subagent.rs`, `sync.rs` — these depend on gateway-specific types. They register into the `river-tools` registry but live in the gateway.

**`river-db`** gets: `db/schema.rs`, `db/messages.rs`, `db/memories.rs`, `db/contexts.rs`, `db/mod.rs`.

---

## File Structure

**New files:**
- `crates/river-tools/Cargo.toml`
- `crates/river-tools/src/lib.rs`
- `crates/river-tools/src/registry.rs` — from `gateway/src/tools/registry.rs`
- `crates/river-tools/src/executor.rs` — from `gateway/src/tools/executor.rs`
- `crates/river-tools/src/file.rs` — from `gateway/src/tools/file.rs`
- `crates/river-tools/src/shell.rs` — from `gateway/src/tools/shell.rs`
- `crates/river-tools/src/web.rs` — from `gateway/src/tools/web.rs`
- `crates/river-tools/src/logging.rs` — from `gateway/src/tools/logging.rs`
- `crates/river-tools/src/model.rs` — from `gateway/src/tools/model.rs`
- `crates/river-tools/src/scheduling.rs` — from `gateway/src/tools/scheduling.rs`
- `crates/river-db/Cargo.toml`
- `crates/river-db/src/lib.rs`
- `crates/river-db/src/schema.rs` — from `gateway/src/db/schema.rs`
- `crates/river-db/src/messages.rs` — from `gateway/src/db/messages.rs`
- `crates/river-db/src/memories.rs` — from `gateway/src/db/memories.rs`
- `crates/river-db/src/contexts.rs` — from `gateway/src/db/contexts.rs`

**Modified files:**
- `Cargo.toml` (workspace members)
- `crates/river-gateway/Cargo.toml` (add river-tools, river-db deps)
- `crates/river-gateway/src/lib.rs` (remove db module, update tool imports)
- `crates/river-gateway/src/tools/mod.rs` (re-export from river-tools + local tools)
- `crates/river-gateway/src/state.rs` (use river_db::Database)
- All files that import `crate::db::*` or `crate::tools::*`

---

## Task 1: Create river-db Crate

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

- [ ] **Step 2: Copy db module files**

```bash
cp crates/river-gateway/src/db/schema.rs crates/river-db/src/
cp crates/river-gateway/src/db/messages.rs crates/river-db/src/
cp crates/river-gateway/src/db/memories.rs crates/river-db/src/
cp crates/river-gateway/src/db/contexts.rs crates/river-db/src/
```

- [ ] **Step 3: Write river-db/src/lib.rs**

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

- [ ] **Step 4: Fix import paths in copied files**

In each copied file, replace `use crate::` and `use super::` references:
- `crate::db::Database` → `crate::schema::Database` or `super::schema::Database`
- Any `river_core::` imports should stay as-is

- [ ] **Step 5: Verify river-db compiles**

```bash
cargo check -p river-db
```

- [ ] **Step 6: Commit**

```bash
git add crates/river-db/
git commit -m "feat: extract river-db crate from gateway"
```

---

## Task 2: Wire Gateway to river-db

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

- [ ] **Step 6: Run all tests**

```bash
cargo test
```

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(gateway): use river-db crate, remove inline db modules"
```

---

## Task 3: Create river-tools Crate (Core)

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

- [ ] **Step 2: Copy registry and executor**

```bash
cp crates/river-gateway/src/tools/registry.rs crates/river-tools/src/
cp crates/river-gateway/src/tools/executor.rs crates/river-tools/src/
```

- [ ] **Step 3: Write river-tools/src/lib.rs with just registry + executor**

```rust
//! River Tools — Tool system for agent capabilities

pub mod registry;
pub mod executor;

pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::{ToolExecutor, ToolCall, ToolCallResponse};
```

- [ ] **Step 4: Fix executor imports**

In `crates/river-tools/src/executor.rs`:
- Remove `use crate::metrics::AgentMetrics;` and the metrics field/logic (metrics stays in gateway)
- Change `use super::{ToolRegistry, ToolResult, ToolSchema};` to `use crate::registry::{ToolRegistry, ToolResult, ToolSchema};`

The executor in river-tools is the pure version. Gateway wraps it with metrics.

- [ ] **Step 5: Fix registry imports**

In `crates/river-tools/src/registry.rs`:
- `use river_core::RiverError;` stays as-is

- [ ] **Step 6: Verify compilation**

```bash
cargo check -p river-tools
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p river-tools
```

- [ ] **Step 8: Commit**

```bash
git add crates/river-tools/
git commit -m "feat: extract river-tools crate (registry + executor)"
```

---

## Task 4: Copy Self-Contained Tools to river-tools

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
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p river-tools
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(tools): add file, shell, web, logging, model, scheduling tools"
```

---

## Task 5: Wire Gateway to river-tools

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
cargo test
```

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(gateway): use river-tools crate, keep gateway-specific tools local"
```

---

## Task 6: Verify Discord Adapter Still Works

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

- [ ] **Step 4: Commit final state**

```bash
git add -A
git commit -m "test: verify all crates compile after extraction"
```

---

## Summary

Phase 0 extracts two crates:
1. **`river-db`** — Database layer (schema, messages, memories, contexts). ~878 lines.
2. **`river-tools`** — Tool system (registry, executor, 8 tool modules). ~3,400 lines.

Gateway-specific tools (`communication`, `memory`, `subagent`, `sync`) stay in gateway because they depend on gateway internals.

Total: 6 tasks, ~40 steps. No functional changes — pure extraction.

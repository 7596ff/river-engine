# Tool Consolidation: Delete river-tools, Reorganize Gateway Tools

**Date:** 2026-04-30
**Status:** Draft
**Authors:** Cass, Iris

---

## 1. Summary

Delete the `river-tools` crate and consolidate all tool code into `river-gateway/src/tools/`. The extraction created a split between "pure" tools (in `river-tools`) and "effectful" tools (in `river-gateway/src/tools/`) that serves no consumer — `river-tools` is only used by `river-gateway`, and the gateway re-exports everything anyway.

While consolidating, reorganize the tools directory so each file represents one concern. Two files need splitting: `scheduling.rs` (context rotation and heartbeat are unrelated) and `communication.rs` (adapter infrastructure, communication tools, and context status are three concerns in one file).

---

## 2. Current State

### river-tools crate (2,969 lines)

| File | Tools | Lines |
|------|-------|-------|
| `registry.rs` | Tool trait, ToolRegistry, ToolSchema, ToolResult | 199 |
| `executor.rs` | ToolExecutor, ToolCall, ToolCallResponse | 165 |
| `file.rs` | read, write, edit, glob, grep | 857 |
| `shell.rs` | bash | 304 |
| `web.rs` | webfetch, websearch | 401 |
| `model.rs` | request_model, release_model, switch_model + shared state | 422 |
| `scheduling.rs` | rotate_context, schedule_heartbeat + shared state | 369 |
| `logging.rs` | log_read | 233 |
| `lib.rs` | re-exports | 19 |

### river-gateway/src/tools/ (2,423 lines)

| File | Tools | Lines |
|------|-------|-------|
| `mod.rs` | re-exports from river-tools + gateway tools | 31 |
| `communication.rs` | send_message, speak, typing, switch_channel, list_adapters, read_channel, context_status + AdapterRegistry + send_to_adapter() | 1,144 |
| `memory.rs` | embed, memory_search, memory_delete, memory_delete_by_source | 334 |
| `subagent.rs` | spawn, list, status, stop, internal_send, internal_receive, wait_for | 696 |
| `sync.rs` | sync_conversation | 218 |

### Consumers

`river-tools` is consumed only by `river-gateway`. No other crate depends on it. All gateway code imports via `crate::tools::*`.

---

## 3. Target State

### Delete

- `crates/river-tools/` — entire crate removed
- `river-tools` entry in workspace `Cargo.toml`
- `river-tools` dependency in `river-gateway/Cargo.toml`

### Target tools/ directory

```
river-gateway/src/tools/
├── mod.rs              # submodule declarations and re-exports
├── adapters.rs         # AdapterRegistry, AdapterConfig, send_to_adapter(), ListAdaptersTool
├── communication.rs    # SendMessageTool, SpeakTool, SwitchChannelTool, TypingTool, ReadChannelTool
├── context.rs          # RotateContextTool, ContextStatusTool, ContextRotation
├── executor.rs         # ToolExecutor, ToolCall, ToolCallResponse
├── file.rs             # ReadTool, WriteTool, EditTool, GlobTool, GrepTool
├── heartbeat.rs        # ScheduleHeartbeatTool, HeartbeatScheduler
├── logging.rs          # LogReadTool
├── memory.rs           # EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool
├── model.rs            # RequestModelTool, ReleaseModelTool, SwitchModelTool, ModelManagerConfig, ModelManagerState
├── registry.rs         # Tool trait, ToolRegistry, ToolSchema, ToolResult
├── shell.rs            # BashTool
├── subagent.rs         # SpawnSubagentTool, ListSubagentsTool, SubagentStatusTool, StopSubagentTool, InternalSendTool, InternalReceiveTool, WaitForSubagentTool
├── sync.rs             # SyncConversationTool
└── web.rs              # WebFetchTool, WebSearchTool
```

14 files, one concern each.

### Concern splits

**`scheduling.rs` splits into:**
- `context.rs` — `RotateContextTool` + `ContextRotation` state struct. Also receives `ContextStatusTool` from communication.rs since it queries context window state.
- `heartbeat.rs` — `ScheduleHeartbeatTool` + `HeartbeatScheduler` state struct.

**`communication.rs` splits into:**
- `adapters.rs` — `AdapterRegistry`, `AdapterConfig`, `send_to_adapter()` shared function, `ListAdaptersTool`. Infrastructure that communication tools depend on.
- `communication.rs` — `SendMessageTool`, `SpeakTool`, `SwitchChannelTool`, `TypingTool`, `ReadChannelTool`. All tools that send/receive through adapters.
- `ContextStatusTool` moves to `context.rs`.

---

## 4. Import Changes

### In tools/ files (moved from river-tools)

All `use river_core::RiverError` imports stay the same — `river-gateway` already depends on `river-core`.

No import changes needed for files moving from `river-tools` — they already use `crate::` style internally within `river-tools`. These become `crate::tools::` references or intra-module references.

### In tools/ files (already in gateway)

Replace:
```rust
use river_tools::{Tool, ToolResult};
```

With:
```rust
use crate::tools::{Tool, ToolResult};
// or just: use super::{Tool, ToolResult};
```

Affected files: `communication.rs`, `memory.rs`, `subagent.rs`, `sync.rs`.

### In rest of gateway

No changes. Everything already imports `use crate::tools::*`.

### Dependencies

No new dependencies needed. `glob` and `regex` (used by GlobTool and GrepTool) are already in `river-gateway/Cargo.toml`.

---

## 5. What Does Not Change

- The `Tool` trait API
- The `ToolRegistry` and `ToolExecutor` API
- Any tool's behavior or parameters
- Any import outside `tools/` (`use crate::tools::*` all stays the same)
- The redis tool files (`redis/working.rs`, etc.) — already structured correctly, already import from `crate::tools`

---

## 6. Testing

Run `cargo test` after consolidation. All existing tests move with their source files. No new tests needed — this is a mechanical refactor with no behavior changes.

Verify:
- `cargo build` succeeds
- `cargo test` passes
- `river-tools` crate is gone from workspace
- No `river_tools` string appears anywhere in the codebase

---

## 7. Risk

Essentially zero. This is a mechanical move. The public API doesn't change, the trait doesn't change, and all consumers already import through `crate::tools`. The concern splits within `communication.rs` and `scheduling.rs` are also mechanical — moving code between files with updated imports.

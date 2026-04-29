# Remove AgentLoop

**Date:** 2026-04-29
**Status:** Approved

## Problem

The `loop/` module is deprecated. The entire module is marked `#[deprecated]`, generating 159 compiler warnings. The replacement architecture — `AgentTask` + `Coordinator` + `SpectatorTask` — is fully operational. But `loop/` still exports shared types used across the crate:

- `ModelClient`, `ModelResponse`, `Usage`, `Provider` (model/HTTP types)
- `ChatMessage` (OpenAI-compatible message format)
- `ToolCallRequest`, `FunctionCall` (model response types)
- `MessageQueue` (priority queue for incoming messages)
- `ContextBuilder` (message accumulation + context assembly)
- `LoopEvent` (wake trigger enum)
- `ContextFile` (JSONL persistence)
- `LoopState`, `WakeTrigger`, `LoopConfig` (loop state machine types)

The `AgentLoop` struct itself (~900 lines) is dead code — nothing constructs it. The types it re-exports are load-bearing.

## Design

### Delete

| File | ~Lines | Reason |
|------|--------|--------|
| `loop/mod.rs` | 1000 | Dead runtime. Nothing constructs `AgentLoop`. |
| `loop/context.rs` | 500 | `ContextBuilder` replaced. Only live consumer (`SubagentRunner`) switches to inline `Vec<ChatMessage>`. |
| `loop/persistence.rs` | 300 | `ContextFile` only used by `AgentLoop`. |
| `loop/state.rs` | 175 | `LoopEvent`, `LoopState`, `WakeTrigger` are dead. `ToolCallRequest`/`FunctionCall` survive and move. |
| `loop/queue.rs` | 260 | File moves, not deleted. |
| `loop/model.rs` | 775 | File moves, not deleted. |

The `loop/` directory is deleted entirely.

### Relocate

| Type(s) | From | To | Rationale |
|---------|------|----|-----------|
| `ChatMessage` | `loop/context.rs` | `model/types.rs` | Core message format used by agent, spectator, subagent |
| `ToolCallRequest`, `FunctionCall` | `loop/state.rs` | `model/types.rs` | Model response types |
| `ModelClient`, `ModelResponse`, `Usage`, `Provider` | `loop/model.rs` | `model/client.rs` | HTTP client for LLM APIs |
| `MessageQueue` | `loop/queue.rs` | `queue.rs` (top-level module) | Coordination type, not model-related |

New module structure:

```
model/
  mod.rs        — pub use types + client
  types.rs      — ChatMessage, ToolCallRequest, FunctionCall
  client.rs     — ModelClient, ModelResponse, Usage, Provider (+ all private API types)
queue.rs        — MessageQueue (moved from loop/queue.rs)
```

### SubagentRunner change

`subagent/runner.rs` currently holds a `ContextBuilder` for message accumulation. Replace with inline fields:

```rust
// Before
context: ContextBuilder,

// After
messages: Vec<ChatMessage>,
tools: Vec<ToolSchema>,
```

Seven `ContextBuilder` method calls to replace:
- `self.context.clear()` → `self.messages.clear()`
- `self.context.add_message(msg)` → `self.messages.push(msg)`
- `self.context.set_tools(schemas)` → `self.tools = schemas`
- `self.context.messages()` → `&self.messages`
- `self.context.tools()` → `&self.tools`
- `self.context.add_assistant_response(content, tool_calls)` → `self.messages.push(ChatMessage::assistant(content, tool_calls))`
- `self.context.add_tool_results(results, incoming)` → inline loop: push `ChatMessage::tool()` per result, then system message for incoming if non-empty (~15 lines)

### Dead plumbing removal

Removing `LoopEvent` forces removal of:

- `loop_tx: mpsc::Sender<LoopEvent>` from `AppState` and its constructor
- The `LoopEvent::InboxUpdate` send in `api/routes.rs` `handle_incoming`
- The `(loop_tx, _loop_rx) = mpsc::channel(256)` in `server.rs`

This is mechanical — deleting dead code that references a deleted type. The send was going nowhere (receiver immediately dropped in `server.rs`).

**Note:** This is a behavioral bugfix. Currently `handle_incoming` always returns 503 because `loop_tx.send()` fails (receiver dropped). After removing the send, the endpoint will correctly return 200. The inbox file write was always succeeding; only the dead wake signal was "failing."

### Import updates

Every `use crate::r#loop::X` becomes `use crate::model::X` or `use crate::queue::MessageQueue`:

- `lib.rs` — remove `pub mod r#loop;`, add `pub mod model;` and `pub mod queue;`
- `agent/task.rs` — `ModelClient`, `MessageQueue`, `ChatMessage`, `ToolCallRequest`
- `agent/context.rs` — `ChatMessage`
- `spectator/mod.rs` — `ModelClient`, `ChatMessage` (in tests)
- `server.rs` — `MessageQueue`, `ModelClient`
- `state.rs` — `MessageQueue` (remove `LoopEvent`)
- `api/routes.rs` — remove `LoopEvent` import and send call
- `tools/subagent.rs` — `ModelClient`
- `subagent/runner.rs` — `ChatMessage`, `ModelClient`, `ToolCallRequest` (remove `ContextBuilder`)

Within the moved files themselves, internal `use crate::r#loop::` imports must be rewritten:
- `model/client.rs` — `ChatMessage` becomes `use super::types::ChatMessage`, `ToolCallRequest`/`FunctionCall` become `use super::types::{ToolCallRequest, FunctionCall}`

### Not in scope

The following are known dead weight noted during analysis but not addressed here:

- **`AgentMetrics`** — created and read by health endpoint, but `AgentTask` never writes to it. Health endpoint shows stale data (always "Sleeping, 0 turns"). Future work: wire AgentTask to metrics.
- **`HealthPolicy`** — same pattern. Created, read by health endpoint, never updated. Future work: wire AgentTask to policy.
- **`LoopStateLabel`** — vestige of old state machine, used only by metrics. Will become dead once metrics are reworked.
- **`git.rs`** — `GitOps` only imported by `AgentLoop`. Becomes fully dead after this change. Future cleanup.
- **`config.rs`** — empty module (single doc comment). Future cleanup.
- **`Session` / `SessionManager`** in `session/mod.rs` — never used outside own module. Only `PRIMARY_SESSION_ID` is imported. Future cleanup.

### Test impact

- All `loop/` module tests deleted with the module (~49 tests across mod.rs, state.rs, context.rs, model.rs, persistence.rs)
- `api/routes.rs` test helpers that construct `AppState` lose the `loop_tx` parameter
- One behavioral bugfix: `handle_incoming` stops returning 503 (see Dead plumbing removal)
- Build must pass with 0 deprecated warnings from the `loop` module

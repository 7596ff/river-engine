# river-worker Brutal Review

> Reviewer: Claude (no subagents)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-01-worker-design.md

## Spec Completion Assessment

### Module Structure - PASS

| Spec Requirement | Implemented | Notes |
|------------------|-------------|-------|
| main.rs | YES | |
| config.rs | YES | |
| state.rs | YES | |
| loop.rs | YES | Named worker_loop.rs |
| tools.rs | YES | |
| http.rs | YES | |
| llm.rs | YES | |
| persistence.rs | YES | |

### Tools - PASS (17/17)

| Tool | Implemented | Notes |
|------|-------------|-------|
| read | YES | |
| write | YES | With embed notification |
| delete | YES | With embed notification |
| bash | YES | With timeout |
| speak | YES | |
| adapter | YES | |
| switch_channel | YES | |
| sleep | YES | |
| watch | YES | |
| summary | YES | |
| create_move | YES | |
| create_moment | YES | |
| create_flash | YES | |
| request_model | YES | |
| switch_roles | YES | |
| search_embeddings | YES | |
| next_embedding | YES | |

### HTTP Endpoints - PASS

| Endpoint | Implemented | Notes |
|----------|-------------|-------|
| POST /notify | YES | |
| POST /flash | YES | |
| POST /registry | YES | |
| POST /prepare_switch | YES | |
| POST /commit_switch | YES | |
| GET /health | YES | |

### Features - PARTIAL

| Feature | Implemented | Notes |
|---------|-------------|-------|
| Orchestrator registration | YES | |
| Context persistence | YES | JSONL format |
| Tool execution | YES | All 17 tools |
| Context pressure | YES | 80%/95% thresholds |
| Sleep/wake | YES | |
| Role switching | YES | |
| Conversation file format | **NO** | Major missing feature |

## CRITICAL ISSUES

### 1. Conversation file format NOT IMPLEMENTED

**Spec defines an entire conversation file system:**
```
workspace/conversations/{adapter}/{guild_id}-{guild_name}/{channel_id}-{channel_name}.txt
```

With detailed line format:
```
[x] 2026-03-21T14:25:00Z 1234567890 <alice:111> message text
[+] 2026-03-21T14:35:00Z 1234567893 <alice:111> new message
[r] 2026-03-21T14:35:30Z 1234567893
```

**Implementation:** DOES NOT EXIST. The `/notify` handler only updates `pending_notifications` in state. No conversation files are written anywhere.

This is **60+ lines of spec** that's completely unimplemented:
- No conversation file writing
- No read receipt tracking
- No compaction logic
- No line type parsing

**Verdict:** CRITICAL SPEC VIOLATION. A major architectural component is missing.

### 2. No malformed tool call retry logic

**Spec says:**
> **Malformed tool calls:**
> - Retry with backoff: 1 minute, 2 minutes, 5 minutes
> - Inject system message explaining error on each retry
> - After 3 failures: exit with `Error` status

**Implementation:**
```rust
let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
    Ok(v) => v,
    Err(e) => {
        return ToolResult::Error(ToolError::ParseError {
            message: e.to_string(),
        });
    }
};
```

Parse errors just return an error to the LLM. No retry, no backoff, no failure counting.

**Verdict:** SPEC VIOLATION. Model gets unlimited retries with no backoff.

### 3. Tools NOT executed in parallel

**Spec says:**
> // Execute all tool calls in parallel
> let results = execute_tools_parallel(calls, state, config).await;

**Implementation:**
```rust
for call in &calls {
    let result = execute_tool(call, &state, config, &mut generator, client).await;
    // ...
}
```

Sequential execution with a loop. The spec explicitly says parallel.

**Verdict:** SPEC VIOLATION. Affects performance on multi-tool responses.

## IMPORTANT ISSUES

### 4. No "busy" check in prepare_switch

**Spec says:**
> **Behavior:**
> 1. Check not mid-tool-execution
> 2. Check not mid-LLM-call
> 3. Set `switch_pending = true` to block new operations

**Implementation:**
```rust
async fn handle_prepare_switch(...) {
    let mut s = state.write().await;
    if s.switch_pending {
        return Ok(Json(PrepareSwitchResponse {
            ready: false,
            reason: Some("switch_already_pending".into()),
        }));
    }
    s.switch_pending = true;
    Ok(Json(PrepareSwitchResponse { ready: true, reason: None }))
}
```

Only checks if switch is already pending. Doesn't check if mid-tool or mid-LLM.

**Verdict:** Role switches can happen mid-operation, causing inconsistent state.

### 5. Summary tool clears context immediately

**Implementation:**
```rust
if let Some(summary) = summary_text {
    // Clear context file - worker is done with this conversation
    if let Err(e) = clear_context(&context_path) {
        tracing::warn!("Failed to clear context: {}", e);
    }
    return WorkerOutput { ... };
}
```

Context is cleared before orchestrator is notified. If the orchestrator fails to receive the output, the context is lost.

Should: return output → orchestrator confirms → then clear.

### 6. Speak tool doesn't write to conversation file

**Spec says:**
> **Side effects:**
> - Appends to conversation file with `[>]` status
> - Does NOT change `current_channel`

**Implementation:** Just sends to adapter, doesn't write to any file.

### 7. No wait_for_first_notify for actor

**Spec says:**
> 7. If actor: wait for first `/notify` to start loop

**Implementation:**
```rust
async fn wait_for_activation(state: &SharedState) {
    loop {
        let s = state.read().await;
        if !s.pending_notifications.is_empty() || !s.pending_flashes.is_empty() {
            return;
        }
        if !s.sleeping && s.pending_notifications.is_empty() && s.pending_flashes.is_empty() {
            // Wait a bit and check again
        } else if s.sleeping {
            return;  // start_sleeping was true
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

The logic is confusing. If `start_sleeping` is false and no notifications, it just spins forever. Doesn't distinguish actor vs spectator startup behavior.

### 8. Channel switch doesn't use build_context

**Implementation has a TODO:**
```rust
// Handle channel switch - save reordered context
// TODO: Use build_context to properly reorder with new channel at end
if channel_switched {
    if let Err(e) = save_context(&context_path, &messages) {
        ...
    }
}
```

The spec says context should be reordered via river-context when switching channels. This is not implemented.

### 9. LLM client doesn't update after model switch

When `request_model` succeeds, state is updated but `LlmClient` is not:
```rust
// In execute_request_model:
if let Ok(new_config) = serde_json::from_value::<crate::config::ModelConfig>(body.clone()) {
    let mut s = state.write().await;
    s.model_config = new_config;  // State updated
}
// But LlmClient in run_loop still has old config!
```

The loop creates `LlmClient` once at startup. Model switch updates state but not the active client.

### 10. No tracing span for tool execution

Tool calls are executed without tracing spans, making debugging difficult.

## MINOR ISSUES

### 11. Polling instead of async notify

```rust
tokio::time::sleep(Duration::from_millis(100)).await;
```

Both `wait_for_activation` and `sleep_until_wake` poll at 100ms intervals. Should use `tokio::sync::Notify` or similar.

### 12. Side is parsed manually instead of using serde

```rust
let side = match args.side.as_str() {
    "left" => Side::Left,
    "right" => Side::Right,
    _ => { ... }
};
```

Could use clap's value_enum or serde.

### 13. Persistence uses std::fs in async context

```rust
use std::fs::{self, File, OpenOptions};
```

Should use `tokio::fs` for async IO consistency.

### 14. No test for tools

Only `persistence.rs` has tests. No tool tests despite 17 tools and 1000+ lines.

## Code Quality Assessment

### Strengths

1. **All 17 tools implemented** - Full spec coverage for tools
2. **Clean tool error types** - Comprehensive ToolError enum
3. **Good tracing** - Uses tracing throughout
4. **Proper async/await** - tokio patterns correct
5. **Tool definitions** - Well-formatted for LLM consumption
6. **Context persistence tests** - Good roundtrip tests

### Weaknesses

1. **Missing conversation files** - Major architectural gap
2. **Sequential tool execution** - Should be parallel
3. **No retry logic** - Malformed calls not handled per spec
4. **Mixed sync/async IO** - Inconsistent
5. **Light testing** - Only persistence tested
6. **Polling loops** - Should use proper async primitives

## Summary

| Category | Score | Notes |
|----------|-------|-------|
| Spec Completion | 65% | Missing conversation files, retry logic |
| Code Quality | 70% | Clean but incomplete |
| Documentation | 60% | TODOs acknowledged, light comments |
| Testing | 25% | Only persistence tested |

### Blocking Issues

1. **Conversation file format not implemented** - 60+ lines of spec ignored
2. **Sequential tool execution** - Spec requires parallel
3. **No malformed call retry** - No backoff/failure counting
4. **Model switch doesn't update LlmClient** - Switching models is broken

### Recommended Actions

1. Implement conversation file format with all line types
2. Make tool execution parallel with `futures::join_all`
3. Add retry logic with exponential backoff for malformed calls
4. Fix model switch to update LlmClient
5. Check busy state in prepare_switch
6. Use `tokio::sync::Notify` instead of polling loops
7. Add tool tests
8. Use tokio::fs consistently

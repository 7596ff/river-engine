# river-worker Code Review

> Reviewer: Claude (Senior Code Reviewer)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-01-worker-design.md
> Implementation: crates/river-worker/

## Executive Summary

The river-worker crate provides a functional implementation of the agent runtime with the think-act loop. The core architecture is sound and follows the spec's philosophy of "worker as shell." However, there are **significant gaps** in spec compliance, particularly around the conversation file format and malformed response handling. The implementation is approximately **70% complete** relative to the spec.

---

## Spec Compliance Checklist

### Crate Structure

| Requirement | Status | Notes |
|-------------|--------|-------|
| `main.rs` - CLI parsing, startup | PASS | Implemented correctly |
| `config.rs` - WorkerConfig from CLI | PASS | Matches spec |
| `state.rs` - WorkerState | PASS | All fields present |
| `loop.rs` (as `worker_loop.rs`) | PASS | Named differently but functional |
| `tools.rs` - tool implementations | PASS | All 17 tools implemented |
| `http.rs` - axum server | PASS | All endpoints present |
| `llm.rs` - LLM client | PASS | OpenAI-compatible |
| `persistence.rs` - JSONL context | PARTIAL | Missing conversation file format |

### CLI Arguments

| Argument | Status | Notes |
|----------|--------|-------|
| `--orchestrator <URL>` | PASS | |
| `--dyad <NAME>` | PASS | |
| `--side <SIDE>` | PASS | Validates "left"/"right" |
| `--port <PORT>` | PASS | Default 0 for OS-assigned |

### Configuration Types

| Type | Status | Notes |
|------|--------|-------|
| `WorkerConfig` | PASS | All fields present |
| `RegistrationResponse` | PASS | Re-exported from river-protocol |
| `ModelConfig` | PASS | Re-exported from river-protocol |

### Startup Sequence

| Step | Status | Notes |
|------|--------|-------|
| Parse CLI args | PASS | |
| Bind HTTP server | PASS | |
| Register with orchestrator | PASS | |
| Initialize WorkerState | PASS | |
| Load existing context | PASS | |
| Load role definition | PASS | |
| Wait for first notify/flash | PASS | |

### Worker State

| Field | Status | Notes |
|-------|--------|-------|
| `dyad`, `side`, `baton` | PASS | |
| `partner_endpoint` | PASS | |
| `ground`, `workspace` | PASS | |
| `current_channel` | PASS | |
| `watch_list` | PASS | Uses `HashSet<String>` key format |
| `registry` | PASS | |
| `model_config` | PASS | |
| `token_count`, `context_limit` | PASS | |
| `sleeping`, `sleep_until` | PASS | |
| `pending_notifications` | PASS | |
| `pending_flashes` | PASS | |
| `switch_pending` | PASS | |
| `role_content` | PASS | Added for role injection |
| `identity_content` | PASS | Added for identity injection |
| `initial_message` | PASS | Added for session continuity |

### Main Loop Behaviors

| Behavior | Status | Notes |
|----------|--------|-------|
| Build context from workspace | PARTIAL | Uses raw messages, no river-context assembly |
| Check context pressure (80%/95%) | PASS | |
| Inject pending flashes | PASS | |
| Call LLM | PASS | |
| Execute tool calls in parallel | **FAIL** | Executes sequentially (see Critical Issues) |
| Persist after each tool result | PASS | |
| Summary exits loop | PASS | |
| Sleep pauses loop | PASS | |
| Text response -> status message | PASS | |
| Wake on watched channel notification | PASS | |

### Tools (17 total)

| Tool | Status | Notes |
|------|--------|-------|
| `read` | PASS | Line range support included |
| `write` | PASS | Modes: overwrite, append, insert |
| `delete` | PASS | Embed server notification included |
| `bash` | PASS | Timeout, working_directory supported |
| `speak` | PASS | Uses current_channel defaults |
| `adapter` | PASS | Generic OutboundRequest execution |
| `switch_channel` | PASS | |
| `sleep` | PASS | Optional minutes |
| `watch` | PASS | Add/remove channels |
| `summary` | PASS | |
| `create_move` | PASS | Snowflake ID generation |
| `create_moment` | PASS | Snowflake ID generation |
| `create_flash` | PASS | P2P via registry lookup |
| `request_model` | PASS | Orchestrator model switch |
| `switch_roles` | PASS | Orchestrator-mediated |
| `search_embeddings` | PASS | |
| `next_embedding` | PASS | |

### HTTP Endpoints

| Endpoint | Status | Notes |
|----------|--------|-------|
| `POST /notify` | PASS | Event batching, wake on watched |
| `POST /flash` | PASS | Queue + wake |
| `POST /registry` | PASS | Updates local copy |
| `POST /prepare_switch` | PASS | Sets switch_pending flag |
| `POST /commit_switch` | PASS | Swaps baton, returns new value |
| `GET /health` | PASS | Returns `{"status": "ok"}` |

### Error Handling

| Requirement | Status | Notes |
|-------------|--------|-------|
| Standard `ToolError` enum | PASS | All error types present |
| Malformed tool call retry (1m, 2m, 5m) | **FAIL** | Not implemented |
| Exit after 3 malformed failures | **FAIL** | Not implemented |
| Tool execution fails -> return to model | PASS | |
| LLM unreachable -> exit with Error | PASS | |

### Context Persistence

| Requirement | Status | Notes |
|-------------|--------|-------|
| JSONL format | PASS | |
| OpenAI message format | PASS | |
| Persist after tool result | PASS | |
| Persist after model response | PASS | |
| Load on startup | PASS | |

---

## Critical Issues (Must Fix)

### 1. Tool Calls Not Executed in Parallel

**Spec requirement (line 261-262):**
```rust
// Execute all tool calls in parallel
let results = execute_tools_parallel(calls, state, config).await;
```

**Actual implementation (`/home/cassie/river-engine/crates/river-worker/src/worker_loop.rs`, lines 212-245):**
```rust
for call in &calls {
    let result = execute_tool(call, &state, config, &mut generator, client).await;
    // ... process result
}
```

Tools are executed sequentially. This violates the spec and degrades performance when multiple independent tools are called.

**Fix:** Use `futures::future::join_all` or `tokio::join!` to execute tools concurrently.

---

### 2. Conversation File Format Not Implemented

**Spec requirement (lines 574-692):**

The spec defines a comprehensive hybrid conversation file format with:
- Sorted compacted section
- Append-only tail
- Line types: `[x]`, `[>]`, `[ ]`, `[+]`, `[r]`, `[!]`
- Compaction logic
- Specific file paths: `workspace/conversations/{adapter}/{guild_id}-{guild_name}/{channel_id}-{channel_name}.txt`

**Actual implementation:**

The entire conversation file format is **not implemented**. The `/notify` handler queues notifications but never writes to conversation files. The `speak` tool sends messages but never appends `[>]` entries.

This is a **major missing feature** that affects:
- Message persistence
- Read/unread tracking
- Conversation history for the model

---

### 3. Malformed Tool Call Retry Logic Missing

**Spec requirement (lines 807-810):**
```
Malformed tool calls:
- Retry with backoff: 1 minute, 2 minutes, 5 minutes
- Inject system message explaining error on each retry
- After 3 failures: exit with Error status
```

**Actual implementation:**

The code handles `ParseError` for malformed arguments but does not implement:
- Retry with exponential backoff
- Error count tracking
- Injection of explanatory system messages
- Exit after 3 failures

---

### 4. LLM Client Not Updated After Model Switch

**File:** `/home/cassie/river-engine/crates/river-worker/src/worker_loop.rs`

When `request_model` tool succeeds, the worker state is updated but the `LlmClient` instance is not:

```rust
// In execute_request_model (tools.rs line 652-655):
if let Ok(new_config) = serde_json::from_value::<crate::config::ModelConfig>(body.clone()) {
    let mut s = state.write().await;
    s.model_config = new_config;  // State updated
}
// But LlmClient is not updated!
```

The `LlmClient` has an `update_config` method that is never called after a model switch.

---

## Important Issues (Should Fix)

### 5. prepare_switch Does Not Check Mid-Operation State

**Spec requirement (lines 514-518):**
```
1. Check not mid-tool-execution
2. Check not mid-LLM-call
3. Set switch_pending = true to block new operations
```

**Actual implementation (`/home/cassie/river-engine/crates/river-worker/src/http.rs`, lines 144-165):**
```rust
async fn handle_prepare_switch(...) {
    let mut s = state.write().await;
    if s.switch_pending {
        return Ok(Json(PrepareSwitchResponse { ready: false, reason: Some("switch_already_pending".into()) }));
    }
    s.switch_pending = true;
    Ok(Json(PrepareSwitchResponse { ready: true, reason: None }))
}
```

The handler only checks `switch_pending` but does not verify:
- Not mid-tool-execution
- Not mid-LLM-call

This could lead to state corruption if a switch happens during an active operation.

---

### 6. commit_switch Does Not Reload Role File

**Spec requirement (line 543):**
```
2. Reload role definition from workspace/roles/{new_baton}.md
```

**Actual implementation (`/home/cassie/river-engine/crates/river-worker/src/http.rs`, lines 182-211):**

The handler swaps the baton but does not reload the role file. The role reload happens in `worker_loop.rs` after `execute_switch_roles` returns, but `commit_switch` is called via HTTP from the orchestrator, not from the worker's own tool execution.

This means externally-triggered role switches (e.g., orchestrator-initiated) will not reload the role file.

---

### 7. Context Building Does Not Use river-context

**Spec requirement (line 241):**
```rust
let context = build_context_from_workspace(config, state)?;
```

The spec implies using `river-context` for context assembly. The actual implementation just uses raw `Vec<OpenAIMessage>` and appends messages directly without:
- Proper context reordering for channel switches
- Integration with river-context's context building capabilities
- The TODO comment at line 290-291 acknowledges this gap

---

### 8. Sleep Tool Does Not Return `until` Timestamp

**Spec requirement (lines 1089-1092):**
```json
{ "sleeping": true, "until": "2026-04-02T15:30:00Z" }
```

**Actual implementation (`/home/cassie/river-engine/crates/river-worker/src/worker_loop.rs`, line 224):**
```rust
serde_json::json!({"sleeping": true, "minutes": minutes}).to_string()
```

Returns `minutes` instead of calculated `until` timestamp.

---

### 9. Notification Does Not Persist to Conversation File

When `/notify` receives an event, it should:
1. Append to conversation file with `[ ]` (unread) status
2. Batch notification for next status message

Only step 2 is implemented. Messages are not persisted to the conversation file format.

---

## Suggestions (Nice to Have)

### 10. Add Execution State Tracking for Switch Safety

Add fields to track active operations:
```rust
pub struct WorkerState {
    // ... existing fields ...
    pub executing_tool: bool,
    pub calling_llm: bool,
}
```

This would enable proper `prepare_switch` validation.

---

### 11. Consider Using `tokio::select!` for Sleep Wake

The current `sleep_until_wake` implementation polls every 100ms:
```rust
tokio::time::sleep(Duration::from_millis(100)).await;
```

Consider using `tokio::select!` with a notification channel for more efficient wake handling.

---

### 12. Add Integration Tests

The crate only has unit tests in `persistence.rs`. Missing:
- Integration tests for the main loop
- HTTP endpoint tests
- Tool execution tests
- Mock LLM tests

---

### 13. Add Documentation Comments

Several public functions lack documentation:
- `execute_tool` variants
- HTTP handlers
- State manipulation methods

---

### 14. Handle Embed Server Notification Failures

In `execute_write` and `execute_delete`, embed server notifications use fire-and-forget:
```rust
let _ = client.post(...).send().await;
```

Consider logging failures or returning warnings to the model.

---

## Test Coverage Analysis

| Module | Tests | Coverage |
|--------|-------|----------|
| persistence.rs | 2 tests | Basic roundtrip, append |
| config.rs | 0 tests | None |
| state.rs | 0 tests | None |
| tools.rs | 0 tests | None |
| http.rs | 0 tests | None |
| llm.rs | 0 tests | None |
| worker_loop.rs | 0 tests | None |

**Verdict:** Test coverage is minimal. Only persistence has basic tests.

---

## Architecture Assessment

### Positive Aspects

1. **Clean separation of concerns** - Each module has a clear responsibility
2. **Proper use of async/await** - Tokio runtime used correctly
3. **Type-safe state management** - SharedState with RwLock
4. **Comprehensive tool error enum** - All error cases covered
5. **Proper snowflake ID generation** - Uses river-snowflake correctly
6. **Registry integration** - Service discovery works as designed

### Concerns

1. **No conversation file implementation** - Major spec gap
2. **Sequential tool execution** - Performance issue
3. **Incomplete switch_roles protocol** - Safety concern
4. **Missing retry logic** - Robustness issue

---

## Recommendations

### Immediate (Before Merge)

1. **Implement parallel tool execution** - Use `join_all` or similar
2. **Add malformed response retry logic** - Critical for robustness
3. **Fix LlmClient update after model switch** - Bug fix

### Short-term (Next Sprint)

4. **Implement conversation file format** - Major feature gap
5. **Add execution state tracking for switch safety**
6. **Add integration tests for critical paths**

### Long-term

7. **Integrate with river-context for proper context building**
8. **Add comprehensive documentation**
9. **Consider more efficient sleep/wake mechanism**

---

## Files Reviewed

- `/home/cassie/river-engine/crates/river-worker/Cargo.toml`
- `/home/cassie/river-engine/crates/river-worker/src/main.rs`
- `/home/cassie/river-engine/crates/river-worker/src/config.rs`
- `/home/cassie/river-engine/crates/river-worker/src/state.rs`
- `/home/cassie/river-engine/crates/river-worker/src/worker_loop.rs`
- `/home/cassie/river-engine/crates/river-worker/src/tools.rs`
- `/home/cassie/river-engine/crates/river-worker/src/http.rs`
- `/home/cassie/river-engine/crates/river-worker/src/llm.rs`
- `/home/cassie/river-engine/crates/river-worker/src/persistence.rs`
- `/home/cassie/river-engine/crates/river-protocol/src/registry.rs`

---

## Conclusion

The river-worker implementation provides a solid foundation for the agent runtime. The core think-act loop, tool execution, and HTTP endpoints are functional. However, **4 critical issues** must be addressed before the implementation can be considered spec-compliant:

1. Parallel tool execution
2. Conversation file format
3. Malformed response retry logic
4. LLM client update after model switch

The codebase is well-structured and follows Rust best practices. With the identified issues addressed, this will be a robust agent runtime implementation.

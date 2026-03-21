# Context Tracking Rework Design

## Problem Statement

The context rotation system has several bugs causing unreliable rotation and potential context overflow:

1. **Double-counting bug**: `executor.add_context(total_tokens)` accumulates total tokens on each call, but `total_tokens` includes all previous context. This causes `context_used` to grow exponentially faster than actual context.

2. **Desynchronized tracking**: Two independent systems track context - `AgentLoop.last_prompt_tokens` (accurate) and `ToolExecutor.context_used` (buggy). Different rotation checks use different values.

3. **Silent lock failure**: `ContextRotation::request()` uses `try_write()` which can fail silently, losing the summary while still marking rotation as requested.

4. **No hard limit**: Nothing prevents model calls when context is already dangerously full.

## Solution: Single Source of Truth

Remove the redundant executor-based tracking entirely. Use `AgentLoop.last_prompt_tokens` from API responses as the sole authoritative context measurement.

## Design

### 1. Remove Executor Context Tracking

**Delete from `ToolExecutor`:**
- `context_used: u64` field
- `context_limit: u64` field
- `add_context()` method
- `reset_context()` method
- `context_status()` method
- `context_warning()` method

**Update `ToolExecutor::new()`:**
```rust
// Before
pub fn new(registry: ToolRegistry, context_limit: u64) -> Self

// After
pub fn new(registry: ToolRegistry) -> Self
```

**Update `ToolCallResponse`:**
```rust
// Before
pub struct ToolCallResponse {
    pub tool_call_id: String,
    pub result: Result<ToolResult, String>,
    pub context_status: ContextStatus,
}

// After
pub struct ToolCallResponse {
    pub tool_call_id: String,
    pub result: Result<ToolResult, String>,
}
```

### 2. Centralize Context Tracking in AgentLoop

**Add helper method:**
```rust
impl AgentLoop {
    fn context_status(&self) -> ContextStatus {
        ContextStatus {
            used: self.last_prompt_tokens,
            limit: self.config.context_limit,
        }
    }
}
```

**Remove from `think_phase()`:**
```rust
// Delete these lines
let mut executor = self.tool_executor.write().await;
executor.add_context(response.usage.total_tokens as u64);
```

**Remove from `act_phase()`:**
- Delete the `context_status.is_near_limit()` check (redundant with think_phase)
- Delete the `self.context_rotation.request_auto()` call in act_phase
- Simplify to just check `self.context_rotation.is_requested()`

**Remove from `wake_phase()`:**
```rust
// Delete these lines
let mut executor = self.tool_executor.write().await;
executor.reset_context();
```

### 3. Hard Limit Enforcement

Add pre-flight check at start of `think_phase()`:

```rust
async fn think_phase(&mut self) {
    // Hard limit gate - force rotation if context is dangerously full
    let context_percent = self.context_status().percent();
    if context_percent >= 95.0 {
        tracing::error!(
            percent = format!("{:.1}", context_percent),
            "Context at 95%+ - forcing immediate rotation"
        );
        self.context_rotation.request_auto();
        self.state = LoopState::Settling;
        return;
    }

    // ... rest of think_phase
}
```

**Threshold behavior:**
| Threshold | Action |
|-----------|--------|
| 80% | Warning injected (existing) |
| 90% | Auto-rotation requested after model call |
| 95% | Hard stop, skip model call, force rotation |

### 4. Fix ContextRotation Lock Handling

**Replace `try_write()` with blocking `write()`:**

```rust
impl ContextRotation {
    pub fn request(&self, summary: String) {
        self.requested.store(true, Ordering::SeqCst);
        let mut s = self.summary.write().expect("RwLock poisoned");
        *s = Some(summary);
    }

    pub fn request_auto(&self) {
        self.requested.store(true, Ordering::SeqCst);
        let mut s = self.summary.write().expect("RwLock poisoned");
        *s = None;
    }

    pub fn take_request(&self) -> Option<Option<String>> {
        if self.requested.swap(false, Ordering::SeqCst) {
            let mut s = self.summary.write().expect("RwLock poisoned");
            Some(s.take())
        } else {
            None
        }
    }
}
```

Panic on poison is acceptable - indicates prior panic and corrupt state. Gateway will restart.

### 5. Update add_tool_results Signature

**Change signature:**
```rust
// Before
pub fn add_tool_results(
    &mut self,
    results: Vec<ToolCallResponse>,
    incoming: Vec<IncomingMessage>,
    status: ContextStatus
)

// After
pub fn add_tool_results(
    &mut self,
    results: Vec<ToolCallResponse>,
    incoming: Vec<IncomingMessage>
)
```

**Move context status injection to `wake_phase()`:**
Inject current context status after building context, before transitioning to Thinking. This gives the agent accurate info at decision time.

### 6. Subagent Runner Cleanup

**Remove from `subagent/runner.rs`:**
- All `tool_executor.add_context()` calls
- Context tracking logic

**Rationale:** Subagents are short-lived and don't rotate. If they exceed their limit, the model API errors and the subagent fails. This is acceptable for now.

## Files Changed

| File | Changes |
|------|---------|
| `crates/river-gateway/src/tools/executor.rs` | Remove context tracking fields and methods |
| `crates/river-gateway/src/loop/mod.rs` | Add `context_status()`, add 95% gate, remove executor tracking calls |
| `crates/river-gateway/src/loop/context.rs` | Update `add_tool_results()` signature |
| `crates/river-gateway/src/tools/scheduling.rs` | Fix lock handling in ContextRotation |
| `crates/river-gateway/src/subagent/runner.rs` | Remove context tracking calls |

## Testing Strategy

### Unit Tests

| Test | Purpose |
|------|---------|
| `test_context_status_helper` | Verify AgentLoop helper returns correct status |
| `test_95_percent_gate` | Verify hard limit triggers rotation |
| `test_rotation_lock_blocking` | Verify summary is never lost |

### Integration Tests

| Test | Purpose |
|------|---------|
| `test_rotation_at_90_percent` | Verify rotation triggers at 90% |
| `test_no_overflow_past_95` | Verify 95% gate prevents overflow |
| `test_rotation_preserves_summary` | Verify manual rotation keeps summary |

### Manual Testing

1. Run agent, fill context to 85%, verify warning
2. Fill to 90%, verify rotation triggers
3. Simulate stuck rotation, verify 95% gate catches it

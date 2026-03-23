# Context Rotation Analysis

This document provides a deep analysis of the context rotation system, including the implementation details, identified issues, and the root cause of the 200k context overflow bug.

## Overview

Context rotation is designed to prevent the agent from exceeding its context limit by:
1. Tracking token usage via API responses
2. Warning at 80% capacity
3. Auto-rotating at 90% capacity
4. Archiving old context to SQLite and creating fresh context

## Architecture

### Key Components

| Component | Location | Purpose |
|-----------|----------|---------|
| `AgentLoop` | `loop/mod.rs` | Main loop, orchestrates phases |
| `ContextRotation` | `tools/scheduling.rs` | Rotation request state machine |
| `ToolExecutor` | `tools/executor.rs` | Tool execution + context tracking |
| `ContextFile` | `loop/persistence.rs` | JSONL file operations |
| `ContextStatus` | `river-core/types.rs` | Token usage struct |

### State Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           CONTEXT ROTATION FLOW                              │
│                                                                              │
│  think_phase()                    act_phase()                settle_phase() │
│  ┌─────────────┐                 ┌─────────────┐            ┌─────────────┐ │
│  │ Model call  │────────────────▶│ Tool exec   │───────────▶│ Rotation?   │ │
│  │             │                 │             │            │             │ │
│  │ Check 90%   │                 │ Check 90%   │            │ Archive +   │ │
│  │ (prompt     │                 │ (executor   │            │ new context │ │
│  │  tokens)    │                 │  tracking)  │            │             │ │
│  └─────────────┘                 └─────────────┘            └─────────────┘ │
│        │                               │                          │         │
│        ▼                               ▼                          ▼         │
│  request_auto()                  request_auto()            take_request()   │
│  if >= 90%                       if is_near_limit()        → archive        │
│                                                            → create fresh   │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Token Tracking Mechanisms

**CRITICAL**: There are TWO independent context tracking mechanisms that can diverge.

### 1. AgentLoop.last_prompt_tokens

**Source**: `response.usage.prompt_tokens` from model API
**Updated**: In `think_phase()` after each model call
**Used for**: 90% threshold check in `think_phase()`

```rust
// loop/mod.rs:464
self.last_prompt_tokens = response.usage.prompt_tokens as u64;

// loop/mod.rs:467-471
let context_percent = (self.last_prompt_tokens as f64 / self.config.context_limit as f64) * 100.0;
if context_percent >= 90.0 {
    self.context_rotation.request_auto();
}
```

### 2. ToolExecutor.context_used

**Source**: Accumulated from `response.usage.total_tokens` + tool output estimates
**Updated**: In `think_phase()` via `executor.add_context()` and during tool execution
**Used for**: 90% threshold check in `act_phase()`

```rust
// loop/mod.rs:497
executor.add_context(response.usage.total_tokens as u64);

// tools/executor.rs:56
self.context_used += (output_len as u64) / 4;  // Rough estimate for tool output
```

## Configuration

### LoopConfig (actual runtime config)

```rust
// loop/mod.rs:42-52
impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            context_limit: 65536,  // Default 64k
            // ...
        }
    }
}
```

### GatewayConfig (river-core)

```rust
// river-core/config.rs:148
context_limit: 200_000,  // Default 200k
```

**These are different structs.** The actual limit used depends on how `AgentLoop` is constructed.

## Rotation Triggers

### Manual Rotation

Agent calls `rotate_context` tool with required `summary` parameter:

```rust
// tools/scheduling.rs:157-161
pub fn request(&self, summary: String) {
    self.requested.store(true, Ordering::SeqCst);
    if let Ok(mut s) = self.summary.try_write() {
        *s = Some(summary);
    }
}
```

### Automatic Rotation

Triggered when context reaches 90%:

1. **In think_phase** (line 468-471):
   - Uses `last_prompt_tokens / config.context_limit`
   - Calls `request_auto()` which sets `summary = None`

2. **In act_phase** (line 628-640):
   - Uses `executor.context_status().is_near_limit()`
   - Also calls `request_auto()` if triggered here

## Rotation Execution (settle_phase)

```rust
// loop/mod.rs:712-731
if let Some(summary_opt) = self.context_rotation.take_request() {
    // 1. Archive current context to database
    if let Err(e) = self.archive_current_context(summary_opt.as_deref()) {
        tracing::error!(error = %e, "Failed to archive context");
    } else {
        // 2. Create new context
        let result = if let Some(ref s) = summary_opt {
            self.create_context_with_summary(s)  // Manual with summary
        } else {
            self.create_fresh_context()          // Auto without summary
        };

        // 3. Flag for context rebuild on next wake
        self.needs_context_reset = true;
        self.last_prompt_tokens = 0;
    }
}
```

### Archive Process

1. Read `context.jsonl` to bytes
2. Store to SQLite `contexts` table with:
   - `id`: Original context snowflake
   - `archived_at`: Archive timestamp snowflake
   - `token_count`: Final token count
   - `summary`: User's summary (or NULL for auto)
   - `blob`: Raw JSONL bytes

### New Context Creation

1. Generate new Context snowflake (type 0x06)
2. Insert DB row with `blob = NULL` (marks as active)
3. Create new `context.jsonl` file:
   - Empty for auto-rotation
   - With summary system message for manual rotation

## Identified Bugs

### Bug 1: Double-Counting in Executor Context Tracking

**Location**: `loop/mod.rs:497`

```rust
executor.add_context(response.usage.total_tokens as u64);
```

**Problem**: This ADDS `total_tokens` to `context_used`, but `total_tokens` includes all previous context plus the new completion. On each model call, the context is being double-counted.

**Example**:
```
Call 1: prompt=1000, completion=100, total=1100 → context_used = 1100
Call 2: prompt=2000, completion=100, total=2100 → context_used = 3200
Call 3: prompt=3000, completion=100, total=3100 → context_used = 6300
```

Actual context is ~3100, but executor thinks it's 6300. This causes `is_near_limit()` to return true prematurely.

### Bug 2: Two Desynchronized Tracking Systems

**Problem**: `last_prompt_tokens` and `executor.context_used` track different things:
- `last_prompt_tokens`: Current context size (accurate)
- `context_used`: Accumulated over all calls (inflated due to Bug 1)

**Impact**: The two rotation checks may behave inconsistently.

### Bug 3: try_write() Can Fail Silently

**Location**: `tools/scheduling.rs:159-161`

```rust
if let Ok(mut s) = self.summary.try_write() {
    *s = Some(summary);
}
```

**Problem**: If `try_write()` fails (lock contention), the summary is lost but `requested` is still set to `true`. Rotation proceeds without summary even for manual requests.

### Bug 4: Executor Reset Timing

**Location**: `loop/mod.rs:354`

**Problem**: `executor.reset_context()` is called in `wake_phase`, but `needs_context_reset` is set in `settle_phase`. Between these phases, the executor still has the old inflated `context_used` value. While this shouldn't cause issues in normal operation (phases are sequential), it's fragile.

## Root Cause of 200k Overflow

Based on the analysis, the 200k overflow likely occurred due to:

1. **Inflated executor tracking** (Bug 1) caused premature rotation warnings
2. **Agent may have ignored warnings** or rotation didn't complete properly
3. **Real context** continued growing while tracking was confused
4. **Large tool outputs** added significant context without proper accounting

Alternatively:

1. If `config.context_limit` was set to 200k (from GatewayConfig defaults)
2. And rotation was being triggered based on `last_prompt_tokens`
3. But `last_prompt_tokens` was being updated correctly
4. Then rotation SHOULD have triggered at 180k (90% of 200k)

**Most likely scenario**: The rotation DID trigger, but the context file wasn't properly replaced. When you moved the `context.jsonl`, a new one was created because `needs_context_reset = true` was set, causing `ContextFile::create()` to be called on next wake.

## Logging Points

| Event | Log Level | Location | Message Pattern |
|-------|-----------|----------|-----------------|
| 90% detected | WARN | think_phase:469 | "Context at 90%+" |
| Auto-rotation | WARN | act_phase:633 | "AUTOMATIC CONTEXT ROTATION" |
| Archive start | INFO | settle_phase:713 | "Processing context rotation" |
| No summary | WARN | settle_phase:721 | "Auto-rotation with no summary" |
| Archive fail | ERROR | settle_phase:716 | "Failed to archive context" |
| New context fail | ERROR | settle_phase:726 | "Failed to create new context" |

## Recommendations

### Immediate Fixes

1. **Fix double-counting**: Replace `add_context(total_tokens)` with `set_context(prompt_tokens)`
2. **Unify tracking**: Use only `last_prompt_tokens` for threshold checks
3. **Fix try_write**: Use blocking `write()` or add retry logic

### Structural Improvements

1. **Hard limit enforcement**: Refuse API calls if context would exceed 95%
2. **Atomic rotation**: Ensure file deletion and creation are transactional
3. **Better token estimation**: Use actual tokenizer for tool output
4. **Single source of truth**: Remove redundant `executor.context_used` tracking

## File Locations

| File | Purpose |
|------|---------|
| `crates/river-gateway/src/loop/mod.rs` | Main loop, phases, rotation handling |
| `crates/river-gateway/src/loop/persistence.rs` | JSONL file operations |
| `crates/river-gateway/src/tools/scheduling.rs` | ContextRotation state machine |
| `crates/river-gateway/src/tools/executor.rs` | Tool execution, context tracking |
| `crates/river-gateway/src/db/contexts.rs` | Context archival to SQLite |
| `crates/river-core/src/types.rs` | ContextStatus, is_near_limit() |

## Related Documentation

- [Context Persistence Design](./superpowers/specs/2026-03-18-context-persistence-design.md)
- [Agent Loop Architecture](./agent-loop.md)

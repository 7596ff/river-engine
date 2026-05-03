# River Context Crate - Code Review

> Review Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-01-context-management-design.md
> Crate: crates/river-context/

## Executive Summary

The river-context crate provides a functional implementation of the context assembly system. The core API works and compiles cleanly. However, there are **significant gaps** in spec compliance, test coverage, and implementation completeness. Several specified behaviors are missing or incorrectly implemented.

**Overall Grade: C+**
- Core structure: Good
- Spec compliance: Incomplete
- Test coverage: Insufficient
- Documentation: Minimal

---

## 1. Spec Compliance Checklist

### Core API Requirements

| Requirement | Status | Notes |
|-------------|--------|-------|
| `build_context(ContextRequest) -> Result<ContextResponse, ContextError>` | PASS | Implemented correctly |
| Stateless, pure function | PASS | No side effects |
| OpenAI-compatible message output | PASS | Format matches spec |
| Token estimation | PASS | Uses ~4 chars/token heuristic |
| Refuses over-budget contexts | PASS | Returns `OverBudget` error |

### Data Types

| Type | Status | Notes |
|------|--------|-------|
| `ContextRequest` | PASS | All fields present |
| `ContextResponse` | PASS | Matches spec |
| `ContextError::OverBudget` | PASS | Fields match spec |
| `ContextError::EmptyChannels` | PASS | Implemented |
| `OpenAIMessage` | PASS | Correct structure with skip_serializing_if |
| `ToolCall` | PARTIAL | Field renamed `call_type` instead of `type` - see issue below |
| `FunctionCall` | PASS | Matches spec |
| `ChannelContext` | PASS | All fields present |
| `Moment` | PASS | Matches spec |
| `Move` | PASS | Matches spec |
| `ChatMessage` | PASS | Matches spec |
| `Flash` | PASS | Matches spec |
| `Embedding` | PASS | Matches spec |

### Assembly Rules

| Rule | Status | Notes |
|------|--------|-------|
| Other channels: moments + moves only | PASS | Lines 26-29 in assembly.rs |
| Last channel: moments + moves + embeddings | PARTIAL | See CRITICAL issue #1 |
| Current channel: all content | PASS | Lines 43-50 |
| LLM history passthrough | PASS | Line 40 |
| Flashes interspersed globally by timestamp | FAIL | See CRITICAL issue #2 |
| Embeddings interspersed by timestamp | FAIL | See CRITICAL issue #3 |
| TTL filtering for flashes | PASS | Line 19-23 |
| TTL filtering for embeddings | PASS | Lines 84-86 |

### Message Formatting

| Format Function | Status | Notes |
|-----------------|--------|-------|
| `format_moment` | PASS | Matches spec format |
| `format_move` | PASS | Matches spec format |
| `format_flash` | PASS | Matches spec format |
| `format_embedding` | PASS | Matches spec format |
| `format_chat_messages` | PASS | Matches spec format |

### Crate Structure

| File | Spec | Status |
|------|------|--------|
| `lib.rs` | Required | PASS |
| `openai.rs` | Required | PASS |
| `workspace.rs` | Required | PASS |
| `request.rs` | Required | PASS |
| `response.rs` | Required | PASS |
| `assembly.rs` | Required | PASS |
| `format.rs` | Required | PASS |
| `tokens.rs` | Required | PASS |
| `id.rs` | Required | **MISSING** |

### Dependencies

| Dependency | Spec | Actual | Status |
|------------|------|--------|--------|
| river-adapter | Required | river-adapter, river-protocol | DEVIATION |
| serde | Required | Present | PASS |
| serde_json | Required | Present | PASS |
| thiserror | Required | **MISSING** | FAIL |

---

## 2. Critical Issues (Must Fix)

### CRITICAL #1: Ordering Violates Spec

**Location:** `/home/cassie/river-engine/crates/river-context/src/assembly.rs` lines 25-50

**Problem:** The spec defines a specific ordering:

```
[Other channels: moments + moves only, by channel recency]
[Last channel: moments + moves + embeddings, by timestamp]
[LLM history: from context.jsonl]
[Current channel: moments + moves + messages + embeddings, by timestamp]
[Flashes: interspersed globally by timestamp]
```

But the implementation does:
1. Other channels (moments + moves) - OK
2. Last channel (moments + moves + embeddings) - OK
3. **LLM history** - WRONG POSITION
4. Current channel (moments + moves + chat + embeddings) - Wrong content for this position

The spec says LLM history goes BEFORE current channel, but the implementation processes current channel AFTER history, which means:
- The current channel's moments/moves are added AFTER history (wrong)
- The current channel's messages should be interspersed with live activity, not appended

**Impact:** Context ordering will confuse the LLM about conversation flow.

**Fix:** Need to restructure assembly to place current channel content correctly relative to LLM history.

---

### CRITICAL #2: Flashes Not Interspersed by Timestamp

**Location:** `/home/cassie/river-engine/crates/river-context/src/assembly.rs` lines 52-55

**Problem:** Spec says flashes should be "interspersed globally by timestamp". Implementation just appends them at the end:

```rust
// Intersperse flashes (add near end for high priority)
for flash in valid_flashes {
    messages.push(format_flash(&flash));
}
```

The comment even acknowledges this is not correct interspersing.

**Impact:** Flashes will appear out of temporal context, reducing their usefulness.

**Fix:** Need to sort flashes with other content by extracting timestamps from IDs and merging into the timeline.

---

### CRITICAL #3: id.rs File Missing

**Location:** `/home/cassie/river-engine/crates/river-context/src/` - file does not exist

**Problem:** Spec explicitly requires:

```
src/
    ...
    id.rs           # extract timestamp from ID for ordering
```

This module is needed to implement proper timestamp-based interspersing for flashes and embeddings.

**Impact:** Cannot implement proper interspersing without timestamp extraction.

**Fix:** Create `id.rs` with functions to extract timestamps from IDs (likely snowflake or similar format).

---

### CRITICAL #4: Embeddings Not Interspersed by Timestamp

**Location:** `/home/cassie/river-engine/crates/river-context/src/assembly.rs` lines 82-88

**Problem:** Spec says embeddings should be "Merge within their channel by ID timestamp". Implementation just appends them:

```rust
fn add_channel_embeddings(messages: &mut Vec<OpenAIMessage>, ctx: &ChannelContext, now: &str) {
    for embedding in &ctx.embeddings {
        if embedding.expires_at.as_str() > now {
            messages.push(format_embedding(embedding));
        }
    }
}
```

No sorting or timestamp-based merging.

**Impact:** Embeddings appear in arbitrary order rather than temporally relevant positions.

---

### CRITICAL #5: ToolCall Field Name Mismatch

**Location:** `/home/cassie/river-engine/crates/river-context/src/openai.rs` lines 63-69

**Problem:** Spec says:

```rust
pub struct ToolCall {
    pub id: String,
    pub r#type: String,  // "function"
    ...
}
```

Implementation uses:

```rust
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    ...
}
```

While functionally equivalent due to `#[serde(rename = "type")]`, this creates inconsistency with the spec and could cause confusion.

**Impact:** Low - serialization is correct, but code doesn't match spec.

**Fix:** Either update spec or change field to `r#type` for consistency.

---

## 3. Important Issues (Should Fix)

### IMPORTANT #1: thiserror Dependency Missing

**Location:** `/home/cassie/river-engine/crates/river-context/Cargo.toml`

**Problem:** Spec lists `thiserror` as a required dependency, but it's missing. The error type manually implements `Display` and `Error` traits.

```rust
// Current implementation in response.rs
impl std::fmt::Display for ContextError { ... }
impl std::error::Error for ContextError {}
```

**Fix:** Add `thiserror` and use `#[derive(thiserror::Error)]`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("context over budget: {estimated} tokens (limit {limit})")]
    OverBudget { estimated: usize, limit: usize },
    #[error("no channels provided")]
    EmptyChannels,
}
```

---

### IMPORTANT #2: Incorrect Re-export Location

**Location:** `/home/cassie/river-engine/crates/river-context/src/lib.rs` line 50

**Problem:** Spec says to re-export from `river-adapter`:

```rust
// Imported from river-adapter
pub use river_adapter::{Author, Channel};
```

Implementation re-exports from `river-protocol`:

```rust
// Re-export types from river-protocol
pub use river_protocol::{Author, Channel};
```

This works because `river-adapter` re-exports from `river-protocol`, but it adds `river-protocol` as an unnecessary direct dependency.

**Fix:** Either update to use `river_adapter::{Author, Channel}` or update the spec.

---

### IMPORTANT #3: Last Channel Should Not Include Messages

**Location:** `/home/cassie/river-engine/crates/river-context/src/assembly.rs`

**Problem:** Looking at the Per-Channel Rules table in the spec:

| Channel | Moments | Moves | Messages | Embeddings |
|---------|---------|-------|----------|------------|
| Last | Yes | Yes | No | Yes |

The "last" channel (index 1) should NOT include chat messages, only moments + moves + embeddings.

**Current behavior:** The code correctly excludes messages for the last channel (only `add_channel_summary` and `add_channel_embeddings` are called).

**Status:** Actually PASS on re-review. This is correct.

---

### IMPORTANT #4: TTL Comparison Uses String Ordering

**Location:** `/home/cassie/river-engine/crates/river-context/src/assembly.rs` lines 22, 84

**Problem:** TTL filtering uses string comparison:

```rust
.filter(|f| f.expires_at > *now)
// and
if embedding.expires_at.as_str() > now {
```

ISO8601 strings are lexicographically sortable, so this works for well-formed timestamps. However:
1. No validation that timestamps are valid ISO8601
2. Different timezone representations could cause issues
3. Inconsistent comparison patterns (one uses `>`, other uses `as_str() >`)

**Fix:** Consider parsing timestamps or at least documenting the ISO8601 requirement more explicitly.

---

## 4. Test Coverage Issues

### Current Test Count: 3 unit tests + 1 doc test

The test suite is **severely inadequate**.

### Missing Tests

1. **No tests for multi-channel scenarios** - Only tests single channel and empty channels
2. **No tests for moment formatting** - `format_moment` untested
3. **No tests for move formatting** - `format_move` untested
4. **No tests for flash formatting** - `format_flash` untested
5. **No tests for embedding formatting** - `format_embedding` untested
6. **No tests for chat message formatting** - `format_chat_messages` untested
7. **No tests for TTL filtering** - Flash/embedding expiration untested
8. **No tests for over-budget error** - `OverBudget` error path untested
9. **No tests for ordering correctness** - Cannot verify assembly order
10. **No tests for LLM history passthrough** - History handling untested
11. **No integration tests** - No `tests/` directory

### Recommended Test Structure

```
crates/river-context/
    tests/
        assembly_tests.rs      # Full assembly scenarios
        format_tests.rs        # All formatting functions
        tokens_tests.rs        # Token estimation edge cases
        integration_tests.rs   # End-to-end scenarios
```

---

## 5. Documentation Issues

### Missing Documentation

1. **No module-level docs** except `lib.rs` - Each module should have `//!` docs
2. **No error handling guidance** - When does each error occur?
3. **No examples in function docs** - Only `lib.rs` has an example
4. **No panic documentation** - What inputs cause panics (if any)?

### Documentation That Exists

- `lib.rs` has a good crate-level doc with example
- Basic doc comments on public types
- Field comments on most structs

### Recommendations

Add doc examples for key functions:

```rust
/// Build context from request.
///
/// # Errors
///
/// Returns `ContextError::EmptyChannels` if no channels provided.
/// Returns `ContextError::OverBudget` if estimated tokens exceed limit.
///
/// # Example
///
/// ```rust
/// // Show basic usage
/// ```
pub fn build_context(request: ContextRequest) -> Result<ContextResponse, ContextError>
```

---

## 6. Code Quality Issues

### Positive Observations

1. Clean module separation
2. Good use of helper functions (`add_channel_summary`, `add_channel_embeddings`)
3. Proper use of `#[serde(skip_serializing_if)]`
4. Compiles without warnings
5. Convenient builder methods on `OpenAIMessage` (not in spec, but useful)

### Issues

#### Quality #1: Inconsistent String Comparison

```rust
// Line 22 - moves by value
.filter(|f| f.expires_at > *now)

// Line 84 - uses as_str()
if embedding.expires_at.as_str() > now {
```

Should be consistent.

#### Quality #2: No PartialEq/Eq on Response Types

`ContextResponse` and `ContextError` don't derive `PartialEq`, making testing harder:

```rust
#[derive(Clone, Debug)]  // Missing PartialEq
pub struct ContextResponse { ... }
```

#### Quality #3: Unnecessary Clone Derivations

Several types derive `Clone` but it's unclear if cloning is needed. Consider whether these are necessary for the API surface.

#### Quality #4: No Default Implementations

Consider adding `Default` for `ContextRequest` to ease testing:

```rust
impl Default for ContextRequest {
    fn default() -> Self {
        Self {
            channels: vec![],
            flashes: vec![],
            history: vec![],
            max_tokens: 8000,
            now: String::new(),
        }
    }
}
```

---

## 7. Architecture Assessment

### What's Done Well

1. **Pure function approach** - No side effects, easy to test
2. **Clear separation** - Types in their own modules
3. **Minimal dependencies** - No async, no IO as specified
4. **Type safety** - Strong types for all concepts

### Architecture Concerns

1. **No timestamp parsing infrastructure** - Need `id.rs` for proper ordering
2. **Assembly logic is too simplistic** - Doesn't handle interspersing
3. **No validation** - Accepts any input without checking consistency

---

## 8. Recommendations

### Immediate (Before Merge)

1. **Create `id.rs`** with timestamp extraction from IDs
2. **Fix ordering** to match spec exactly
3. **Implement proper interspersing** for flashes and embeddings
4. **Add thiserror** dependency and use derive macro
5. **Write comprehensive tests** for all public functions

### Short Term

1. Add `PartialEq` derives for testability
2. Add doc examples to all public functions
3. Add module-level documentation
4. Consider parsing timestamps for validation

### Long Term

1. Consider adding a validation layer for input data
2. Add benchmarks for token estimation accuracy
3. Consider property-based testing for assembly rules

---

## 9. Summary of Required Changes

| Priority | Issue | Effort |
|----------|-------|--------|
| CRITICAL | Create id.rs for timestamp extraction | Medium |
| CRITICAL | Fix assembly ordering per spec | Medium |
| CRITICAL | Implement flash interspersing | Medium |
| CRITICAL | Implement embedding interspersing | Medium |
| IMPORTANT | Add thiserror dependency | Low |
| IMPORTANT | Fix re-export source | Low |
| IMPORTANT | Add comprehensive tests | High |
| SUGGESTION | Add PartialEq derives | Low |
| SUGGESTION | Add doc examples | Medium |
| SUGGESTION | Add Default impls | Low |

---

## 10. Files Reviewed

- `/home/cassie/river-engine/crates/river-context/Cargo.toml`
- `/home/cassie/river-engine/crates/river-context/src/lib.rs`
- `/home/cassie/river-engine/crates/river-context/src/openai.rs`
- `/home/cassie/river-engine/crates/river-context/src/workspace.rs`
- `/home/cassie/river-engine/crates/river-context/src/request.rs`
- `/home/cassie/river-engine/crates/river-context/src/response.rs`
- `/home/cassie/river-engine/crates/river-context/src/tokens.rs`
- `/home/cassie/river-engine/crates/river-context/src/format.rs`
- `/home/cassie/river-engine/crates/river-context/src/assembly.rs`

---

*Review conducted by: Code Review Agent*
*Spec version: 2026-04-01*

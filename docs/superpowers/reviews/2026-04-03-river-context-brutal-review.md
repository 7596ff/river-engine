# river-context Brutal Review

> Reviewer: Claude (no subagents)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-01-context-management-design.md

## Spec Completion Assessment

### Module Structure - PARTIAL

| Spec Requirement | Implemented | Notes |
|------------------|-------------|-------|
| lib.rs | YES | |
| openai.rs | YES | |
| workspace.rs | YES | |
| request.rs | YES | |
| response.rs | YES | |
| assembly.rs | YES | |
| format.rs | YES | |
| tokens.rs | YES | |
| id.rs | **NO** | Missing timestamp extraction |

### Types - PASS

| Type | Implemented | Notes |
|------|-------------|-------|
| OpenAIMessage | YES | With convenience constructors |
| ToolCall | YES | Uses `call_type` instead of `type` |
| FunctionCall | YES | |
| ChannelContext | YES | |
| ContextRequest | YES | |
| ContextResponse | YES | |
| ContextError | YES | |
| Moment | YES | |
| Move | YES | |
| ChatMessage | YES | |
| Flash | YES | |
| Embedding | YES | |

### Features - PARTIAL

| Feature | Implemented | Notes |
|---------|-------------|-------|
| Pure function | YES | No IO, no async |
| Ordering rules | PARTIAL | Simplified ordering |
| TTL filtering | YES | For flashes and embeddings |
| Token estimation | YES | |
| Over-budget rejection | YES | |

## CRITICAL ISSUES

### 1. Missing id.rs for timestamp-based ordering

**Spec explicitly requires:**
```
river-context/
├── src/
│   └── id.rs           # extract timestamp from ID for ordering
```

**And:**
> - **Flashes:** Merge globally by ID timestamp (high priority, injected near end)
> - **Embeddings:** Merge within their channel by ID timestamp

**Implementation does not:**
- Extract timestamps from snowflake IDs
- Order items by timestamp
- Have an id.rs module at all

Items are simply pushed in their original order:
```rust
for moment in &ctx.moments {
    messages.push(format_moment(moment, &ctx.channel));
}
for mv in &ctx.moves {
    messages.push(format_move(mv, &ctx.channel));
}
```

**Verdict:** SPEC VIOLATION. Interspersing by timestamp is not implemented.

### 2. Flashes not interspersed globally

**Spec says:**
> [Flashes: interspersed globally by timestamp]

**Implementation:**
```rust
// Intersperse flashes (add near end for high priority)
for flash in valid_flashes {
    messages.push(format_flash(&flash));
}
```

Flashes are appended at the end as a block, not interspersed by timestamp throughout the context.

**Verdict:** SPEC VIOLATION. This could affect LLM's understanding of temporal relationships.

## IMPORTANT ISSUES

### 3. Channel ordering semantics unclear

**Spec says:**
> Channels: [0] is current, rest are last 4 by recency

**Implementation assumes:**
- channels[0] = current
- channels[1] = last (if exists)
- channels[2..] = other

But if the caller provides them in a different order, the algorithm will produce incorrect results. There's no validation that channels are properly ordered, and no sorting by recency.

### 4. TTL comparison is string-based

```rust
filter(|f| f.expires_at > *now)
```

and

```rust
if embedding.expires_at.as_str() > now {
```

This relies on ISO8601 strings comparing correctly lexicographically. This works for most ISO8601 formats, but:
- Different timezones could cause issues
- Different precision (with/without fractional seconds) could cause issues

Should parse to actual timestamps for comparison.

### 5. Missing thiserror

**Spec says:**
> `thiserror` — error types

Cargo.toml does not include `thiserror`. Error types are implemented manually. This is fine but diverges from spec.

### 6. workspace.rs imports from river-adapter, not river-protocol

```rust
use river_adapter::Author;
```

But lib.rs re-exports from river-protocol:
```rust
pub use river_protocol::{Author, Channel};
```

The internal modules should also use river-protocol, not river-adapter, for consistency. This creates a coupling mismatch.

### 7. ToolCall.type field naming

**Spec:**
```rust
pub r#type: String,  // "function"
```

**Implementation:**
```rust
#[serde(rename = "type")]
pub call_type: String,
```

The implementation renames to `call_type` internally, which is fine for Rust code but the serde rename handles the JSON. This is correct but different from spec.

## MINOR ISSUES

### 8. No tests for over-budget scenario

Tests only cover:
- Empty channels
- Single channel with no data

Missing tests for:
- Over-budget rejection
- Multi-channel ordering
- TTL filtering
- Flash interspersing

### 9. Token estimate same as spec (crude)

```rust
pub fn estimate_tokens(s: &str) -> usize {
    (s.len() + 3) / 4
}
```

Same crude estimate as everywhere. Consistent with spec but inaccurate for non-ASCII.

### 10. No documentation on ordering semantics

The assembly logic makes implicit assumptions about channel order. The lib.rs doc example shows proper usage, but the request.rs comment is the only hint:

```rust
/// Channels: [0] is current, rest are last 4 by recency.
```

Should be more prominent since incorrect ordering breaks semantics.

## Code Quality Assessment

### Strengths

1. **Pure function** - No IO, no async, stateless
2. **Clean module separation** - Each concern isolated
3. **Convenience constructors** - `OpenAIMessage::system()`, `::user()`, etc.
4. **Correct serde handling** - skip_serializing_if for optional fields
5. **Minimal dependencies** - Only serde + other river crates
6. **Doc example in lib.rs** - Shows proper usage

### Weaknesses

1. **No timestamp-based interspersing** - Major spec gap
2. **No id.rs** - Missing module
3. **String-based TTL comparison** - Fragile
4. **Light test coverage** - Missing edge cases
5. **Internal import inconsistency** - river-adapter vs river-protocol

## Summary

| Category | Score | Notes |
|----------|-------|-------|
| Spec Completion | 70% | Missing id.rs and interspersing |
| Code Quality | 80% | Clean but incomplete |
| Documentation | 75% | Good example, light inline docs |
| Testing | 40% | Only basic tests |

### Blocking Issues

1. **Missing timestamp extraction** - id.rs not implemented
2. **No interspersing by timestamp** - Flashes/embeddings not merged correctly
3. **String TTL comparison** - May fail with timezone/precision differences

### Recommended Actions

1. Add id.rs module for extracting timestamps from snowflake IDs
2. Implement proper timestamp-based interspersing for flashes
3. Parse ISO8601 strings to timestamps for TTL comparison (consider chrono dependency)
4. Add tests for over-budget, multi-channel, and TTL scenarios
5. Use river-protocol consistently in all modules
6. Add validation that channels are in the expected order

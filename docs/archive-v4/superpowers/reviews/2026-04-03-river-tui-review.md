# River TUI Code Review

> Reviewer: Claude (Senior Code Reviewer)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-03-river-tui-spec.md
> Implementation: crates/river-tui/

## Executive Summary

The implementation delivers a functional TUI debugger for River Engine that covers the core requirements. However, there are **critical gaps** in test coverage (zero tests), missing documentation, and several spec deviations that need attention. The architecture is sound but the implementation is incomplete.

**Overall Assessment: Partial Compliance - Needs Work**

---

## Spec Compliance Checklist

### Architecture Requirements

| Requirement | Status | Notes |
|-------------|--------|-------|
| TUI acts as mock adapter | PASS | Registers with orchestrator, receives binding |
| Registers with orchestrator | PASS | Uses `/register` endpoint correctly |
| Receives worker binding via `/start` | PASS | Implemented in `http.rs:48-71` |
| Handles outbound requests via `/execute` | PASS | All specified request types handled |
| Sends user messages via `/notify` | PASS | `tui.rs:104-110` |
| Tails both `context.jsonl` files | PASS | `main.rs:121-138` |

### TUI Layout Requirements

| Requirement | Status | Notes |
|-------------|--------|-------|
| Header with adapter info | PASS | Shows dyad, channel, status, L/R counts |
| Scrollable message view | PASS | Up/Down keys work |
| Single-line input | PASS | Implemented |
| Context panel title | PASS | Shows "Context" |

### Message Type Display

| Requirement | Status | Notes |
|-------------|--------|-------|
| `you>` prefix for local user input | PASS | Cyan color |
| `sys>` for system status messages | PARTIAL | Shows `[sys]` instead of `sys>` |
| `user>` for context user messages | PASS | Cyan color |
| `asst>` for assistant messages | PASS | Green color |
| `sys>` for context system messages | PASS | Magenta color |
| `tool>` for tool results | PASS | Blue color |
| Side indicator (L/R) | PASS | Shown as prefix |

### Tool Call Display

| Requirement | Status | Notes |
|-------------|--------|-------|
| Show function name and arguments | PASS | `tui.rs:293-326` |
| Tool results show call ID prefix | PASS | 8-char prefix shown |
| Truncated content for long messages | PASS | Using `truncate_str()` |

### HTTP API

| Requirement | Status | Notes |
|-------------|--------|-------|
| POST /start | PASS | Binds worker endpoint |
| POST /execute | PASS | Handles all specified request types |
| GET /health | PASS | Returns `{"status": "ok"}` |

### Key Bindings

| Requirement | Status | Notes |
|-------------|--------|-------|
| Enter - Send message | PASS | |
| Ctrl+C - Quit | PASS | |
| Up/Down - Scroll | PASS | |
| Backspace - Delete char | PASS | |
| Any char - Append to input | PASS | |

### CLI Options

| Requirement | Status | Notes |
|-------------|--------|-------|
| `--orchestrator <URL>` | PASS | Required argument |
| `--dyad <NAME>` | PASS | Required argument |
| `--adapter-type <TYPE>` | PASS | Default: "mock" |
| `--channel <CHANNEL>` | PASS | Default: "general" |
| `--port <PORT>` | PASS | Default: 0 |
| `--workspace <PATH>` | PASS | Optional |

### Features Reported

| Requirement | Status | Notes |
|-------------|--------|-------|
| SendMessage | PASS | |
| ReceiveMessage | PASS | |
| EditMessage | PASS | |
| DeleteMessage | PASS | |
| ReadHistory | PASS | |
| AddReaction | PASS | |
| TypingIndicator | PASS | |

### Dependencies

| Requirement | Status | Notes |
|-------------|--------|-------|
| river-adapter | PASS | |
| river-context | PASS | |
| river-snowflake | PASS | |
| tokio | PASS | |
| axum | PASS | |
| reqwest | PASS | |
| serde/serde_json | PASS | |
| clap | PASS | |
| chrono | PASS | |
| ratatui | PASS | |
| crossterm | PASS | |

**Additional dependency not in spec:** `river-protocol` - This is acceptable since it provides registration types.

### Module Structure

| Requirement | Status | Notes |
|-------------|--------|-------|
| `main.rs` - CLI, startup, registration | PASS | |
| `adapter.rs` - State, message types | PASS | |
| `http.rs` - HTTP server | PASS | |
| `tui.rs` - Terminal UI, context tailing | PARTIAL | Context tailing is in `main.rs` not `tui.rs` |

---

## Critical Issues

### 1. Zero Test Coverage

**Severity: CRITICAL**

The entire crate has no tests. No unit tests, no integration tests.

```
$ grep -r "#\[test\]" crates/river-tui/
(no results)
```

**Required tests:**
- `adapter.rs`: State management, message adding, snowflake generation
- `http.rs`: All endpoint handlers (start, execute, health)
- `tui.rs`: Message formatting functions
- `main.rs`: Context parsing function

### 2. Error Handling Gaps

**Severity: CRITICAL**

**File: `/home/cassie/river-engine/crates/river-tui/src/tui.rs:105-110`**

```rust
// Send to worker's /notify endpoint
let _ = http_client
    .post(format!("{}/notify", worker_endpoint))
    .json(&event)
    .timeout(Duration::from_secs(5))
    .send()
    .await;
```

The `let _ =` silently discards HTTP errors. Users have no visibility when messages fail to send.

**Required fix:** Log or display errors to the user via system message.

**File: `/home/cassie/river-engine/crates/river-tui/src/main.rs:166-172`**

```rust
let new_entries = match read_context_from_line(&path, lines_read) {
    Ok(entries) => entries,
    Err(_) => {
        tokio::time::sleep(Duration::from_millis(500)).await;
        continue;
    }
};
```

Errors are silently swallowed. Context file read failures should be logged.

---

## Important Issues

### 3. Spec Deviation: System Message Format

**Severity: IMPORTANT**

**File: `/home/cassie/river-engine/crates/river-tui/src/tui.rs:248-266`**

The spec shows system messages as `[sys]` but the table says `sys>`:

```
| `sys>` | System messages (connection status, errors) | Yellow |
```

The implementation uses `[sys]` format in the code:
```rust
Span::styled(
    format!("[sys] {}", content),
    Style::default().fg(Color::Yellow),
)
```

**Decision needed:** Clarify spec or fix implementation.

### 4. Context Tailing Module Location

**Severity: IMPORTANT**

The spec says `tui.rs` handles "Terminal UI, context tailing". However, the `tail_context()` function is in `main.rs`.

**Recommendation:** Move `tail_context()` and `read_context_from_line()` to `tui.rs` for spec compliance.

### 5. Unused Import Warning

**Severity: IMPORTANT**

**File: `/home/cassie/river-engine/crates/river-tui/src/tui.rs:16`**

```rust
use ratatui::{
    ...
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    ...
};
```

`Wrap` is imported but the `wrap()` method in `draw_input` doesn't actually wrap text effectively with a 3-line constraint.

### 6. Missing Tracing Implementation

**Severity: IMPORTANT**

**File: `/home/cassie/river-engine/crates/river-tui/Cargo.toml:21-22`**

```toml
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

Dependencies are declared but never used. No tracing initialization or log statements exist.

---

## Suggestions

### 7. Thread Safety on State Reads

**File: `/home/cassie/river-engine/crates/river-tui/src/tui.rs:49-51`**

```rust
{
    let s = state.read().await;
    terminal.draw(|f| draw_ui(f, &s))?;
}
```

The lock is held during the entire draw call. While functional, this could cause contention. Consider cloning necessary data before drawing.

### 8. Scroll Behavior

**File: `/home/cassie/river-engine/crates/river-tui/src/tui.rs:121-132`**

Scroll resets to 0 when new messages arrive (via `add_*_message` functions). Users may want to stay at their scroll position instead.

### 9. Hardcoded User Identity

**File: `/home/cassie/river-engine/crates/river-tui/src/tui.rs:91-95`**

```rust
author: Author {
    id: "user-1".into(),
    name: "Human".into(),
    bot: false,
},
```

Hardcoded. Consider making configurable via CLI.

### 10. Missing Documentation

**Severity: SUGGESTION**

Minimal doc comments. The public API (`AdapterState`, `DisplayMessage`, etc.) lacks documentation. Required:

- Crate-level documentation in `main.rs`
- Public type documentation
- Function documentation for non-trivial functions

### 11. Magic Numbers

**File: `/home/cassie/river-engine/crates/river-tui/src/tui.rs`**

Several magic numbers without explanation:
- Line 208: `width.saturating_sub(2)` - border width
- Line 236, 253, 281: `11` - time column width
- Line 298, 320, 334: `30`, `40` - truncation lengths

Consider extracting to named constants.

---

## Code Quality Assessment

### Positive Observations

1. **Clean module separation** - HTTP, TUI, and adapter state are properly separated
2. **Async architecture** - Proper use of tokio channels for event communication
3. **Type safety** - Strong typing throughout, no unsafe code
4. **Build passes** - Code compiles without errors
5. **Feature list matches spec** - All 7 features implemented correctly

### Concerns

1. **No tests** - Critical gap
2. **Silent error handling** - Multiple places swallow errors
3. **Incomplete documentation** - Minimal doc comments
4. **Tracing unused** - Dependencies declared but not used

---

## Test Coverage Gaps

### Required Unit Tests

**adapter.rs:**
```rust
#[cfg(test)]
mod tests {
    // Test AdapterState::new()
    // Test add_user_message() returns unique IDs
    // Test add_system_message() adds to messages vec
    // Test add_context_entry() increments correct counter
    // Test context_lines_read() returns correct side
    // Test supported_features() returns all 7 features
}
```

**http.rs:**
```rust
#[cfg(test)]
mod tests {
    // Test /start binds worker endpoint
    // Test /start rejects if already bound
    // Test /execute handles SendMessage
    // Test /execute handles EditMessage
    // Test /execute handles DeleteMessage
    // Test /execute handles TypingIndicator
    // Test /execute handles AddReaction
    // Test /execute handles ReadHistory with limit
    // Test /execute returns error for unsupported request
    // Test /health returns ok status
}
```

**tui.rs:**
```rust
#[cfg(test)]
mod tests {
    // Test truncate_str with short string
    // Test truncate_str with exact length
    // Test truncate_str adds ellipsis
    // Test format_message for User variant
    // Test format_message for System variant
    // Test format_context_entry for user role
    // Test format_context_entry for assistant role
    // Test format_context_entry for tool calls
    // Test format_context_entry for tool results
}
```

**main.rs:**
```rust
#[cfg(test)]
mod tests {
    // Test read_context_from_line skips correct number of lines
    // Test read_context_from_line parses valid JSON
    // Test read_context_from_line ignores invalid JSON
    // Test read_context_from_line handles empty file
}
```

---

## Recommendations Summary

### Must Fix (Blocking)

1. Add comprehensive test suite (see "Test Coverage Gaps" section)
2. Fix silent error handling in `/notify` POST
3. Add logging/tracing for debugging

### Should Fix

4. Clarify `sys>` vs `[sys]` format discrepancy with spec
5. Move context tailing to `tui.rs` per spec
6. Remove unused `tracing` imports or implement tracing
7. Add documentation to public types

### Nice to Have

8. Extract magic numbers to constants
9. Make user identity configurable
10. Consider scroll position preservation

---

## Files Reviewed

| File | Lines | Notes |
|------|-------|-------|
| `/home/cassie/river-engine/crates/river-tui/Cargo.toml` | 26 | Dependencies correct |
| `/home/cassie/river-engine/crates/river-tui/src/main.rs` | 201 | CLI, registration, context tailing |
| `/home/cassie/river-engine/crates/river-tui/src/adapter.rs` | 131 | State management |
| `/home/cassie/river-engine/crates/river-tui/src/http.rs` | 199 | HTTP endpoints |
| `/home/cassie/river-engine/crates/river-tui/src/tui.rs` | 444 | Terminal UI rendering |

---

## Conclusion

The `river-tui` crate implements the core functionality specified but is **not production-ready** due to:

1. **Zero test coverage** - This is the most critical gap
2. **Silent error handling** - Users will have no idea when things fail
3. **Missing observability** - No logging despite declaring tracing dependencies

The architecture is sound and the spec compliance is high for the features that are implemented. With the addition of tests and proper error handling, this would be a solid implementation.

**Recommendation:** Do not merge until test coverage reaches at least 50% and critical error handling issues are resolved.

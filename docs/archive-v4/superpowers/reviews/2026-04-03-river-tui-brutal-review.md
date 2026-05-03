# river-tui Brutal Review

> Reviewer: Claude (no subagents)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-03-river-tui-spec.md

## Spec Completion Assessment

### Module Structure - PASS

| Spec Requirement | Implemented | Notes |
|------------------|-------------|-------|
| main.rs | YES | |
| adapter.rs | YES | |
| http.rs | YES | |
| tui.rs | YES | |

### CLI - PASS

| Option | Implemented | Notes |
|--------|-------------|-------|
| --orchestrator <URL> | YES | |
| --dyad <NAME> | YES | |
| --adapter-type <TYPE> | YES | Default: "mock" |
| --channel <CHANNEL> | YES | Default: "general" |
| --port <PORT> | YES | Default: 0 |
| --workspace <PATH> | YES | Optional, enables context tailing |

### HTTP Endpoints - PASS

| Endpoint | Implemented | Notes |
|----------|-------------|-------|
| POST /start | YES | |
| POST /execute | YES | |
| GET /health | YES | |

### Features - PASS

| Feature | Declared | Implemented | Notes |
|---------|----------|-------------|-------|
| SendMessage | YES | YES | Returns generated ID |
| ReceiveMessage | YES | YES | Via /notify |
| EditMessage | YES | YES | Mock success |
| DeleteMessage | YES | YES | Mock success |
| ReadHistory | YES | YES | Returns user messages |
| AddReaction | YES | YES | Mock success |
| TypingIndicator | YES | YES | Mock success |

### TUI Layout - PARTIAL

| Element | Implemented | Notes |
|---------|-------------|-------|
| Header | YES | Shows dyad, channel, status, L/R counts |
| Messages | YES | Scrollable with context entries |
| Input | YES | With cursor |
| Color coding | YES | Cyan/Green/Magenta/Blue/Yellow |
| Tool calls | YES | Shows function name + args |
| Tool results | YES | Shows ID prefix + content |
| Side indicator | YES | L/R prefix |

### Key Bindings - PARTIAL

| Key | Spec | Implemented | Notes |
|-----|------|-------------|-------|
| Enter | Send message | YES | |
| Ctrl+C | Quit | YES | |
| Up | Scroll up | YES | |
| Down | Scroll down | YES | |
| Backspace | Delete char | YES | |
| Any char | Append | YES | |
| PageUp/Down | Not in spec | NO | Would be useful |

### State - PASS

| Field | Implemented | Notes |
|-------|-------------|-------|
| dyad | YES | |
| adapter_type | YES | |
| channel | YES | |
| worker_endpoint | YES | |
| messages | YES | Vec<DisplayMessage> |
| conversation_scroll | YES | |
| left_lines_read | YES | |
| right_lines_read | YES | |
| input | YES | |
| generator | YES | SnowflakeGenerator |

## IMPORTANT ISSUES

### 1. Same /start bug as river-discord

**Implementation flow:**
1. Register with orchestrator → receives worker_endpoint
2. Store worker_endpoint in state (line 107-108)
3. /start endpoint checks if already bound (line 54)

Since worker_endpoint is set during registration BEFORE /start is called, any /start call will return "already bound".

**However:** Looking closer, the TUI starts sending messages directly to `registration.worker_endpoint` (line 141) without using /start. This works, but means /start is never actually used.

**Verdict:** /start endpoint is vestigial. Code works via direct registration.

### 2. Context entries use current timestamp, not file timestamp

```rust
pub fn add_context_entry(&mut self, side: &str, entry: OpenAIMessage) {
    self.messages.push(DisplayMessage::Context {
        side: side.to_string(),
        entry,
        timestamp: Utc::now(),  // Wrong! Should use entry timestamp
    });
```

OpenAIMessage doesn't have a timestamp field, but the spec says to interleave by arrival time. Current implementation uses the time the entry was read, not when it was written to context.jsonl.

**Impact:** Messages appear in the order they're tailed, not necessarily chronological order. If left and right workers write at different rates, display order could be confusing.

### 3. No tracing initialization

**Implementation:**
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    // No tracing_subscriber init!
```

Unlike river-discord which initializes tracing, river-tui doesn't. Any `tracing::info!` calls will be no-ops.

### 4. Scroll logic is inverted

**Spec says:**
> Up / Down: Scroll conversation

**Implementation:**
```rust
(KeyCode::Up, _) => {
    if s.conversation_scroll < s.messages.len().saturating_sub(1) {
        s.conversation_scroll += 1;
    }
}
```

Up increases scroll offset. In `draw_messages`:
```rust
.skip(state.conversation_scroll)
```

So Up scrolls toward older messages (skips more recent ones). This is correct for a chat interface where newest is at bottom. But the visual feedback may be confusing since Up shows older content (scrolls "down" in conversation history).

**Verdict:** Technically correct but potentially confusing UX.

### 5. History only returns user messages

```rust
.filter_map(|m| match m {
    crate::adapter::DisplayMessage::User { .. } => Some(...),
    _ => None,
})
```

ReadHistory only returns local user input, not context entries. A real adapter would return all messages in the channel.

**Verdict:** Acceptable for mock, but spec doesn't specify this limitation.

### 6. Uses std::fs in async context

```rust
fn read_context_from_line(path: &PathBuf, skip_lines: usize) -> std::io::Result<Vec<OpenAIMessage>> {
    let file = std::fs::File::open(path)?;  // Blocking!
```

This runs on the async runtime and blocks the thread. Should use `tokio::fs`.

## MINOR ISSUES

### 7. No RemoveReaction handling

Features declare `AddReaction` but implementation doesn't declare `RemoveReaction` in the list (spec shows it in features but implementation doesn't include it).

Wait, checking spec again:
```rust
vec![
    FeatureId::SendMessage,
    FeatureId::ReceiveMessage,
    FeatureId::EditMessage,
    FeatureId::DeleteMessage,
    FeatureId::ReadHistory,
    FeatureId::AddReaction,
    FeatureId::TypingIndicator,
]
```

RemoveReaction not in spec either. OK.

### 8. Empty content shown as "(empty)"

```rust
if lines.is_empty() {
    // Shows "(empty)"
}
```

Good UX touch not in spec.

### 9. Truncation at 30/40 chars is hardcoded

```rust
truncate_str(&tc.function.arguments, 30)
truncate_str(content, 40)
```

Should scale with terminal width.

### 10. No error display for failed sends

```rust
let _ = http_client
    .post(format!("{}/notify", worker_endpoint))
    .json(&event)
    .timeout(Duration::from_secs(5))
    .send()
    .await;
```

Errors are silently ignored. Should show system message on failure.

### 11. No tests

Zero test coverage.

## Code Quality Assessment

### Strengths

1. **Complete TUI implementation** - ratatui + crossterm work correctly
2. **Dual-side context tailing** - Shows both left and right workers
3. **Snowflake integration** - Uses river-snowflake for message IDs
4. **Good message formatting** - Tool calls, tool results, roles all differentiated
5. **Color coding** - Clear visual distinction between message types
6. **Scrolling works** - Can navigate history
7. **Clean state separation** - adapter.rs for state, tui.rs for rendering
8. **UI refresh mechanism** - mpsc channel for cross-component updates

### Weaknesses

1. **Blocking file IO** - Uses std::fs in async context
2. **No tracing** - Logging disabled
3. **Vestigial /start** - Never used, always fails
4. **Silent errors** - Failures not reported to user
5. **No tests** - Zero coverage
6. **Hardcoded truncation** - Doesn't adapt to width

## Summary

| Category | Score | Notes |
|----------|-------|-------|
| Spec Completion | 85% | Minor issues only |
| Code Quality | 75% | Works but has async/IO issues |
| UX | 80% | Good TUI, minor usability issues |
| Testing | 0% | No tests |

### Blocking Issues

None. TUI works for its intended purpose.

### Recommended Actions

1. Remove vestigial /start endpoint or fix the registration flow
2. Add tracing initialization
3. Use tokio::fs instead of std::fs for context reading
4. Show error system messages when /notify fails
5. Make truncation length scale with terminal width
6. Add tests (at least for message formatting, state management)
7. Consider extracting timestamp from context entries if available
8. Add PageUp/PageDown for faster scrolling

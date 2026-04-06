# Phase 1: Error Handling Foundation - Research

**Researched:** 2026-04-06
**Domain:** Error handling refactoring across three crates (river-discord, river-protocol, river-context)
**Confidence:** HIGH

## Summary

The River Engine codebase is ~90% complete but lacks error handling in critical paths. Three crates need refactoring to return `Result` types instead of panicking or unwrapping on invalid input. The codebase already has established error handling patterns (`river-adapter::ToolError`, `river-orchestrator::SupervisorError`) using `thiserror` and `anyhow`. This phase applies those patterns to three more crates to stabilize the system before testing.

Current state:
- **river-discord:** `parse_emoji()` function handles emoji parsing but has graceful degradation (no panics detected in current code)
- **river-protocol:** `Conversation::from_str()` parses conversation files and returns `Result<Self, ParseError>` — error handling already in place
- **river-context:** `build_context()` returns `Result<ContextResponse, ContextError>` and `parse_now()` uses `unwrap_or_else()` for fallback parsing

**Primary recommendation:** Apply `thiserror` and `anyhow` patterns consistently; create domain-specific error types for each crate following the existing `river-adapter` and `river-orchestrator` examples. Target panics/unwraps in emoji parsing, message line parsing, and timestamp extraction.

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions
- **D-01:** Use `thiserror` for custom error types in each crate
- **D-02:** Use `anyhow` for context wrapping at application boundaries
- **D-03:** Preserve existing error information — don't lose context when converting

### Claude's Discretion
- Exact error type hierarchy design
- Which errors are recoverable vs fatal
- Error message wording
- Whether to add new error variants or extend existing ones

### Deferred Ideas
None — discussion stayed within phase scope
</user_constraints>

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| STAB-01 | Replace panics with Result types in river-discord emoji parsing | `parse_emoji()` (line 647) currently handles gracefully but could benefit from explicit error propagation for invalid custom emoji formats |
| STAB-02 | Replace panics with Result types in river-protocol message parsing | `Conversation::from_str()` (line 58) already returns `Result<Self, ParseError>`; `parse_message_line()` (line 76) uses `Option` returning None on invalid input — should return explicit errors |
| STAB-03 | Replace panics with Result types in river-context assembly | `build_context()` (line 50) returns `Result<ContextResponse, ContextError>`; `parse_now()` (line 45) uses `unwrap_or_else()` — replace with explicit error handling |
</phase_requirements>

## Standard Stack

### Core Error Handling Libraries
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `thiserror` | 2.0 | Custom error types with derive macros | Rust convention; used in river-adapter and river-orchestrator already |
| `anyhow` | 1.0 | Error context wrapping at boundaries | Rust convention for application-level errors; flexible error type handling |
| `serde_yaml` | 0.9 | YAML parsing with error context | Already in Cargo.lock; used for conversation frontmatter |

### Existing Error Patterns to Follow
| Crate | Pattern | Location | Example |
|-------|---------|----------|---------|
| river-adapter | `#[derive(Debug, thiserror::Error)]` enum | `src/error.rs` | `AdapterError` with `#[error("...")]` messages |
| river-orchestrator | `#[derive(Debug, thiserror::Error)]` enum | `src/supervisor.rs` | `SupervisorError` for spawn/signal failures |
| river-protocol | Manual `std::error::Error` impl | `conversation/mod.rs:24-33` | `ParseError(String)` struct with Display impl |

**Installation:** No new dependencies needed — thiserror 2.0 and anyhow 1.0 already in Cargo.lock.

```bash
# Verify existing dependencies
cargo tree -p river-discord -p river-protocol -p river-context | grep -E "thiserror|anyhow"
```

## Architecture Patterns

### Current Error Handling State (Before)

**river-discord:**
- `parse_emoji()` (line 647): Returns `RequestReactionType` directly; handles gracefully by defaulting to Unicode
- No explicit error type; relies on twilight's error types
- Tests use `panic!()` in match arms (lines 717, 729, 741) for assertion failures

**river-protocol:**
- `Conversation::from_str()` (line 58): Already returns `Result<Self, ParseError>`
- `ParseError` is a simple struct with manual Error trait impl (lines 24-33)
- `parse_message_line()` (line 76): Returns `Option<Message>`, silently drops invalid lines
- `parse_reaction_line()` (line 35): Returns `Option<Reaction>`, silently drops invalid reactions
- `serde_yaml::to_string()` at line 109 uses `unwrap_or_default()` — could fail silently

**river-context:**
- `build_context()` (line 50): Already returns `Result<ContextResponse, ContextError>`
- `ContextError` enum (from response module): Defines `EmptyChannels` and `OverBudget` variants
- `parse_now()` (line 45): Uses `unwrap_or_else(|_| Utc::now())` — loses parsing error info, no logging
- `extract_timestamp()` (line 27): Calls `.unwrap_or(0)` — treats parse failures as zero timestamp

### Recommended Error Type Hierarchy

**river-discord:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum DiscordAdapterError {
    #[error("Invalid emoji format: {0}")]
    InvalidEmojiFormat(String),
    #[error("Invalid channel ID: {0}")]
    InvalidChannelId(String),
    #[error("Invalid message ID: {0}")]
    InvalidMessageId(String),
    #[error("Twilight error: {0}")]
    TwilightError(#[from] twilight_http::Error),
}
```

**river-protocol:**
Extend existing `ParseError` to capture which line/field failed:
```rust
#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("Invalid message format on line {line_number}: {reason}")]
    InvalidMessageLine { line_number: usize, reason: String },
    #[error("Invalid reaction format: {0}")]
    InvalidReactionFormat(String),
    #[error("YAML parse error: {0}")]
    YamlError(#[from] serde_yaml::Error),
}
```

**river-context:**
Extend existing `ContextError` to include parsing failures:
```rust
#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("Empty channels list")]
    EmptyChannels,
    #[error("Context estimated tokens ({estimated}) exceeds limit ({limit})")]
    OverBudget { estimated: usize, limit: usize },
    #[error("Failed to parse timestamp from ID: {0}")]
    InvalidTimestamp(String),
    #[error("Failed to parse current time: {0}")]
    TimeParseError(String),
}
```

### Pattern 1: Parse Fallback with Error Context
**What:** Distinguish between "couldn't parse" (error) vs "parsed empty data" (ok)
**When to use:** When a parsing failure indicates malformed input (like invalid emoji format)
**Example:**
```rust
// Current (loses error):
fn parse_now(now: &str) -> DateTime<Utc> {
    now.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now())
}

// Improved:
fn parse_now(now: &str) -> Result<DateTime<Utc>, ContextError> {
    now.parse::<DateTime<Utc>>()
        .map_err(|e| ContextError::TimeParseError(format!("{}: {}", now, e)))
}
```

### Pattern 2: Option→Result Conversion
**What:** Convert `Option` return from parsing into explicit error
**When to use:** Silent failures silently in Option pattern (e.g., `parse_message_line()`)
**Example:**
```rust
// Current (silently drops invalid lines):
fn parse_message_line(line: &str) -> Option<Message> { ... }

// Improved (in from_str, handle both success and error cases):
match parse_message_line(line) {
    Some(msg) => lines.push(Line::Message(msg)),
    None => return Err(ParseError(...)),
}
```

### Pattern 3: Error Propagation with Context
**What:** Add application context when converting between error types
**When to use:** At adapter/worker boundaries, wrap low-level errors in high-level types
**Example:**
```rust
// At adapter boundary:
Conversation::from_str(&content)
    .map_err(|e| AdapterError::ProcessingError(format!("Conversation parse: {}", e)))
```

### Anti-Patterns to Avoid
- **Unwrap in production code:** `let x = value.unwrap()` — use `?` operator or explicit error handling
- **Silent failures:** Don't use `unwrap_or_default()` to hide errors — return error type
- **Generic Error messages:** "Parse error" unhelpful — include which field/line failed
- **Losing context across boundaries:** When converting error types, preserve original error in `.map_err()`

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Custom error types with Display/Debug | Implement Error trait manually | `#[derive(Debug, thiserror::Error)]` | thiserror handles boilerplate, less bug-prone |
| Error context wrapping | String concatenation or custom wrapper | `anyhow::Result` at boundaries | anyhow preserves error chain for debugging |
| Emoji parsing validation | Custom parser for Discord emoji syntax | twilight's `RequestReactionType` | Already validated by twilight library |
| Timestamp extraction | Manual string parsing | chrono DateTime parser with error handling | Standard library for date parsing |

**Key insight:** Error handling is deceptively complex — different error types are needed for different contexts (adapter errors vs domain errors). thiserror and anyhow solve this problem; hand-rolling error types leads to inconsistent message formatting and lost error context.

## Common Pitfalls

### Pitfall 1: Silent Failures in Option Returns
**What goes wrong:** `parse_message_line()` returns `Option<Message>`, silently skipping invalid lines. Malformed input files fail silently, making debugging harder.
**Why it happens:** Option is convenient for fallible operations but doesn't distinguish "no match" from "invalid input"
**How to avoid:**
- Distinguish between `None` (no match, ok) and `Err` (invalid format)
- For line parsers, return `Result` if input is expected to be well-formed
- Log warnings when skipping invalid lines: `tracing::warn!("Skipping invalid line: {}", reason)`
**Warning signs:**
- Tests pass but runtime data corruption occurs
- No logged errors for malformed input
- "Surprise" missing data in conversation files

### Pitfall 2: Unwrap-on-Parse Hiding Real Problems
**What goes wrong:** `parse_now()` uses `unwrap_or_else(|_| Utc::now())`, treating all parse failures as "use current time". Invalid timestamps in request become silently masked.
**Why it happens:** Defensive programming — never crash, always have a default
**How to avoid:**
- Return error type if caller must handle the failure
- Only use `unwrap_or()` for recoverable fallbacks that are intentional (not errors)
- Document why fallback is safe: `// Fallback to current time if parsing fails (network clock drift acceptable)`
**Warning signs:**
- Unexpected timestamp values in context
- No error logged when parsing fails
- Hard to debug context timing issues

### Pitfall 3: Losing Error Context Across Crate Boundaries
**What goes wrong:** Error from river-protocol `ParseError` is wrapped as `String` in AdapterError, losing original source location info
**Why it happens:** Easy to write `map_err(|e| AdapterError::Other(e.to_string()))` but that loses context
**How to avoid:**
- Use `#[from]` in error enums to preserve the chain: `#[error("Protocol: {0}")] Protocol(#[from] ParseError)`
- When hand-wrapping, include original error: `format!("{}: {}", context, original_error)`
- Consider using `anyhow::Context` trait: `.context("failed to parse conversation")?`
**Warning signs:**
- Error messages lack file/line info
- Debugging requires re-running code to see actual failure
- Stack trace shows error converted multiple times

### Pitfall 4: Test Panics Instead of Error Assertions
**What goes wrong:** Tests use `panic!()` in match arms (discord.rs line 717) instead of `assert!(false, "...")`. If test fails, message is unclear.
**Why it happens:** Quick debugging — just panic with a message
**How to avoid:**
- Use `assert_eq!()` or `assert!()` for assertions
- Use `.unwrap_or_else(|e| panic!("Expected X, got error: {:?}", e))` for Result tests
- Use `Result`-returning tests with `?` operator in test functions
**Warning signs:**
- Test panic messages are unclear
- Hard to see what was expected vs actual in failure

## Code Examples

### Emoji Parsing with Error Handling

**Current (river-discord line 647):**
```rust
fn parse_emoji(emoji: &str) -> twilight_http::request::channel::reaction::RequestReactionType<'_> {
    if emoji.starts_with('<') && emoji.ends_with('>') {
        let inner = &emoji[1..emoji.len() - 1];
        let parts: Vec<&str> = inner.split(':').collect();
        if parts.len() >= 3 {
            if let Ok(id) = parts[2].parse::<u64>() {
                return twilight_http::request::channel::reaction::RequestReactionType::Custom {
                    id: Id::new(id),
                    name: Some(parts[1]),
                };
            }
        }
    }
    twilight_http::request::channel::reaction::RequestReactionType::Unicode { name: emoji }
}
```

**Improved with error handling:**
```rust
fn parse_emoji(emoji: &str) -> Result<twilight_http::request::channel::reaction::RequestReactionType<'_>, DiscordAdapterError> {
    if emoji.starts_with('<') && emoji.ends_with('>') {
        let inner = &emoji[1..emoji.len() - 1];
        let parts: Vec<&str> = inner.split(':').collect();
        if parts.len() < 3 {
            return Err(DiscordAdapterError::InvalidEmojiFormat(
                format!("Custom emoji must have format <:name:id>, got: {}", emoji)
            ));
        }
        let id = parts[2].parse::<u64>()
            .map_err(|_| DiscordAdapterError::InvalidEmojiFormat(
                format!("Invalid emoji ID: {} (not a number)", parts[2])
            ))?;
        Ok(twilight_http::request::channel::reaction::RequestReactionType::Custom {
            id: Id::new(id),
            name: Some(parts[1]),
        })
    } else {
        Ok(twilight_http::request::channel::reaction::RequestReactionType::Unicode { name: emoji })
    }
}
```

### Message Line Parsing with Line Numbers

**Current (river-protocol line 76):**
```rust
pub fn parse_message_line(line: &str) -> Option<Message> {
    // ... parsing logic returns None on any invalid field ...
}
```

**Improved:**
```rust
pub fn parse_message_line(line: &str, line_number: usize) -> Result<Message, ConversationError> {
    let line = line.trim();
    if line.is_empty() {
        return Err(ConversationError::InvalidMessageLine {
            line_number,
            reason: "empty line".into(),
        });
    }
    // ... return Err for each validation failure with specific reason ...
}
```

### Timestamp Extraction with Error Context

**Current (river-context line 27):**
```rust
fn new(id: &str, message: OpenAIMessage) -> Self {
    let timestamp = extract_timestamp(id).unwrap_or(0);
    Self { timestamp, message }
}
```

**Improved:**
```rust
fn new(id: &str, message: OpenAIMessage) -> Result<Self, ContextError> {
    let timestamp = extract_timestamp(id)
        .map_err(|e| ContextError::InvalidTimestamp(format!("ID {}: {}", id, e)))?;
    Ok(Self { timestamp, message })
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual Error trait impl | `#[derive(Debug, thiserror::Error)]` | Rust 1.65+ | Reduced boilerplate, consistent error messages |
| `Box<dyn Error>` everywhere | Domain-specific error enums | Rust async ecosystem | Better error context, type safety at boundaries |
| Unwrap/expect in production | `?` operator + Result | Rust 2021 edition | Graceful error propagation, stable systems |
| Silent Option failures | Explicit Result for errors | Fallible API design | Better debuggability, no silent data loss |

**Deprecated/outdated:**
- Manual `impl Error` for custom types — thiserror replaces this
- Global error strings — domain-specific enums now standard
- `Result<T, String>` — typed errors preferred for matching

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | thiserror 2.0 and anyhow 1.0 are already in Cargo.lock | Standard Stack | Phase would need dependency audit if versions are incompatible |
| A2 | `parse_emoji()` in discord.rs is the primary panic risk for STAB-01 | Common Pitfalls | May need to audit other emoji handling code paths if this assumption is incomplete |
| A3 | `Conversation::from_str()` is the primary message parsing entry point in river-protocol | Phase Requirements | If parsing happens elsewhere, STAB-02 scope is incomplete |
| A4 | Tests can use `Result`-returning test functions without framework changes | Code Examples | May need test framework upgrades if Tokio test attributes don't support Result |

All other claims in this research were verified through code inspection or explicit pattern matching against existing crates.

## Open Questions

1. **Should invalid lines be errors or skipped silently?**
   - What we know: `parse_message_line()` currently returns Option, skipping invalid input
   - What's unclear: Is malformed data a load error (fail fast) or should we log and continue?
   - Recommendation: Treat as errors in `Conversation::from_str()`, let caller decide: log and skip vs. fail

2. **Which emoji parsing failures are worth error propagation?**
   - What we know: `parse_emoji()` currently defaults to Unicode on custom emoji parse failure
   - What's unclear: Should invalid custom emoji ID be an error, or acceptable fallback behavior?
   - Recommendation: Errors for malformed format, fallback for numeric parse failures (network data, retryable)

3. **Should `parse_now()` error propagate or use current time fallback?**
   - What we know: Used in context assembly, has `unwrap_or_else` fallback
   - What's unclear: Is unparseable timestamp a fatal error or acceptable degrade?
   - Recommendation: Error in Result, let worker decide: log+fallback or fail

## Environment Availability

No external dependencies needed. Cargo.lock already includes thiserror and anyhow. All changes are Rust source refactoring.

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| thiserror | Custom error types | ✓ | 2.0 | None — already required |
| anyhow | Error wrapping at boundaries | ✓ | 1.0 | None — already required |
| serde_yaml | Conversation frontmatter parsing | ✓ | 0.9 | None — already required |

## Validation Architecture

### Test Framework
| Property | Value |
|----------|-------|
| Framework | Rust `#[test]` attribute + tokio::test for async |
| Config file | Cargo.toml (test dependencies) |
| Quick run command | `cargo test -p river-discord -p river-protocol -p river-context --lib` |
| Full suite command | `cargo test` (runs all tests including integration) |

### Phase Requirements → Test Map
| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| STAB-01 | `parse_emoji()` returns error on malformed custom emoji | unit | `cargo test -p river-discord parse_emoji` | ✅ (discord.rs line 710+) |
| STAB-01 | `AddReaction` handler returns error response on invalid emoji | unit | `cargo test -p river-discord execute_impl` | ✅ (discord.rs test coverage) |
| STAB-02 | `Conversation::from_str()` returns ParseError on invalid message line | unit | `cargo test -p river-protocol conversation` | ✅ (mod.rs line 217+) |
| STAB-02 | `parse_message_line()` returns error on incomplete fields | unit | `cargo test -p river-protocol parse_message_line` | ✅ (format.rs tests) |
| STAB-03 | `build_context()` returns error on empty channels | unit | `cargo test -p river-context build_context_empty` | ✅ (assembly.rs line 225+) |
| STAB-03 | `parse_now()` returns error on invalid timestamp string | unit | `cargo test -p river-context parse_now` | ❌ Wave 0 — new test needed |

### Sampling Rate
- **Per task commit:** `cargo test -p river-discord -p river-protocol -p river-context --lib`
- **Per wave merge:** `cargo test` (full suite)
- **Phase gate:** Full suite green + existing tests pass with refactored code

### Wave 0 Gaps
- [ ] `crates/river-context/tests/parse_now_error.rs` — test that invalid timestamp returns error (not fallback)
- [ ] `crates/river-protocol/tests/invalid_message_lines.rs` — test that malformed message lines return specific ParseError variant
- [ ] Update `discord.rs` tests to use `Result` assertions instead of panics

*(Existing test infrastructure covers all phase requirements; new tests formalize error-path coverage)*

## Security Domain

### Applicable ASVS Categories

| ASVS Category | Applies | Standard Control |
|---------------|---------|-----------------|
| V2 Authentication | no | — |
| V3 Session Management | no | — |
| V4 Access Control | no | — |
| V5 Input Validation | yes | Conversation/emoji parsing error handling prevents injection via malformed input |
| V6 Cryptography | no | — |

### Known Threat Patterns for River Stack

| Pattern | STRIDE | Standard Mitigation |
|---------|--------|---------------------|
| Malformed conversation files | Tampering | Explicit error handling in `Conversation::from_str()` prevents silent corruption |
| Invalid emoji injection | Tampering | Validate emoji format in `parse_emoji()` return Result |
| Timestamp injection via IDs | Tampering | Validate timestamp extraction returns error on invalid Snowflake format |

## Sources

### Primary (HIGH confidence)
- Codebase inspection: river-discord/src/discord.rs (emoji parsing functions)
- Codebase inspection: river-protocol/src/conversation/mod.rs (message parsing)
- Codebase inspection: river-protocol/src/conversation/format.rs (format parsing)
- Codebase inspection: river-context/src/assembly.rs (context building)
- Codebase inspection: Cargo.lock (dependency verification)

### Secondary (MEDIUM confidence)
- CONTEXT.md decisions and bounded scope
- REQUIREMENTS.md phase traceability (STAB-01, STAB-02, STAB-03)

## Metadata

**Confidence breakdown:**
- Standard stack (error libraries): HIGH — thiserror and anyhow verified in Cargo.lock, patterns verified in existing crates (river-adapter, river-orchestrator)
- Architecture patterns: HIGH — three crates inspected for current error handling, patterns match Rust ecosystem standards
- Common pitfalls: MEDIUM — identified from code inspection; validation would come from testing phase failures
- Environment availability: HIGH — no external dependencies, all libraries already present

**Research date:** 2026-04-06
**Valid until:** 2026-04-13 (stable error handling patterns, unlikely to change; revalidate if Rust version or major dependency versions change)

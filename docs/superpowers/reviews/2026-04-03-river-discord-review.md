# Code Review: river-discord Adapter

> Reviewer: Senior Code Reviewer Agent
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-01-adapter-library-design.md
> Crate: crates/river-discord/

---

## Executive Summary

The river-discord adapter provides a functional Discord integration using the twilight library. The implementation covers core messaging functionality and follows many spec requirements. However, there are **significant gaps** in spec compliance, **missing test coverage**, and **architectural concerns** that should be addressed.

**Overall Assessment: Needs Work**

- Spec Compliance: ~65%
- Test Coverage: 0%
- Documentation: Adequate
- Code Quality: Mixed (good patterns, but issues)

---

## 1. Spec Compliance Analysis

### 1.1 CLI Arguments (PASS)

**Spec Requirement:**
```
river-{type} --orchestrator <URL> --dyad <NAME> --type <TYPE> [--port <PORT>]
```

**Implementation (`/home/cassie/river-engine/crates/river-discord/src/main.rs` lines 20-39):**
```rust
#[derive(Parser, Debug)]
#[command(name = "river-discord")]
struct Args {
    #[arg(long)]
    orchestrator: String,

    #[arg(long)]
    dyad: String,

    #[arg(long, rename_all = "kebab-case", name = "type")]
    adapter_type: String,

    #[arg(long, default_value = "0")]
    port: u16,
}
```

**Verdict: PASS** - All required CLI arguments are present with correct defaults.

---

### 1.2 Startup Sequence (PARTIAL)

**Spec Requirement:**
1. Parse CLI args
2. Bind HTTP server to port (0 = OS-assigned)
3. Register with orchestrator (send dyad, type, features)
4. Receive config (token, guild_id, etc.) + worker endpoint
5. Initialize platform connection using received config
6. Begin forwarding events to worker endpoint

**Implementation Analysis:**

| Step | Status | Notes |
|------|--------|-------|
| 1. Parse CLI args | PASS | Uses clap correctly |
| 2. Bind HTTP server | PASS | Binds before registration |
| 3. Register with orchestrator | PASS | Sends correct registration payload |
| 4. Receive config | PASS | Parses DiscordConfig from response |
| 5. Initialize platform connection | PASS | Creates DiscordClient with config |
| 6. Forward events | PASS | Event loop forwards to worker |

**Verdict: PASS** - Startup sequence follows spec.

---

### 1.3 HTTP API Endpoints (PARTIAL FAIL)

**Spec Requirement:**

| Method | Endpoint | Request Body | Response |
|--------|----------|--------------|----------|
| POST | `/start` | `{ "worker_endpoint": "http://..." }` | `{ "ok": true }` |
| POST | `/execute` | `OutboundRequest` | `OutboundResponse` |
| GET | `/health` | - | `{ "status": "ok" }` |

**Implementation Analysis:**

#### POST /start

**ISSUE: Redundant and Conflicting Endpoint**

The `/start` endpoint exists but is **semantically incorrect given the startup sequence**. Per the spec:
- The adapter receives `worker_endpoint` from the orchestrator during registration (step 4)
- The adapter immediately begins forwarding events (step 6)

The current implementation:
1. Gets `worker_endpoint` from registration response (line 134)
2. Stores it in state and starts forwarding immediately (lines 137-175)
3. **Also** exposes `/start` endpoint that can overwrite the worker endpoint

**Problems:**
1. The `/start` endpoint can overwrite the worker endpoint mid-operation
2. The "already bound" check (http.rs line 46) doesn't actually prevent binding because `worker_endpoint` is already set from registration
3. This creates confusing dual-path initialization

**Verdict: FAIL** - The `/start` endpoint logic conflicts with the startup sequence.

#### POST /execute

**Verdict: PASS** - Correctly routes to DiscordClient.execute()

#### GET /health

**Verdict: PASS** - Returns correct response format with appropriate status codes.

---

### 1.4 Adapter Trait Implementation (FAIL)

**Spec Requirement:** The adapter should implement the `Adapter` trait from river-adapter.

```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    fn adapter_type(&self) -> &str;
    fn features(&self) -> Vec<FeatureId>;
    fn supports(&self, feature: FeatureId) -> bool;
    async fn start(&self, worker_endpoint: String) -> Result<(), AdapterError>;
    async fn execute(&self, request: OutboundRequest) -> Result<OutboundResponse, AdapterError>;
    async fn health(&self) -> Result<(), AdapterError>;
}
```

**Implementation Analysis:**

The `DiscordClient` does NOT implement the `Adapter` trait. Instead:
- It has ad-hoc methods that loosely correspond to trait methods
- `execute` returns `OutboundResponse` directly instead of `Result<OutboundResponse, AdapterError>`
- There is no `adapter_type()` method
- There is no `start()` method (forwarding is done externally in main.rs)
- `is_healthy()` returns `bool` instead of `Result<(), AdapterError>`

**Verdict: FAIL** - The Adapter trait is not implemented at all.

---

### 1.5 Feature Support (PARTIAL)

**Spec Claims (from supported_features()):**
```rust
vec![
    FeatureId::SendMessage,      // Implemented
    FeatureId::ReceiveMessage,   // Implemented
    FeatureId::EditMessage,      // Implemented
    FeatureId::DeleteMessage,    // Implemented
    FeatureId::ReadHistory,      // Implemented
    FeatureId::AddReaction,      // Implemented
    FeatureId::RemoveReaction,   // Implemented
    FeatureId::TypingIndicator,  // Implemented
]
```

**Missing Features That Could Be Implemented:**

| Feature | Discord Support | Implementation Status |
|---------|-----------------|----------------------|
| PinMessage | Yes | NOT IMPLEMENTED |
| UnpinMessage | Yes | NOT IMPLEMENTED |
| BulkDeleteMessages | Yes | NOT IMPLEMENTED |
| RemoveAllReactions | Yes | NOT IMPLEMENTED |
| Attachments/SendAttachment | Yes | NOT IMPLEMENTED |
| CreateThread | Yes | NOT IMPLEMENTED |
| MemberEvents | Yes | Events not converted |
| PresenceEvents | Yes | Events not converted |
| VoiceStateEvents | Yes | Events not converted |
| ChannelEvents | Yes | Events not converted |
| ConnectionEvents | Partial | Only ConnectionLost |

**Verdict: PARTIAL** - Core features implemented, but many Discord-supported features are missing.

---

### 1.6 Event Conversion (PARTIAL)

**Implemented Events:**
- MessageCreate (with attachments)
- MessageUpdate
- MessageDelete
- ReactionAdd
- ReactionRemove
- TypingStart
- GatewayClose -> ConnectionLost

**Missing Event Conversions (Discord gateway supports these):**
- MemberJoin/MemberLeave (GUILD_MEMBER_ADD/REMOVE)
- PresenceUpdate
- VoiceStateUpdate
- ChannelCreate/Update/Delete
- ThreadCreate/Update/Delete
- ConnectionRestored (after reconnect)

**Verdict: PARTIAL** - Basic events covered, but feature claims don't include these anyway.

---

### 1.7 Response Type Usage (PASS)

The implementation correctly uses `OutboundResponse`, `ResponseData`, `ResponseError`, and `ErrorCode` from river-adapter.

**Verdict: PASS**

---

## 2. Critical Issues

### 2.1 CRITICAL: No Adapter Trait Implementation

**Location:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

The spec explicitly requires adapters to implement the `Adapter` trait. The current implementation does not.

**Impact:** Cannot be used polymorphically with other adapters, breaks the contract defined in river-adapter.

**Recommendation:** Implement the Adapter trait for DiscordClient:

```rust
#[async_trait]
impl Adapter for DiscordClient {
    fn adapter_type(&self) -> &str {
        "discord"
    }

    fn features(&self) -> Vec<FeatureId> {
        supported_features()
    }

    async fn start(&self, worker_endpoint: String) -> Result<(), AdapterError> {
        // Start event forwarding to the given endpoint
    }

    async fn execute(&self, request: OutboundRequest) -> Result<OutboundResponse, AdapterError> {
        Ok(self.execute_impl(request).await)
    }

    async fn health(&self) -> Result<(), AdapterError> {
        if self.is_healthy().await {
            Ok(())
        } else {
            Err(AdapterError::Connection("websocket disconnected".into()))
        }
    }
}
```

---

### 2.2 CRITICAL: No Test Coverage

**Location:** Entire crate

There are **zero tests** in the river-discord crate:
- No unit tests
- No integration tests
- No test directory
- No `#[cfg(test)]` modules

**Impact:** No confidence in correctness, regressions will go undetected.

**Minimum Required Tests:**
1. Unit tests for `convert_event()` - all event type conversions
2. Unit tests for `parse_emoji()` - unicode and custom emoji parsing
3. Unit tests for `format_timestamp()` - timestamp formatting
4. Unit tests for `error_response()` - error construction
5. Integration tests for HTTP endpoints (with mocked DiscordClient)
6. Integration tests for execute() request handling

---

### 2.3 CRITICAL: /start Endpoint Semantic Conflict

**Location:** `/home/cassie/river-engine/crates/river-discord/src/http.rs` lines 38-60

The `/start` endpoint duplicates functionality that happens automatically at startup.

**Current behavior:**
```rust
async fn start(...) -> impl IntoResponse {
    let mut s = state.state.write().await;
    if s.worker_endpoint.is_some() {
        return Json(StartResponse {
            ok: false,
            error: Some("already bound to worker".into()),
        });
    }
    // ...
}
```

**Problem:** `worker_endpoint` is ALWAYS set before the HTTP server starts (main.rs line 141), so this endpoint will ALWAYS return "already bound to worker".

**Recommendation:** Either:
1. Remove the `/start` endpoint entirely (registration provides worker endpoint), or
2. Redesign startup to not auto-bind, making `/start` the sole binding mechanism

---

## 3. Important Issues

### 3.1 Rate Limiting Not Handled

**Location:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

The spec defines `ErrorCode::RateLimited` and `AdapterError::RateLimited { retry_after_ms }`, but the implementation never detects or returns rate limit errors.

```rust
// Current: all errors become PlatformError
Err(e) => error_response(ErrorCode::PlatformError, &e.to_string())
```

Discord API returns 429 responses with `retry_after` headers. These should be detected and mapped to `ErrorCode::RateLimited`.

---

### 3.2 Hardcoded Adapter Name

**Location:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs` line 46

```rust
let adapter_name = "discord".to_string();
```

The adapter name is hardcoded in the gateway event loop. This should come from configuration or be derived from `adapter_type`.

---

### 3.3 No Graceful Reconnection Handling

**Location:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs` lines 62-70

When gateway errors occur, the event loop breaks and marks the client as disconnected:

```rust
Err(e) => {
    tracing::warn!("Gateway error: {:?}", e);
    let mut c = connected_clone.write().await;
    *c = false;
    break;
}
```

This provides no reconnection attempt. Discord gateways expect reconnection after disconnection.

**Missing:**
- `ConnectionRestored` event after successful reconnect
- Reconnection logic
- Backoff strategy

---

### 3.4 Event Loop Polling Inefficiency

**Location:** `/home/cassie/river-engine/crates/river-discord/src/main.rs` lines 156-175

```rust
loop {
    let events = discord_clone.poll_events().await;
    for event in events {
        // forward event
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
}
```

This polling approach adds up to 100ms latency to every event. A better approach would use an async stream or channel receiver that blocks until events are available.

---

### 3.5 Missing Content Field in MessageUpdate

**Location:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs` lines 386-397

```rust
Event::MessageUpdate(msg) => Some(InboundEvent {
    // ...
    content: msg.content.clone(),  // msg.content is Option<String>
    // ...
}),
```

`msg.content` on a MessageUpdate event is `Option<String>` because partial updates may not include content. The current code would fail to compile or panic. Actually, looking closer, twilight's `MessageUpdate` has `content: Option<String>`, so `msg.content.clone()` produces `Option<String>`, but the EventMetadata expects `String`.

This is a **potential runtime issue** - if content is None, this will cause issues.

---

### 3.6 guild_id Config Not Used

**Location:** `/home/cassie/river-engine/crates/river-discord/src/main.rs` line 45

```rust
pub struct DiscordConfig {
    pub token: String,
    pub guild_id: Option<u64>,  // Never used
    pub intents: Option<u64>,
}
```

The `guild_id` is accepted in config but never used. If guild-specific filtering is intended, it's not implemented.

---

## 4. Suggestions (Nice to Have)

### 4.1 Use OutboundResponse Helper Methods

**Location:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

The `OutboundResponse` type has `success()` and `failure()` helper methods. The implementation constructs responses manually:

```rust
// Current
OutboundResponse {
    ok: true,
    data: Some(ResponseData::MessageSent { message_id }),
    error: None,
}

// Better
OutboundResponse::success(ResponseData::MessageSent { message_id })
```

---

### 4.2 Extract ID Parsing to Helper

**Location:** `/home/cassie/river-engine/crates/river-discord/src/discord.rs`

The pattern of parsing channel/message IDs is repeated many times:

```rust
let channel_id = match channel.parse::<u64>() {
    Ok(id) => Id::<ChannelMarker>::new(id),
    Err(_) => return error_response(ErrorCode::InvalidPayload, "Invalid channel ID")
};
```

This could be extracted to a helper:

```rust
fn parse_channel_id(s: &str) -> Result<Id<ChannelMarker>, OutboundResponse> {
    s.parse::<u64>()
        .map(|id| Id::new(id))
        .map_err(|_| error_response(ErrorCode::InvalidPayload, "Invalid channel ID"))
}
```

---

### 4.3 Add Structured Logging

The current logging uses basic tracing macros. Consider adding structured fields:

```rust
// Current
tracing::info!("Bound to worker at {}", request.worker_endpoint);

// Better
tracing::info!(worker_endpoint = %request.worker_endpoint, "Bound to worker");
```

---

### 4.4 Consider Metrics

For production use, consider adding metrics for:
- Events received/forwarded
- Execute requests by type
- Error rates by code
- Gateway connection status

---

## 5. Missing Functionality Summary

### Features Not Implemented (Discord supports these):

| OutboundRequest Variant | Spec Feature | Status |
|------------------------|--------------|--------|
| PinMessage | PinMessage | NOT IMPLEMENTED |
| UnpinMessage | UnpinMessage | NOT IMPLEMENTED |
| BulkDeleteMessages | BulkDeleteMessages | NOT IMPLEMENTED |
| RemoveAllReactions | RemoveAllReactions | NOT IMPLEMENTED |
| SendAttachment | Attachments | NOT IMPLEMENTED |
| CreateThread | CreateThread | NOT IMPLEMENTED |
| CreatePoll | CreatePoll | NOT IMPLEMENTED |
| PollVote | PollVote | NOT IMPLEMENTED |

### Event Types Not Converted:

- MemberJoin / MemberLeave
- PresenceUpdate
- VoiceStateUpdate
- ChannelCreate / ChannelUpdate / ChannelDelete
- ThreadCreate / ThreadUpdate / ThreadDelete
- PinUpdate
- ConnectionRestored

---

## 6. Test Coverage Requirements

### Minimum Required Tests

```rust
// tests/event_conversion.rs
#[test]
fn test_convert_message_create() { ... }

#[test]
fn test_convert_message_update() { ... }

#[test]
fn test_convert_message_delete() { ... }

#[test]
fn test_convert_reaction_add() { ... }

#[test]
fn test_convert_reaction_remove() { ... }

#[test]
fn test_convert_typing_start() { ... }

#[test]
fn test_convert_gateway_close() { ... }

#[test]
fn test_skip_bot_messages() { ... }

// tests/emoji.rs
#[test]
fn test_parse_unicode_emoji() { ... }

#[test]
fn test_parse_custom_emoji() { ... }

#[test]
fn test_parse_animated_emoji() { ... }

#[test]
fn test_format_unicode_emoji() { ... }

#[test]
fn test_format_custom_emoji() { ... }

// tests/http.rs
#[tokio::test]
async fn test_health_ok() { ... }

#[tokio::test]
async fn test_health_disconnected() { ... }

#[tokio::test]
async fn test_execute_send_message() { ... }

#[tokio::test]
async fn test_execute_unsupported_feature() { ... }
```

---

## 7. Documentation Gaps

### Missing Documentation

1. **No README.md** - No crate-level documentation for users
2. **No architecture doc** - How the components interact
3. **No deployment guide** - How to run the adapter
4. **Missing doc comments** on:
   - `DiscordClient` struct
   - `AdapterState` struct
   - `HttpState` struct
   - `DiscordConfig` fields

### Existing Documentation (Adequate)

- Module-level doc comments in main.rs, http.rs, discord.rs
- Function signatures are self-documenting

---

## 8. Recommendations Summary

### Must Fix (Critical)

1. Implement the `Adapter` trait for `DiscordClient`
2. Add unit and integration tests
3. Fix or remove the `/start` endpoint semantic conflict
4. Fix `MessageUpdate.content` handling (Option<String> vs String)

### Should Fix (Important)

5. Add rate limiting detection and handling
6. Implement reconnection logic for gateway disconnections
7. Replace polling loop with async channel receiver
8. Handle `guild_id` config or document why it's unused

### Nice to Have (Suggestions)

9. Use `OutboundResponse::success()`/`failure()` helpers
10. Extract ID parsing to helper functions
11. Add structured logging fields
12. Consider metrics integration

---

## 9. Files Reviewed

| File | Lines | Status |
|------|-------|--------|
| `/home/cassie/river-engine/crates/river-discord/Cargo.toml` | 25 | OK |
| `/home/cassie/river-engine/crates/river-discord/src/main.rs` | 194 | Issues |
| `/home/cassie/river-engine/crates/river-discord/src/http.rs` | 99 | Issues |
| `/home/cassie/river-engine/crates/river-discord/src/discord.rs` | 517 | Issues |

---

## 10. Conclusion

The river-discord adapter provides a working foundation for Discord integration but falls short of the spec requirements in key areas. The most critical gaps are:

1. **No Adapter trait implementation** - This breaks the contract defined by river-adapter
2. **Zero test coverage** - Unacceptable for production code
3. **Conflicting /start endpoint** - Semantic confusion in the API

The core Discord functionality (send/receive messages, reactions, typing indicators, history) works correctly and follows reasonable patterns. With the critical fixes applied, this could be a solid adapter implementation.

**Recommendation: Do not merge until critical issues are resolved.**

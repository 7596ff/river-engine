# river-discord Brutal Review

> Reviewer: Claude (no subagents)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-01-adapter-library-design.md (adapter binary contract)
>
> Note: No dedicated river-discord spec exists. Review based on adapter library spec.

## Spec Completion Assessment

### CLI - PASS

| Spec Requirement | Implemented | Notes |
|------------------|-------------|-------|
| --orchestrator <URL> | YES | |
| --dyad <NAME> | YES | |
| --type <TYPE> | YES | Named `adapter_type` internally |
| --port <PORT> | YES | Default 0 for OS-assigned |

### HTTP Endpoints - PASS

| Endpoint | Implemented | Notes |
|----------|-------------|-------|
| POST /start | YES | |
| POST /execute | YES | |
| GET /health | YES | Returns 503 when disconnected |

### Startup Sequence - PASS

| Step | Implemented | Notes |
|------|-------------|-------|
| Parse CLI args | YES | |
| Bind HTTP server | YES | Port 0 supported |
| Register with orchestrator | YES | |
| Receive config + worker endpoint | YES | |
| Initialize platform connection | YES | Uses twilight |
| Forward events to worker | YES | |

### Feature Implementation - PARTIAL

| Feature | Declared | Implemented | Notes |
|---------|----------|-------------|-------|
| SendMessage | YES | YES | With reply support |
| ReceiveMessage | YES | YES | Via gateway events |
| EditMessage | YES | YES | |
| DeleteMessage | YES | YES | |
| ReadHistory | YES | YES | With before/limit |
| AddReaction | YES | YES | Custom + unicode |
| RemoveReaction | YES | YES | |
| TypingIndicator | YES | YES | |
| PinMessage | NO | NO | |
| UnpinMessage | NO | NO | |
| BulkDeleteMessages | NO | NO | |
| RemoveAllReactions | NO | NO | |
| Attachments | NO | NO | SendAttachment not implemented |
| CreateThread | NO | NO | |
| CreatePoll | NO | NO | |
| PollVote | NO | NO | |

### Event Forwarding - PARTIAL

| Event | Forwarded | Notes |
|-------|-----------|-------|
| MessageCreate | YES | Filters bot messages |
| MessageUpdate | YES | |
| MessageDelete | YES | |
| ReactionAdd | YES | |
| ReactionRemove | YES | |
| TypingStart | YES | |
| ConnectionLost | YES | Via GatewayClose |
| ConnectionRestored | NO | Not emitted |
| MemberJoin | NO | |
| MemberLeave | NO | |
| PresenceUpdate | NO | |
| VoiceStateUpdate | NO | |
| ChannelCreate/Update/Delete | NO | |
| ThreadCreate/Update/Delete | NO | |
| PinUpdate | NO | |
| PollVote | NO | |
| ScheduledEvent | NO | |

## IMPORTANT ISSUES

### 1. Event polling instead of direct forwarding

**Implementation:**
```rust
let event_task = tokio::spawn(async move {
    loop {
        let events = discord_clone.poll_events().await;
        for event in events {
            // forward...
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
});
```

Events are queued in a channel and polled every 100ms. This adds latency. Better pattern: forward directly from the gateway event loop.

**Verdict:** Suboptimal but functional. 100ms latency acceptable.

### 2. No reconnection handling

**Spec implies:**
> ConnectionRestored { downtime_seconds: u64 }

**Implementation:**
- GatewayReconnect event is logged but not forwarded
- After gateway error, `connected` flag is set false and loop exits
- No automatic reconnection attempt

**Verdict:** Adapter dies on disconnect. Should reconnect or emit ConnectionLost with `reconnecting: false`.

### 3. Adapter trait not implemented

**Spec says adapters should implement the trait:**
```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    fn adapter_type(&self) -> &str;
    fn features(&self) -> Vec<FeatureId>;
    ...
}
```

**Implementation:** `DiscordClient` does not implement the `Adapter` trait. The trait exists in river-adapter but isn't used.

**Verdict:** Spec violation. The trait provides a standard interface. Implementation took shortcuts.

### 4. No /start endpoint state check

**Spec says:**
> Response 400 (already started): `{ "ok": false, "error": "already bound to worker" }`

**Implementation checks this but:** Worker endpoint is set during registration, BEFORE /start is called. So /start will always return "already bound" error.

Looking at the flow:
1. Register with orchestrator → receives worker_endpoint
2. Store worker_endpoint in state
3. /start endpoint checks if worker_endpoint exists → always true

**Verdict:** BUG. /start will always fail because endpoint is pre-set.

### 5. MessageUpdate content might be None

**twilight_model::channel::Message** has `content` as non-optional but **MessageUpdate** only has what changed. Need to handle missing content.

```rust
Event::MessageUpdate(msg) => Some(InboundEvent {
    ...
    content: msg.content.clone(),  // May be stale if only other fields changed
```

Should check if this is actually the updated content or if it needs explicit handling.

## MINOR ISSUES

### 6. Hardcoded adapter name

```rust
let adapter_name = "discord".to_string();
```

Should use `args.adapter_type` for consistency.

### 7. No rate limit handling

Discord API responses can include rate limit headers. Implementation doesn't check for or handle `ErrorCode::RateLimited`.

### 8. Channel buffer size hardcoded

```rust
let (event_tx, event_rx) = mpsc::channel::<InboundEvent>(256);
```

256 events. If worker is slow, events will back up. Should be configurable or use unbounded.

### 9. No tests

Zero test coverage. Should at least test:
- Event conversion
- Emoji parsing
- Error response construction

### 10. Signal handling aborts tasks

```rust
tokio::signal::ctrl_c().await?;
event_task.abort();
server.abort();
```

Abrupt abort. Should signal graceful shutdown and wait for clean exit.

### 11. Missing side argument

**Spec (adapter library design) and orchestrator spawn:**
```bash
river-discord --orchestrator URL --dyad NAME --side left --type discord
```

**Implementation CLI:**
```rust
#[arg(long)]
dyad: String,

#[arg(long, rename_all = "kebab-case", name = "type")]
adapter_type: String,
```

No `--side` argument. However, registration request does include side in `AdapterRegistration`. This may be an issue if orchestrator expects `--side`.

Actually, looking at orchestrator supervisor.rs:
```rust
.arg("--type")
.arg(&adapter_config.adapter_type)
```

Orchestrator passes `--type` but not `--side`. However, AdapterConfig includes `side`. This is a mismatch between what orchestrator spawns and what adapter needs.

**Verdict:** Potential integration issue. Registration works but side info may be lost.

## Code Quality Assessment

### Strengths

1. **Uses twilight** - Modern, async Discord library
2. **Clean event conversion** - Comprehensive mapping
3. **Custom + unicode emoji handling** - Properly parses both formats
4. **Bot message filtering** - Prevents feedback loops
5. **Proper error responses** - Uses river_adapter error codes
6. **Health endpoint** - Returns 503 when disconnected
7. **Logging** - Good tracing throughout
8. **Timestamp handling** - Proper chrono conversion

### Weaknesses

1. **No Adapter trait** - Doesn't implement the spec trait
2. **Polling events** - Adds 100ms latency
3. **No reconnection** - Dies on disconnect
4. **No tests** - Zero coverage
5. **Limited features** - Only 8 of 24 features
6. **Abrupt shutdown** - Tasks aborted, not graceful

## Summary

| Category | Score | Notes |
|----------|-------|-------|
| Spec Completion | 75% | CLI/HTTP match, trait not implemented |
| Feature Coverage | 33% | 8 of 24 features |
| Event Coverage | 35% | 7 of 20 event types |
| Code Quality | 70% | Works but has issues |
| Testing | 0% | No tests |

### Blocking Issues

1. **/start endpoint always fails** - BUG: worker_endpoint pre-set during registration
2. **No Adapter trait implementation** - Spec violation
3. **No reconnection** - Adapter dies on disconnect

### Recommended Actions

1. Fix /start endpoint logic - either don't pre-set worker_endpoint or remove /start requirement
2. Implement `Adapter` trait on `DiscordClient`
3. Add reconnection logic for gateway disconnects
4. Forward events directly instead of polling
5. Add more features (at least Attachments, CreateThread)
6. Add more events (ConnectionRestored, channel events)
7. Handle rate limiting from Discord API
8. Add tests for event conversion and emoji parsing
9. Implement graceful shutdown (signal tasks, wait for clean exit)
10. Add `--side` argument if needed for orchestrator integration

# river-orchestrator Brutal Review

> Reviewer: Claude (no subagents)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-01-orchestrator-design.md

## Spec Completion Assessment

### Module Structure - PASS

| Spec Requirement | Implemented | Notes |
|------------------|-------------|-------|
| main.rs | YES | |
| config.rs | YES | |
| registry.rs | YES | |
| supervisor.rs | YES | |
| respawn.rs | YES | |
| http.rs | YES | |
| model.rs | YES | |

### HTTP Endpoints - PASS

| Endpoint | Implemented | Notes |
|----------|-------------|-------|
| POST /register | YES | |
| POST /model/switch | YES | |
| POST /switch_roles | YES | |
| POST /worker/output | YES | |
| GET /registry | YES | |
| GET /health | YES | |

### Features - PARTIAL

| Feature | Implemented | Notes |
|---------|-------------|-------|
| CLI parsing | YES | |
| Config loading with env substitution | YES | |
| Process spawning | YES | |
| Health checks | YES | 60s interval |
| Registry push | YES | |
| Worker registration | YES | |
| Adapter registration | YES | |
| Embed registration | YES | |
| Model switching | YES | |
| Role switching | PARTIAL | **Missing two-phase commit** |
| Respawn policy | YES | |
| Graceful shutdown | YES | Signal handling with timeout |
| Feature validation | **NO** | Missing required feature check |

## CRITICAL ISSUES

### 1. Role switching is NOT two-phase commit

**Spec requires full protocol:**
```
Worker                 Orchestrator                Partner
   |--POST /switch_roles-->|                           |
   |                       | [acquire dyad lock]       |
   |                       |--POST /prepare_switch---->|
   |                       |<--{"ready":true}----------|
   |                       |--POST /prepare_switch---->| (to initiator too)
   |<--prepare_switch------|                           |
   |--{"ready":true}------>|                           |
   |                       | [both ready, commit]      |
   |                       |--POST /commit_switch----->|
   |<--commit_switch-------|                           |
   |                       | [update registry]         |
   |<--{"switched":true}---|                           |
```

**Implementation:**
```rust
async fn handle_switch_roles(...) {
    // ... just updates state, no prepare/commit protocol
    let mut reg = state.registry.write().await;
    // Directly swaps batons
    let (your_new, partner_new) = if current_left_baton == Baton::Actor {
        (Baton::Spectator, Baton::Actor)
    } else {
        (Baton::Actor, Baton::Spectator)
    };
    // No calls to workers at all!
}
```

The implementation just swaps batons in the registry. It does NOT:
- Call `POST /prepare_switch` on either worker
- Wait for readiness responses
- Call `POST /commit_switch` to commit
- Handle atomicity or rollback

**Verdict:** CRITICAL SPEC VIOLATION. Role switching is not atomic and workers aren't notified.

### 2. No adapter feature validation

**Spec requires:**
```rust
fn validate_adapter_features(features: &[u16]) -> Result<Vec<FeatureId>, RegistrationError> {
    // Required features
    if !parsed.contains(&FeatureId::SendMessage) {
        return Err(RegistrationError::MissingFeature(FeatureId::SendMessage));
    }
    if !parsed.contains(&FeatureId::ReceiveMessage) {
        return Err(RegistrationError::MissingFeature(FeatureId::ReceiveMessage));
    }
}
```

**Implementation:** No validation. Features are accepted as-is:
```rust
processes.push(ProcessEntry::Adapter {
    endpoint: req.endpoint.clone(),
    adapter_type: adapter.adapter_type.clone(),
    dyad: adapter.dyad.clone(),
    side: adapter.side.clone(),
    features: adapter.features.clone(),  // No validation!
});
```

**Verdict:** SPEC VIOLATION. Adapters missing required features are accepted.

### 3. ProcessEntry format differs from spec

**Spec says:**
```rust
#[serde(untagged)]
pub enum ProcessEntry {
    Worker { ... },
    Adapter { ... },
    EmbedService { ... },
}
```

**Implementation in river-protocol:**
```rust
#[serde(tag = "entry_type", rename_all = "snake_case")]
pub enum ProcessEntry {
    Worker { ... },
    Adapter { ... },
    Embed { ... },
}
```

Uses `#[serde(tag = "entry_type")]` (tagged) instead of `#[serde(untagged)]`. This changes the JSON format:

Spec expects:
```json
{ "endpoint": "...", "dyad": "...", "side": "left", ... }
```

Implementation produces:
```json
{ "entry_type": "worker", "endpoint": "...", "dyad": "...", "side": "left", ... }
```

**Verdict:** API INCOMPATIBILITY. Consumers expecting untagged format will fail.

## IMPORTANT ISSUES

### 4. No dyad lock for role switching

**Spec says:**
> - Dyad lock prevents concurrent switch attempts
> - First switch request acquires lock
> - Concurrent requests get 409 "switch_in_progress"

**Implementation:** No lock. Multiple concurrent switch requests could interleave and corrupt state.

### 5. WorkerOutput format differs from spec

**Spec says:**
```rust
pub struct WorkerOutput {
    pub status: ExitStatus,
    pub summary: String,
}
```

**Implementation:**
```rust
pub struct WorkerOutput {
    pub dyad: String,
    pub side: Side,
    pub status: ExitStatus,
    pub summary: String,
}
```

Added `dyad` and `side` fields. This is arguably better (self-identifying), but diverges from spec.

### 6. Health check interval hardcoded

**Spec says:**
> `GET /health` to every process every 60 seconds

**Implementation:** Hardcoded 60 seconds, but should be configurable.

### 7. No thiserror as specified

**Spec dependencies:**
```toml
thiserror = { workspace = true }
```

**Implementation Cargo.toml:** No thiserror. Errors implemented manually.

### 8. No logging of unknown worker output

**Spec says:**
> Worker output for unknown worker → log warning, ignore

**Implementation:** Returns 404 error but doesn't log a warning:
```rust
return (
    StatusCode::NOT_FOUND,
    Json(OutputAck { acknowledged: false }),
);
```

### 9. shutdown timeout hardcoded

**Implementation:**
```rust
supervisor.shutdown(Duration::from_secs(300)).await;  // 5 minutes
```

This matches spec but should be configurable.

## MINOR ISSUES

### 10. Adapter side field not in spec

ProcessEntry::Adapter has a `side` field not in the spec. May be intentional enhancement.

### 11. No validation on config model references

**Spec says:**
> Unknown model reference in worker config → exit with error on startup

**Implementation:** No validation that `left_model` and `right_model` exist in the models map at config load time.

### 12. respawn.rs has tests, supervisor.rs does not

Good: respawn.rs has unit tests for exit status handling.
Bad: supervisor.rs (critical process management) has no tests.

### 13. Uses river-adapter instead of river-protocol

```rust
use river_adapter::Side;
```

Should use `river_protocol::Side` for consistency. `river-adapter` re-exports it but creates coupling.

## Code Quality Assessment

### Strengths

1. **Clean module separation** - Each concern isolated
2. **Good respawn logic** - Correctly implements all 4 exit status cases
3. **Proper async/await** - tokio patterns correct
4. **Signal handling** - Uses tokio::signal for graceful shutdown
5. **Health check recovery** - Resets failure count on success
6. **Registry push on change** - Correctly pushes to all processes
7. **env var substitution** - Works correctly with regex replacement
8. **Respawn tests** - Unit tests for exit status handling

### Weaknesses

1. **No two-phase commit** - Major protocol missing
2. **No feature validation** - Adapters not validated
3. **No dyad lock** - Role switch race conditions
4. **Light testing** - Only respawn.rs tested
5. **Hardcoded values** - Timeouts, intervals not configurable
6. **Import inconsistency** - river-adapter vs river-protocol

## Summary

| Category | Score | Notes |
|----------|-------|-------|
| Spec Completion | 60% | Missing two-phase commit, feature validation |
| Code Quality | 70% | Clean but incomplete |
| Documentation | 50% | Module docs only |
| Testing | 30% | Only respawn tested |

### Blocking Issues

1. **Role switching not two-phase commit** - Workers not notified of switch
2. **No feature validation** - Adapters with missing features accepted
3. **No dyad lock** - Concurrent switches can corrupt state
4. **ProcessEntry tagging differs** - API incompatibility

### Recommended Actions

1. Implement full two-phase commit for role switching:
   - Add dyad lock
   - Call /prepare_switch on both workers
   - Wait for ready responses
   - Call /commit_switch on both
   - Handle failures with rollback
2. Add adapter feature validation (SendMessage, ReceiveMessage required)
3. Change ProcessEntry to `#[serde(untagged)]` or update spec
4. Add dyad lock (tokio::sync::Mutex or RwLock per dyad)
5. Validate model references at config load time
6. Add supervisor.rs tests
7. Make timeouts/intervals configurable
8. Use river-protocol consistently

# Code Review: river-orchestrator

**Reviewer:** Code Review Agent
**Date:** 2026-04-03
**Crate:** `crates/river-orchestrator/`
**Spec:** `docs/superpowers/specs/2026-04-01-orchestrator-design.md`

---

## Executive Summary

The river-orchestrator implementation is **largely complete** and demonstrates solid architectural decisions. The crate compiles successfully and covers most of the specified functionality. However, there are several **critical gaps**, **spec deviations**, and **code quality issues** that need attention before this is production-ready.

**Overall Assessment:** 70% spec compliance, needs work on tests, error handling, and some missing features.

---

## 1. Spec Compliance Analysis

### 1.1 Crate Structure

| Requirement | Status | Notes |
|-------------|--------|-------|
| `main.rs` - CLI parsing, startup | PASS | Correctly implements clap CLI with `-c` and `-p` flags |
| `config.rs` - Config loading, env vars | PASS | Env var substitution implemented with regex |
| `registry.rs` - Registry state, push | PASS | Clean implementation with proper async |
| `supervisor.rs` - Spawning, health checks | PASS | All functionality present |
| `respawn.rs` - Respawn policy, wake timers | PASS | Implements all four respawn behaviors |
| `http.rs` - Axum server, endpoints | PASS | All endpoints present |
| `model.rs` - Model config resolution | PASS | Simple but correct |

### 1.2 Dependencies

| Spec Dependency | Actual | Status |
|-----------------|--------|--------|
| `river-adapter` | Present | PASS |
| `tokio` | Present | PASS |
| `axum` | Present | PASS |
| `reqwest` | Present | PASS |
| `serde` | Present | PASS |
| `serde_json` | Present | PASS |
| `clap` | Present | PASS |
| `thiserror` | **MISSING** | FAIL - Uses manual error impl |

**Issue:** Spec requires `thiserror` but implementation uses manual `Display` and `Error` impls. This is technically fine but deviates from spec.

**Additional dependencies not in spec:**
- `river-protocol` - Added correctly for shared types
- `tracing` / `tracing-subscriber` - Good addition for logging
- `tower-http` - Good addition for HTTP tracing
- `regex` - Used for env var substitution

### 1.3 CLI Interface

| Requirement | Status | Notes |
|-------------|--------|-------|
| `-c, --config <PATH>` | PASS | Default "river.json" |
| `-p, --port <PORT>` | PASS | Overrides config port |
| `-h, --help` | PASS | Provided by clap |

### 1.4 Configuration

| Requirement | Status | Notes |
|-------------|--------|-------|
| `Config` struct | PASS | All fields present |
| `ModelConfig` struct | PASS | Has endpoint, name, api_key, context_limit, dimensions |
| `EmbedConfig` struct | PASS | Has model reference |
| `DyadConfig` struct | **PARTIAL** | See deviation below |
| `AdapterConfig` struct | **PARTIAL** | Has extra `side` field not in spec |
| Env var substitution | PASS | `$VAR_NAME` syntax works |
| Validation | PASS | Checks model refs, context_limit, dimensions |

**DEVIATION - DyadConfig.initial_actor:**
Spec defines `left_starts_as: Baton` but implementation has `initial_actor: Side`.

```rust
// Spec:
pub struct DyadConfig {
    pub left_starts_as: Baton,  // which baton left worker starts with
    // ...
}

// Implementation (config.rs line 45):
pub struct DyadConfig {
    pub initial_actor: Side,    // DIFFERENT SEMANTICS
    // ...
}
```

This is semantically equivalent but breaks config file compatibility. Users cannot use the spec's JSON format.

**DEVIATION - AdapterConfig.side:**
Spec does not include a `side` field on AdapterConfig, but implementation has:

```rust
// Implementation (config.rs line 56):
pub struct AdapterConfig {
    pub side: river_adapter::Side,  // NOT IN SPEC
    // ...
}
```

This may be intentional (adapters per worker?) but undocumented.

### 1.5 HTTP API Endpoints

| Endpoint | Status | Compliance |
|----------|--------|------------|
| `POST /register` | PASS | Handles Worker, Adapter, Embed |
| `POST /model/switch` | PASS | Returns ModelConfig |
| `POST /switch_roles` | PASS | Two-phase protocol |
| `POST /worker/output` | PASS | Handles all exit statuses |
| `GET /registry` | PASS | Returns full registry |
| `GET /health` | PASS | Returns counts |

### 1.6 Registration Protocol

**Worker Registration:**

| Field | Request | Response | Status |
|-------|---------|----------|--------|
| endpoint | PASS | N/A | |
| dyad | PASS | N/A | |
| side | PASS | N/A | |
| accepted | N/A | PASS | |
| baton | N/A | PASS | |
| partner_endpoint | N/A | PASS | |
| model | N/A | PASS | Different type (see below) |
| ground | N/A | PASS | |
| workspace | N/A | PASS | |
| initial_message | N/A | PASS | |
| start_sleeping | N/A | PASS | |

**DEVIATION - Worker model response type:**

Spec shows `model` as full `ModelConfig`:
```json
"model": {
  "endpoint": "...",
  "name": "...",
  "api_key": "...",
  "context_limit": 200000
}
```

Implementation defines `WorkerModelConfig` (http.rs lines 60-66):
```rust
pub struct WorkerModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,
    pub context_limit: usize,  // Not Option<usize>
}
```

This is actually correct - spec shows resolved context_limit without Option. Minor win.

**Adapter Registration:**

| Field | Request | Response | Status |
|-------|---------|----------|--------|
| endpoint | PASS | N/A | |
| dyad | PASS | PASS | |
| type | PASS | N/A | |
| features | PASS | PASS (validated_features) | |
| accepted | N/A | PASS | |
| worker_endpoint | N/A | **OPTIONAL** | Spec shows required, impl is Option |
| config | N/A | PASS | |

**Issue:** Adapter registration response has `worker_endpoint: Option<String>` but spec shows it as required `String`. If actor worker hasn't registered yet, adapter gets `null`.

**Feature Validation:**
- PASS: Validates required features (SendMessage, ReceiveMessage)
- PASS: Rejects unknown feature IDs

**Embed Registration:** PASS - All fields correct.

### 1.7 Registry

| Requirement | Status | Notes |
|-------------|--------|-------|
| ProcessEntry::Worker | **PARTIAL** | See below |
| ProcessEntry::Adapter | **PARTIAL** | See below |
| ProcessEntry::EmbedService | PASS | Matches spec |
| Push on change | PASS | After every registration/update |

**DEVIATION - ProcessEntry serde format:**

Spec shows `#[serde(untagged)]` enum (discriminated by field presence):
```rust
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProcessEntry {
    Worker { ... },
    Adapter { ... },
    EmbedService { ... },
}
```

river-protocol (registry.rs) uses tagged:
```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessEntry { ... }
```

This produces different JSON:
```json
// Spec (untagged):
{"endpoint": "...", "dyad": "...", "side": "left", ...}

// Implementation (tagged):
{"type": "worker", "endpoint": "...", "dyad": "...", "side": "left", ...}
```

**This breaks wire compatibility** with any clients expecting spec format.

### 1.8 Startup Sequence

| Step | Status | Notes |
|------|--------|-------|
| 1. Parse CLI, load config | PASS | |
| 2. Resolve env vars | PASS | |
| 3. Bind HTTP server | PASS | |
| 4. Spawn embed service | PASS | Spawns if configured |
| 5a. Spawn left worker | PASS | |
| 5b. Spawn right worker | PASS | |
| 5c. Spawn adapters | PASS | |
| 6. Actor waits for /notify | N/A | Worker responsibility |
| 7. Spectator waits for /flash | N/A | Worker responsibility |
| 8. Enter supervision loop | PASS | |

### 1.9 Process Supervision

| Requirement | Status | Notes |
|-------------|--------|-------|
| Health check every 60s | PASS | Configurable interval |
| Dead after 3 failures | PASS | 3 consecutive failures |
| Remove dead from registry | PASS | |
| Respawn workers | PASS | |
| Respawn adapters | PASS | |
| Graceful shutdown | **PARTIAL** | See below |

**DEVIATION - Graceful shutdown:**

Spec requires:
1. Send SIGTERM to workers
2. Workers call `summary` tool and exit
3. Wait up to 5 minutes
4. SIGKILL remaining

Implementation (supervisor.rs lines 186-194, 198-234):
- Uses `start_kill()` which is platform-specific
- On Unix this is SIGKILL, not SIGTERM
- No signal handling for workers to clean up

```rust
pub async fn terminate_all(&mut self) {
    for (key, handle) in &mut self.processes {
        if let Err(e) = handle.child.start_kill() {  // This is SIGKILL!
            // ...
        }
    }
}
```

**This is a bug.** Workers will be killed immediately without chance to write summaries.

### 1.10 Worker Output and Respawn

| Exit Status | Expected Action | Implementation | Status |
|-------------|-----------------|----------------|--------|
| Done { wake_after_minutes: None } | Respawn with start_sleeping: true | PASS | |
| Done { wake_after_minutes: Some(N) } | Wait N minutes, respawn with summary | PASS | |
| ContextExhausted | Respawn immediately with summary | PASS | |
| Error | Respawn immediately from JSONL | PASS | |

### 1.11 Model Switching

| Requirement | Status | Notes |
|-------------|--------|-------|
| POST /model/switch | PASS | |
| Look up model config | PASS | |
| Update registry | PASS | |
| Push registry | PASS | |
| Return new ModelConfig | PASS | |
| 400 for unknown model | PASS | |

**DEVIATION - Request format:**

Spec shows:
```json
{ "worker_name": "river", "model": "large" }
```

Implementation uses:
```json
{ "dyad": "river", "side": "left", "model": "large" }
```

The implementation is actually **better** - `worker_name` is ambiguous (dyad or something else?), while `dyad` + `side` uniquely identifies a worker. This is a beneficial deviation.

### 1.12 Role Switching (switch_roles)

| Requirement | Status | Notes |
|-------------|--------|-------|
| Acquire dyad lock | PASS | Using HashMap<String, bool> |
| GET endpoints from registry | PASS | |
| POST /prepare_switch to both | PASS | |
| POST /commit_switch to both | PASS | |
| Update registry batons | **PARTIAL** | See below |
| Push registry | PASS | |
| Release lock | PASS | |
| 409 on concurrent attempts | PASS | |
| 503 on unreachable partner | PASS | |

**BUG - Baton swap assumes initiator is actor:**

```rust
// http.rs lines 606-611
reg.update_worker_baton(&req.dyad, &req.side, Baton::Spectator);
reg.update_worker_baton(&req.dyad, &partner_side, Baton::Actor);

// The initiator becomes spectator, partner becomes actor
// (Assuming initiator was actor requesting the switch)
(Baton::Spectator, Baton::Actor)
```

This hardcodes the assumption that the initiator is always the actor becoming spectator. The spec says "Either worker can call switch_roles" - what if spectator initiates?

The implementation should **read current batons and swap them**, not assume direction.

---

## 2. Missing Functionality

### 2.1 Critical Missing Features

1. **SIGTERM for graceful shutdown** - Workers cannot save state before death

2. **Respawn on crash detection** - main.rs handles dead process detection, but after respawn the supervisor doesn't set endpoint back (endpoint is None until re-registration). This works but the supervisor's ProcessHandle will have stale endpoint=None.

3. **Summaries HashMap** - Spec shows orchestrator should store summaries:
   ```rust
   struct OrchestratorState {
       summaries: HashMap<(String, Side), String>,
       wake_timers: HashMap<(String, Side), Instant>,
   }
   ```

   Implementation stores this in RespawnManager, which is equivalent but named differently.

### 2.2 Minor Missing Features

1. **Config validation for adapter binary existence** - No check that binary path is valid

2. **Duplicate registration handling** - Spec says "Duplicate registration -> update endpoint, push registry". Implementation does this but doesn't log it as duplicate.

---

## 3. Code Quality Issues

### 3.1 Critical Issues

**1. Blocking file I/O in async context (config.rs line 107):**
```rust
let content = std::fs::read_to_string(path)?;
```

Should use `tokio::fs::read_to_string`. This blocks the runtime during config loading.

**2. Unwraps that can panic (http.rs line 303):**
```rust
Ok(Json(serde_json::to_value(response).unwrap()))
```

This can panic if serialization fails. Should use proper error handling.

**3. Race condition in switch_roles (http.rs lines 562-576):**
```rust
let prepare_result = prepare_both(...).await;
if !prepare_result {
    release_lock(&state, &req.dyad).await;
    // No abort sent to workers who might have prepared!
```

If one worker prepares successfully but the other fails, the prepared worker is left in an inconsistent state. Need to send abort on partial failure.

### 3.2 Important Issues

**1. Dead code warnings (4 warnings):**
- `get_embed_endpoint` - unused
- `next_wake_time` - unused
- `kill` - unused
- `KillFailed` variant - unused

These should either be used or removed.

**2. Hardcoded timeouts scattered throughout:**
```rust
Duration::from_secs(2)   // main.rs:106 - wait for registration
Duration::from_secs(5)   // http.rs:638 - prepare timeout
Duration::from_secs(10)  // supervisor.rs:287 - health check timeout
Duration::from_secs(60)  // main.rs:123 - health interval
```

Consider consolidating into constants or config.

**3. Clone-heavy code (http.rs):**
```rust
dyad_config.ground.clone()  // line 261, 297
req.worker.dyad.clone()     // multiple places
```

Many clones could be avoided with better lifetime management.

**4. Inconsistent error types:**
- `ConfigError` - manual impl
- `SupervisorError` - manual impl
- `RegistrationError` - JSON response struct
- `ModelSwitchError` - JSON response struct
- `SwitchRolesError` - JSON response struct

Consider using thiserror consistently as spec requires.

### 3.3 Suggestions

**1. Consider using `tokio::select!` with `biased` for wake timer:**
```rust
// Current approach polls every 10s
let wake_interval = Duration::from_secs(10);
let mut wake_ticker = tokio::time::interval(wake_interval);
```

Could use `next_wake_time()` to sleep precisely until needed.

**2. Health checks should run in parallel:**
```rust
// Current: sequential
for (key, endpoint) in endpoints {
    let result = client.get(&url)...
```

Should use `futures::future::join_all` for concurrent health checks.

**3. Consider adding metrics:**
- Process spawn count
- Health check failure count
- Registry push latency
- Registration latency

---

## 4. Test Coverage

### 4.1 Existing Tests

| File | Tests | Coverage |
|------|-------|----------|
| config.rs | 2 tests | Env var substitution only |
| respawn.rs | 2 tests | Done{None} and ContextExhausted |

**Total: 4 unit tests**

### 4.2 Missing Tests

**Critical - No tests for:**
- HTTP endpoints (register, model switch, switch_roles, worker output, health)
- Registry push functionality
- Supervisor process spawning
- Health check failure tracking
- Graceful shutdown
- Configuration validation

**Integration tests needed:**
- Full startup sequence
- Worker registration flow
- Adapter registration flow
- Role switch protocol
- Respawn behavior

### 4.3 Test Recommendations

1. Add integration tests using `axum::test::TestClient`
2. Mock process spawning for supervisor tests
3. Test health check failure thresholds
4. Test concurrent switch_roles requests (409 response)
5. Test config validation error cases

---

## 5. Documentation

### 5.1 Present

- Module-level doc comments on all files
- Function doc comments on some public APIs
- Cargo.toml description

### 5.2 Missing

- No README.md for the crate
- No examples
- No doc comments on many public structs/functions
- No architecture documentation

---

## 6. Recommendations

### 6.1 Critical Fixes (Must Do)

1. **Fix graceful shutdown to use SIGTERM** - Workers need time to save state
2. **Fix switch_roles to read actual batons** - Don't assume initiator is actor
3. **Add abort phase to switch_roles** - Handle partial prepare failures
4. **Fix blocking file I/O** - Use tokio::fs in config loading
5. **Remove unwrap() calls** - Use proper error handling

### 6.2 Important Fixes (Should Do)

1. **Align config format with spec** - Change `initial_actor: Side` to `left_starts_as: Baton`
2. **Change ProcessEntry serde to untagged** - Or document the change as intentional
3. **Add integration tests** - At minimum for HTTP endpoints
4. **Address dead code warnings** - Use or remove
5. **Add thiserror** - Per spec requirement

### 6.3 Nice to Have

1. Consolidate timeout constants
2. Parallel health checks
3. Add metrics/observability
4. Add crate README
5. Optimize cloning

---

## 7. Summary of Spec Deviations

| Deviation | Severity | Recommendation |
|-----------|----------|----------------|
| `left_starts_as: Baton` -> `initial_actor: Side` | HIGH | Align with spec |
| ProcessEntry tagged vs untagged | HIGH | Use untagged or update spec |
| AdapterConfig extra `side` field | MEDIUM | Document or remove |
| thiserror not used | LOW | Add or update spec |
| model/switch uses dyad+side not worker_name | LOW | Keep - it's better |
| Graceful shutdown uses SIGKILL | CRITICAL | Fix to SIGTERM |
| switch_roles hardcodes baton direction | HIGH | Fix to read actual batons |

---

## 8. Files Reviewed

- `/home/cassie/river-engine/crates/river-orchestrator/Cargo.toml`
- `/home/cassie/river-engine/crates/river-orchestrator/src/main.rs`
- `/home/cassie/river-engine/crates/river-orchestrator/src/config.rs`
- `/home/cassie/river-engine/crates/river-orchestrator/src/registry.rs`
- `/home/cassie/river-engine/crates/river-orchestrator/src/supervisor.rs`
- `/home/cassie/river-engine/crates/river-orchestrator/src/respawn.rs`
- `/home/cassie/river-engine/crates/river-orchestrator/src/http.rs`
- `/home/cassie/river-engine/crates/river-orchestrator/src/model.rs`
- `/home/cassie/river-engine/docs/superpowers/specs/2026-04-01-orchestrator-design.md`
- `/home/cassie/river-engine/crates/river-protocol/src/registry.rs`
- `/home/cassie/river-engine/crates/river-protocol/src/registration.rs`
- `/home/cassie/river-engine/crates/river-adapter/src/feature.rs`

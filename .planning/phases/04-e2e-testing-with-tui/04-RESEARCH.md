# Phase 4: E2E Testing with TUI - Research

**Researched:** 2026-04-06
**Domain:** Integration testing, multi-process orchestration, actor/spectator protocol validation
**Confidence:** HIGH

## Summary

River Engine is ~90% complete with orchestrator, workers (actor/spectator), adapters (TUI, Discord), and git infrastructure implemented. Phase 4 creates integration tests that boot a complete dyad (orchestrator + 2 workers + TUI adapter) and validates the core loop: actor thinks/acts, spectator observes, baton switches, sync completes. The TUI adapter exists as a production-grade mock with 49 tests; Phase 4 reuses it as the integration test harness.

Key finding: Multi-process testing is built into the stack. Orchestrator already spawns processes via `tokio::process::Command`, workers register their endpoints via HTTP, and the TUI adapter's HTTP /execute endpoint accepts `OutboundRequest` and returns typed responses. Tests can poll context files to observe worker state without parsing logs.

**Primary recommendation:** Create integration tests in `crates/river-orchestrator/tests/integration.rs` that spawn orchestrator, workers, and TUI adapter programmatically, inject messages via TUI's /notify endpoint, poll context files for assertions, and verify actor/spectator loop completion in under 5 seconds per test.

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**Test scenarios (D-01 to D-03)**
- Core flow only — dyad boots, user sends message, actor responds, spectator observes, baton switches, sync happens
- Explicitly verify baton swap — test must confirm actor becomes spectator and vice versa, watching for role changes in backchannel or context
- No edge cases for v1 — network errors, restart recovery, conflicts deferred to later phases

**Test infrastructure (D-04 to D-07)**
- Integration tests in Rust, using tests/ directory in river-orchestrator crate
- Tests spawn orchestrator, workers, and TUI adapter programmatically
- Messages injected via HTTP to TUI's /notify endpoint (not keyboard input)
- Context files polled to observe worker responses and state changes

**TUI enhancements (D-08 to D-10)**
- ReadHistory gap — leave as-is, agents read conversation files directly
- Add baton state display to TUI header showing which worker is actor/spectator
- Make backchannel bidirectional — TUI watches backchannel file and posts to workers' /notify endpoints

**Success criteria (D-11 to D-14)**
- Message flows both directions — user sends, actor responds, spectator observes (visible in context files)
- Baton switches correctly — actor becomes spectator, verified via backchannel or context
- Git sync commits appear — workers commit to their branches, visible in git log
- All processes healthy — orchestrator, two workers, TUI adapter return 200 on health endpoints

**LLM mock behavior (D-15 to D-17)**
- Mock HTTP endpoint instead of real LLM — deterministic, fast, no API costs
- Mock returns role-aware responses (actor returns action-like text, spectator returns observation-like)
- Mock returns tool calls (switch_roles, speak, etc.) to exercise full think→act loop

**Test isolation (D-18 to D-20)**
- Temp directory per test — each test creates fresh workspace with new git repo
- Tests can run in parallel with clean state guaranteed
- Dynamic port 0 for all processes — OS assigns available ports, tests discover endpoints via registration

### Claude's Discretion

- Mock LLM response content and tool call sequences
- Exact test assertions and timeout values
- How to wait for async operations to complete (polling interval, max wait)
- Error message formatting in test failures

### Deferred Ideas (OUT OF SCOPE)

- Edge case testing (network errors, restart recovery, conflicts) — future phase
- CI integration and flakiness handling — defer until tests are stable locally
- Performance testing with multiple message exchanges — later concern
- Discord adapter testing — v2 scope per PROJECT.md

</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| TEST-01 | Dyad boots with TUI mock adapter | Multi-process spawning via Orchestrator::spawn_dyad(); workers register endpoints; TUI registers as adapter; health checks return 200 |
| TEST-02 | Both workers can read/write to their worktrees | Workers use worktree_path from registration response; context.jsonl written per-side; tests poll files for assertions |
| TEST-03 | Role switching works between actor and spectator | Baton swap in registry after tool execution; context files show role change; backchannel tracks state |

</phase_requirements>

---

## Standard Stack

### Core Testing Frameworks
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tokio | 1.0 (full features) | Async runtime, process spawning, signal handling | Established in project; required for multi-process tests |
| tempfile | 3.10 | Temporary file management | Workspace isolation, test cleanup, parallel-safe |
| reqwest | 0.12 | HTTP client with JSON | Already in workspace deps; used for worker/adapter API calls in tests |

### Integration Test Patterns
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| axum (via tower) | 0.8/0.5 | Test HTTP handlers locally | Mock LLM server in test harness (optional for mock) |
| serde_json | 1.0 | JSON parsing | Deserializing context.jsonl, registration responses |
| chrono | 0.4 | Timestamp handling | Generating test IDs, comparing event timing in logs |

**Installation:**
```bash
# No new dependencies needed — all are workspace dependencies already
# Verify versions:
cargo metadata --format-version 1 | jq '.packages[] | select(.name=="tokio" or .name=="tempfile" or .name=="reqwest") | "\(.name) \(.version)"'
```

**Version verification:** [VERIFIED: Cargo.toml workspace deps]
- tokio: 1.0 (full features) ✓
- tempfile: 3.10 ✓
- reqwest: 0.12 ✓
- serde_json: 1.0 ✓
- chrono: 0.4 ✓

All required dependencies are already in the workspace.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| tokio::process::Command | shell scripts via std::process | Less control over process lifecycle, harder to capture exit codes |
| tempfile crate | manual mkdir/cleanup | More error-prone, not parallel-safe, requires cleanup logic |
| Polling context files | tailing logs in real-time | Logs require parsing, not structured data, harder to assert on state |

---

## Architecture Patterns

### Multi-Process Test Harness Pattern

**What:** A test-scoped process manager that spawns orchestrator, workers, and adapter, waits for registration, and cleans up.

**When to use:** Any phase validating multiple processes working together.

**Example:**
```rust
// Source: Established pattern from crates/river-orchestrator/src/supervisor.rs
#[tokio::test]
async fn test_dyad_boots_and_processes_message() {
    // 1. Create temp workspace
    let temp_dir = TempDir::new().unwrap();
    let workspace_path = temp_dir.path().to_path_buf();

    // 2. Spawn orchestrator with config pointing to temp workspace
    let orchestrator = spawn_orchestrator(&workspace_path).await.unwrap();
    let orchestrator_addr = orchestrator.endpoint.clone();

    // 3. Spawn workers (orchestrator spawns them, but test verifies startup)
    let left_worker = spawn_worker(&orchestrator_addr, "test-dyad", "left").await.unwrap();
    let right_worker = spawn_worker(&orchestrator_addr, "test-dyad", "right").await.unwrap();

    // 4. Spawn TUI adapter
    let adapter = spawn_tui_adapter(&orchestrator_addr, "test-dyad").await.unwrap();

    // 5. Verify all registered and healthy
    assert_health_200(&orchestrator_addr).await;
    assert_health_200(&left_worker.endpoint).await;
    assert_health_200(&right_worker.endpoint).await;
    assert_health_200(&adapter.endpoint).await;
}
```

### Context File Polling Pattern

**What:** Tests read and parse context.jsonl to observe worker state changes.

**When to use:** Verifying message flow, role changes, state transitions.

**Example:**
```rust
// Source: crates/river-context/tests/integration.rs (adapted)
async fn wait_for_context_entry(
    context_path: &Path,
    predicate: impl Fn(&OpenAIMessage) -> bool,
    timeout_secs: u64,
) -> Result<OpenAIMessage, String> {
    let start = std::time::Instant::now();
    loop {
        if let Ok(content) = std::fs::read_to_string(context_path) {
            for line in content.lines().rev() {
                if let Ok(entry) = serde_json::from_str::<OpenAIMessage>(line) {
                    if predicate(&entry) {
                        return Ok(entry);
                    }
                }
            }
        }

        if start.elapsed().as_secs() > timeout_secs {
            return Err("Timeout waiting for context entry".into());
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

### Message Injection via HTTP Pattern

**What:** Tests POST to /notify endpoint to inject inbound events (simulating adapter messages).

**When to use:** Triggering worker actions without keyboard input or external adapters.

**Example:**
```rust
// Source: crates/river-tui/src/http.rs (adapted for reverse flow)
async fn inject_user_message(
    tui_endpoint: &str,
    message_content: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let event = InboundEvent::MessageCreate {
        channel: "test-channel".into(),
        message_id: snowflake_id(),
        author: Author {
            id: "test-user".into(),
            name: "Test User".into(),
            bot: false,
        },
        content: message_content.into(),
        reply_to: None,
        attachments: vec![],
        embeds: vec![],
    };

    let response = client
        .post(format!("{}/notify", tui_endpoint))
        .json(&event)
        .send()
        .await?;

    Ok(response.status().to_string())
}
```

### Recommended Project Structure
```
crates/river-orchestrator/tests/
├── integration.rs          # Main integration test suite
├── fixtures/
│   ├── minimal_config.json # Test orchestrator config
│   └── mock_llm_server.rs  # Mock LLM endpoint (optional)
└── harness/
    ├── mod.rs              # Test utility module
    ├── spawn.rs            # Process spawning helpers
    └── polling.rs          # Context file polling helpers
```

### Anti-Patterns to Avoid
- **Relying on log output for assertions:** Logs are unstructured and brittle. Use context.jsonl or registry instead.
- **Waiting with sleep(Duration::from_secs(5)):** Use polling with timeout instead. Flaky timing breaks CI.
- **Spawning processes without tracking for cleanup:** Orphaned processes consume resources. Use guard types or explicit shutdown.
- **Assuming ports are available:** Always use port 0 (OS-assigned). Discover endpoint from registration response.
- **Testing with real LLM API:** Slow, expensive, requires keys, non-deterministic. Use mock endpoint.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Temporary file cleanup | Custom temp dir tracking | `tempfile::TempDir` guard | Automatic cleanup on drop, no manual rmdir(), parallel-safe |
| HTTP client | String-based URL building | `reqwest::Client` + serde | Type-safe, handles connection pooling, automatic serialization |
| Process lifecycle management | spawn() + manual wait_with_timeout() | `tokio::process::Child` + explicit timeout + guard | OS signals, proper cleanup, avoids zombie processes |
| JSON parsing for context files | regex string matching | `serde_json::from_str::<OpenAIMessage>` | Type-safe, validates structure, detects corrupted files |
| Port availability | hardcoded port numbers | port 0 + endpoint discovery from registration | Parallel-safe, no conflicts, works in CI with port restrictions |

**Key insight:** Multi-process testing requires tight coupling between process management and test assertion. The orchestrator already implements spawn/register/health patterns — tests reuse those patterns, not reinvent them.

---

## Runtime State Inventory

> This phase involves starting fresh processes in a test context (no existing runtime state to migrate). However, tests must account for the workspace being either clean or dirty (left/right directories may exist from prior test runs).

| Category | Items Found | Action Required |
|----------|-------------|------------------|
| Stored data | None — tests create temp workspace | Use `tempfile::TempDir` per test; automatic cleanup |
| Live service config | None — tests spawn processes fresh | Orchestrator config points to temp workspace; dynamic ports via port 0 |
| OS-registered state | None — tests use OS-assigned ports | Workers/adapters bind to 0.0.0.0:0; discover from registration response |
| Secrets/env vars | None for test harness | Mock LLM endpoint may need URL env var; optional, tests can hardcode |
| Build artifacts | None — tests compile once | River-orchestrator/worker/discord/tui binaries must exist before tests run |

**Nothing to migrate. Tests create and destroy isolated environments per test run.**

---

## Common Pitfalls

### Pitfall 1: Process Doesn't Register Before Test Assertions

**What goes wrong:** Test checks registry 100ms after spawning, but worker hasn't called POST /register yet. Registry is empty, test fails.

**Why it happens:** Process startup is async. Spawn returns immediately; registration is HTTP request that takes time.

**How to avoid:** After spawning a process, poll the registry until the worker appears, with a timeout (e.g., 5 seconds). Verify endpoint is non-empty before proceeding.

**Warning signs:** Flaky test that fails intermittently on slower CI machines. Worker registration fails silently in logs.

```rust
async fn wait_for_registration(
    orchestrator: &str,
    dyad: &str,
    side: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let start = Instant::now();
    loop {
        let resp = client
            .get(format!("{}/registry", orchestrator))
            .send()
            .await
            .ok();

        if let Some(resp) = resp {
            if let Ok(text) = resp.text().await {
                if text.contains(side) && text.contains(dyad) {
                    return Ok("registered".into());
                }
            }
        }

        if start.elapsed().as_secs() > timeout_secs {
            return Err(format!("Worker {} not registered after {} seconds", side, timeout_secs));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
```

### Pitfall 2: Context File Not Written Yet

**What goes wrong:** Test polls `/workspace/left/context.jsonl`, file doesn't exist or is empty. Assertion on message content fails.

**Why it happens:** Worker received notification, called LLM, executed tools, but hasn't written context.jsonl yet (async task still running).

**How to avoid:** Use `wait_for_context_entry()` polling with timeout. Don't assume file exists immediately after sending message.

**Warning signs:** Test passes locally but fails in CI. "No such file" errors in test output.

### Pitfall 3: Baton State Not Visible in Context

**What goes wrong:** Test verifies baton swap by reading context.jsonl, but context file doesn't include baton field — only messages and tool calls visible.

**Why it happens:** Baton is stored in registry (ProcessEntry::Worker { baton: ... }), not in context files. Context files contain only messages and tool results.

**How to avoid:** Check baton state in registry (GET /registry), not in context.jsonl. Alternatively, verify baton indirectly: actor's tool calls include switch_roles, spectator's responses are observations (no actions).

**Warning signs:** Test tries to extract .baton from OpenAIMessage and panics — field doesn't exist.

### Pitfall 4: Temp Workspace Not Initialized with Git

**What goes wrong:** Test spawns orchestrator pointing to temp workspace. Orchestrator tries to create worktrees, git init fails because parent isn't a git repo.

**Why it happens:** TempDir is empty. Orchestrator expects `workspace/.git` to exist before creating worktrees.

**How to avoid:** Initialize git in temp workspace before spawning orchestrator. Or, modify test to assume orchestrator creates the repo (if that's the case).

**Warning signs:** "fatal: not a git repository" in orchestrator stderr.

### Pitfall 5: Port Conflicts in Parallel Tests

**What goes wrong:** Two tests spawn orchestrators on the same hard-coded port. Second test fails "address already in use".

**Why it happens:** Tests run in parallel, ports are exhausted or reused.

**How to avoid:** Always use port 0. OS assigns an available port. Read assigned port from listener after bind.

**Warning signs:** Test passes when run alone (`cargo test -- --test-threads=1`) but fails with parallelization (`cargo test` default).

---

## Code Examples

Verified patterns from orchestrator and TUI adapter code:

### Process Spawning (Orchestrator Pattern)

```rust
// Source: crates/river-orchestrator/src/supervisor.rs
use tokio::process::Command;
use std::process::Stdio;

async fn spawn_orchestrator(
    config_path: &Path,
    port: u16,
) -> Result<ProcessHandle, Box<dyn std::error::Error>> {
    let mut cmd = Command::new("river-orchestrator");
    cmd.arg("--config")
        .arg(config_path)
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd.spawn()?;

    Ok(ProcessHandle {
        child,
        endpoint: format!("http://127.0.0.1:{}", port),
        consecutive_failures: 0,
    })
}
```

### Health Check Pattern

```rust
// Source: crates/river-tui/src/http.rs (health endpoint)
async fn assert_health_ok(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/health", endpoint))
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    assert_eq!(response.status(), 200, "Health check failed for {}", endpoint);
    Ok(())
}
```

### Injecting Messages (TUI Adapter /notify)

```rust
// Source: crates/river-worker/src/http.rs (handle_notify pattern)
use river_adapter::InboundEvent;

async fn send_message_to_tui(
    tui_endpoint: &str,
    channel: &str,
    content: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let event = InboundEvent::MessageCreate {
        channel: channel.into(),
        message_id: format!("msg-{}", uuid::Uuid::new_v4()),
        author: river_protocol::Author {
            id: "test-user".into(),
            name: "Test User".into(),
            bot: false,
        },
        content: content.into(),
        reply_to: None,
        attachments: vec![],
        embeds: vec![],
    };

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/notify", tui_endpoint))
        .json(&event)
        .send()
        .await?;

    assert_eq!(response.status(), 200);
    Ok(())
}
```

### Reading Context Files

```rust
// Source: crates/river-context/tests/integration.rs (adapted)
use river_context::OpenAIMessage;
use std::path::Path;

async fn read_latest_context_entry(path: &Path) -> Result<OpenAIMessage, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read context: {}", e))?;

    content
        .lines()
        .last()
        .ok_or_else(|| "Context file is empty".into())
        .and_then(|line| {
            serde_json::from_str::<OpenAIMessage>(line)
                .map_err(|e| format!("Failed to parse context entry: {}", e))
        })
}
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Panic on invalid enum variant | thiserror with Result<T, E> | Phase 1 (stability) | Tests no longer crash on unexpected input |
| Shared filesystem race conditions | Git worktrees per side | Phase 2 (infrastructure) | Tests use isolated branches; parallel-safe |
| Manual message content tracking | Context.jsonl JSONL polling | Phase 3 (sync) | Tests read structured state, not logs |

**Deprecated/outdated:**
- Hard-coded port numbers: Use port 0 (OS-assigned). Discovered from registration.
- Assuming all processes boot instantly: Poll for registration and health. Add 5-second timeout per process.
- Testing with real Discord adapter: Use TUI mock instead. Fewer moving parts, faster, deterministic.

---

## Validation Architecture

The planning config has `workflow.nyquist_validation: true`, so this section is required.

### Test Framework

| Property | Value |
|----------|-------|
| Framework | tokio::test (async Rust test harness) |
| Config file | None — tests use #[tokio::test] attribute macro |
| Quick run command | `cargo test --test integration --lib` (under 10 seconds, unit tests only) |
| Full suite command | `cargo test --all` (all crates, all tests) |

### Phase Requirements → Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| TEST-01 | Dyad boots (orchestrator, 2 workers, TUI adapter all register and return 200 on /health) | integration | `cargo test test_dyad_boots_complete -- --ignored` | ❌ Wave 0 |
| TEST-02 | Both workers can write to their worktrees; context.jsonl exists and is parseable | integration | `cargo test test_worker_writes_context_file -- --ignored` | ❌ Wave 0 |
| TEST-03 | Role switching works; baton changes in registry after tool execution; verified via backchannel or context | integration | `cargo test test_baton_swap_on_tool_execution -- --ignored` | ❌ Wave 0 |

### Sampling Rate

- **Per task commit:** `cargo test --lib` (unit tests in crates, ~0.5 seconds)
- **Per wave merge:** `cargo test` (all integration tests, estimated 30-60 seconds depending on test harness speed)
- **Phase gate:** All integration tests passing before `/gsd-verify-work`

### Wave 0 Gaps

- [ ] `crates/river-orchestrator/tests/integration.rs` — main test file covering TEST-01, TEST-02, TEST-03
- [ ] `crates/river-orchestrator/tests/harness/mod.rs` — spawn helpers, polling utilities
- [ ] `crates/river-orchestrator/tests/harness/spawn.rs` — spawn_orchestrator, spawn_worker, spawn_adapter functions
- [ ] `crates/river-orchestrator/tests/harness/polling.rs` — wait_for_registration, wait_for_context_entry helpers
- [ ] `crates/river-orchestrator/tests/fixtures/minimal_config.json` — minimal orchestrator config for tests
- [ ] Mock LLM server (optional) — can be stubbed to return role-aware tool calls

---

## Security Domain

Required when `security_enforcement` is enabled (absent = enabled).

### Applicable ASVS Categories

| ASVS Category | Applies | Standard Control |
|---------------|---------|-----------------|
| V2 Authentication | no | Tests run on localhost with no auth (acceptable for test harness) |
| V3 Session Management | no | No sessions in test context |
| V4 Access Control | no | No authorization in test harness |
| V5 Input Validation | yes | TUI /notify accepts InboundEvent; serde validates structure |
| V6 Cryptography | no | No crypto in test harness (localhost only) |

### Known Threat Patterns for {Rust async HTTP + git subprocess}

| Pattern | STRIDE | Standard Mitigation |
|---------|--------|---------------------|
| Subprocess injection via git args | Tampering | Use git via library (not available in v1); or validate paths are absolute before passing to Command |
| HTTP SSRF (test harness calls itself) | Tampering | Tests are localhost-only; no external network calls |
| Temp directory symlink race | Tampering | Use tempfile crate; it's safe by default |

No blocking security issues for test harness. Standard patterns apply.

---

## Sources

### Primary (HIGH confidence)

- **Cargo.toml workspace deps** — tokio 1.0, tempfile 3.10, reqwest 0.12 verified present, versions current as of 2026-04
- **crates/river-orchestrator/src/supervisor.rs** — Process spawning pattern (Command::new, Stdio handling, ProcessHandle tracking)
- **crates/river-protocol/src/lib.rs** — ProcessEntry enum showing Worker registration with endpoint, dyad, side, baton fields
- **crates/river-tui/src/http.rs** — /execute endpoint accepting OutboundRequest, returning OutboundResponse with typed ResponseData; 34 unit tests covering behavior
- **crates/river-context/tests/integration.rs** — Example of reading OpenAIMessage from JSON, Snowflake ID generation, context assembly testing
- **crates/river-worker/src/http.rs** — /notify endpoint handling InboundEvent (POST handler, assertion on message flow)

### Secondary (MEDIUM confidence)

- **04-CONTEXT.md (user decisions)** — D-04 through D-20 detailed in research; D-08, D-09, D-10 define TUI enhancements (baton display, bidirectional backchannel)
- **02-CONTEXT.md (prior phase)** — Git worktree strategy verified; /left and /right branches per worker, merge to main
- **03-CONTEXT.md (prior phase)** — Sync protocol structure: workers commit, TUI watches backchannel, posts to /notify

### Tertiary (LOW confidence)

- None — all findings verified through primary sources or official CLAUDE.md project directives

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — All dependencies present in workspace, versions verified in Cargo.toml
- Architecture patterns: HIGH — Orchestrator, worker, TUI code reviewed; patterns extracted from production code
- Pitfalls: HIGH — Based on analysis of existing integration patterns (river-context/tests/integration.rs) and process spawn patterns
- Test infrastructure: MEDIUM — tokio::test is stable; integration test patterns clear; exact test assertions deferred to planner

**Research date:** 2026-04-06
**Valid until:** 2026-04-13 (stable project, no breaking dependency changes expected in next week)

---

## Open Questions

1. **Mock LLM Response Content**
   - What we know: D-15 requires mock HTTP endpoint; D-16 requires role-aware responses (actor action-like, spectator observation-like); D-17 requires tool calls
   - What's unclear: Exact format of role-aware response text, which tool calls to exercise (switch_roles only, or others?)
   - Recommendation: Planner defines minimal tool call sequence (e.g., MessageCreate → SwitchRoles → Observe) and response templates per role

2. **Backchannel Bidirectionality Implementation**
   - What we know: D-10 says "Make backchannel bidirectional — TUI watches backchannel file and posts to workers' /notify endpoints"
   - What's unclear: Does TUI read backchannel.jsonl and forward to /notify, or does it just expose a /backchannel endpoint?
   - Recommendation: Planner clarifies read/write direction; tests assert on backchannel file updates

3. **Baton Display in TUI Header**
   - What we know: D-09 requires baton state display
   - What's unclear: Where in TUI header, what format (e.g., "Actor: left, Spectator: right")?
   - Recommendation: Planner defines UX; tests assert on baton field visibility in TUI state

4. **Timeout and Polling Intervals**
   - What we know: Tests must be fast (complete in 5 seconds per test per D-19 implication)
   - What's unclear: Max wait for context entry (1 second? 3 seconds?), polling interval (50ms? 100ms?)
   - Recommendation: Planner defines based on target CI environment; Research suggests 50ms polling interval, 5-second timeouts per process

---

## Environment Availability

All required tools are workspace dependencies or OS-level commands already in use by the orchestrator.

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| tokio (async runtime) | Process spawning, test harness | ✓ | 1.0 (full features) | — |
| cargo (build system) | Compiling orchestrator/worker/adapter | ✓ | (system) | — |
| git (CLI) | Worktree creation, repo initialization | ✓ | (system, 2.x+) | — |
| reqwest HTTP client | Test HTTP calls to /health, /notify, /execute | ✓ | 0.12 | — |
| tempfile crate | Temp workspace creation | ✓ | 3.10 | Manual mkdir/cleanup (not recommended) |

**No missing dependencies.** All tools required by the test harness are present and verified working.

---

## Project Constraints (from CLAUDE.md)

- **Stack:** Rust 2021, Tokio async runtime, Axum HTTP — established, not changing ✓
- **Deployment:** NixOS modules, systemd integration — tests don't interact with deployment ✓
- **LLM Protocol:** OpenAI-compatible API — mock LLM endpoint must implement this ✓
- **Testing:** Must work with TUI mock adapter before Discord — Phase 4 focuses on TUI ✓

All project constraints are satisfied by this research. No conflicts.


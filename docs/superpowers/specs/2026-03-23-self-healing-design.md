# Phase 2: Self-Healing Agent Policy

**Status:** Draft
**Date:** 2026-03-23
**Author:** Cassie + Claude

## Problem

River agents can get stuck or fail without external intervention:
- Tool errors accumulate without adaptation
- Model failures require manual restart
- Agents loop on the same action without progress
- No visibility into degraded state
- Manual restarts required when issues occur

## Goals

1. **Self-healing behaviors** — Agents adapt to transient failures automatically
2. **Stuck detection** — Recognize when the loop isn't making progress
3. **Graceful degradation** — Continue operating in reduced capacity
4. **Escalation mechanism** — Signal when human attention needed
5. **Systemd watchdog** — Prove liveness to systemd for auto-restart

## Non-Goals (Future Phases)

- Cross-agent monitoring (Phase 3)
- External alerting (Discord webhooks)
- Prometheus metrics endpoint (Phase 4)

---

## Design

### 1. Health Status Model

Three-tier status reflecting agent capability:

```rust
// src/policy.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Operating normally
    Healthy,
    /// Experiencing issues but still functional
    Degraded,
    /// Requires human attention
    NeedsAttention,
}
```

Status transitions:
- `Healthy` → `Degraded`: On repeated errors, stuck detection, or failed self-healing
- `Degraded` → `NeedsAttention`: After escalation threshold (3+ failed recovery attempts)
- `Degraded` → `Healthy`: After successful recovery (clean turn with no errors)
- `NeedsAttention` → `Healthy`: After human intervention clears ATTENTION.md

### 2. Policy Module

Central decision-making for self-healing:

```rust
// src/policy.rs

pub struct HealthPolicy {
    // Identity & config
    agent_name: String,
    data_dir: PathBuf,

    // Current status
    status: HealthStatus,

    // Error tracking
    consecutive_errors: u32,
    last_error: Option<DateTime<Utc>>,
    tool_failures: HashMap<String, u32>,  // Per-tool failure counts

    // Stuck detection
    last_action_hash: Option<u64>,
    repeated_action_count: u32,
    context_tokens_at_turn_start: u64,
    context_tokens_at_turn_end: u64,
    low_progress_turns: u32,

    // Recovery tracking
    recovery_attempts: u32,
    last_recovery: Option<DateTime<Utc>>,

    // Escalation tracking
    attention_created_at: Option<DateTime<Utc>>,
}

impl HealthPolicy {
    pub fn new(agent_name: String, data_dir: PathBuf) -> Self {
        Self {
            agent_name,
            data_dir,
            status: HealthStatus::Healthy,
            consecutive_errors: 0,
            last_error: None,
            tool_failures: HashMap::new(),
            last_action_hash: None,
            repeated_action_count: 0,
            context_tokens_at_turn_start: 0,
            context_tokens_at_turn_end: 0,
            low_progress_turns: 0,
            recovery_attempts: 0,
            last_recovery: None,
            attention_created_at: None,
        }
    }

    /// Called after each tool execution
    pub fn on_tool_result(&mut self, tool: &str, success: bool, _duration: Duration) {
        if success {
            // Clear failure count on success
            self.tool_failures.remove(tool);
        } else {
            let count = self.tool_failures.entry(tool.to_string()).or_insert(0);
            *count += 1;
        }
    }

    /// Get current health status for /health endpoint
    pub fn status(&self) -> HealthStatus { self.status }

    /// Get backoff for specific tool based on its failure count
    pub fn tool_backoff(&self, tool: &str) -> Duration { /* see Section 3 */ }

    /// Get global error backoff based on consecutive errors
    pub fn error_backoff(&self) -> Duration { /* see Section 3 */ }
}
```

### 3. Self-Healing Behaviors

Two levels of backoff operate independently:

1. **Tool Backoff** — Per-tool delays (caps at 8 min) for repeated failures of the same tool
2. **Error Backoff** — Global delays (caps at 15 min) for accumulated errors across any source

**Tool Backoff:**
When a specific tool fails repeatedly, delay before retrying that tool:
- 1st failure: immediate retry
- 2nd failure: 1 minute wait
- 3rd failure: 2 minutes
- 4th failure: 4 minutes
- 5th+ failure: 8 minutes (cap)

Tool backoff caps lower (8 min) because individual tool issues shouldn't block the entire agent — it can try other actions.

```rust
impl HealthPolicy {
    pub fn tool_backoff(&self, tool: &str) -> Duration {
        let failures = self.tool_failures.get(tool).unwrap_or(&0);
        match failures {
            0 | 1 => Duration::ZERO,
            2 => Duration::from_secs(60),
            3 => Duration::from_secs(120),
            4 => Duration::from_secs(240),
            _ => Duration::from_secs(480),
        }
    }
}
```

**Model Retry:**
On model API errors, retry with backoff:
- Rate limit (429): respect Retry-After header, or 60s default
- Server error (5xx): exponential backoff up to 15 minutes
- Client error (4xx except 429): no retry, escalate

**Error Backoff (General):**
Global backoff between turns when errors accumulate:
- 1 error: no delay
- 2 errors: 1 minute
- 3 errors: 2 minutes
- 4 errors: 4 minutes
- 5 errors: 8 minutes
- 6+ errors: 15 minutes (cap), mark NeedsAttention

```rust
const BACKOFF_CAP: Duration = Duration::from_secs(900); // 15 minutes

impl HealthPolicy {
    pub fn error_backoff(&self) -> Duration {
        if self.consecutive_errors == 0 {
            return Duration::ZERO;
        }
        let base = Duration::from_secs(60);
        let multiplier = 2u32.saturating_pow(self.consecutive_errors.saturating_sub(1));
        std::cmp::min(base * multiplier, BACKOFF_CAP)
    }
}
```

**Recovery Logic:**
State resets when the agent completes a "clean turn" — a turn with no errors:

```rust
impl HealthPolicy {
    pub fn on_turn_complete(&mut self, had_errors: bool) {
        if had_errors {
            self.consecutive_errors += 1;
            self.last_error = Some(Utc::now());
            if self.consecutive_errors >= 6 {
                self.status = HealthStatus::NeedsAttention;
            } else if self.consecutive_errors >= 2 {
                self.status = HealthStatus::Degraded;
            }
        } else {
            // Clean turn — reset error tracking
            self.consecutive_errors = 0;
            self.tool_failures.clear();
            self.low_progress_turns = 0;
            self.repeated_action_count = 0;

            // Recover from Degraded (but not NeedsAttention)
            if self.status == HealthStatus::Degraded {
                self.status = HealthStatus::Healthy;
                self.recovery_attempts = 0;
                tracing::info!(event = "recovery", "Agent recovered to healthy");
            }
        }
    }

    /// Called at start of each turn to check ATTENTION.md clearance
    pub fn check_attention_cleared(&mut self) -> bool {
        if self.status != HealthStatus::NeedsAttention {
            return false;
        }
        let attention_path = self.data_dir.join("ATTENTION.md");
        if !attention_path.exists() {
            // Human cleared the file — allow recovery on next clean turn
            self.status = HealthStatus::Degraded;
            self.attention_created_at = None;
            tracing::info!(event = "attention.cleared", "ATTENTION.md removed, allowing recovery");
            true
        } else {
            false
        }
    }
}
```

A "clean turn" means:
- Model call succeeded
- All tool calls succeeded (or agent chose not to call tools)
- No stuck detection triggered

### 4. Stuck Detection

Three signals indicate the agent may be stuck:

**Same Action Repeated:**
Hash the tool call (name + key arguments). If the same hash appears 3+ times consecutively, likely stuck.

```rust
// src/policy.rs

/// Compute a hash of the action for stuck detection.
/// Called from AgentLoop after parsing tool calls from model response.
pub fn compute_action_hash(tool_calls: &[ToolCall]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    for call in tool_calls {
        call.name.hash(&mut hasher);
        // Hash key fields, not full args (timestamps change)
        if let Some(path) = call.arguments.get("path") {
            path.to_string().hash(&mut hasher);
        }
        if let Some(command) = call.arguments.get("command") {
            command.to_string().hash(&mut hasher);
        }
    }
    hasher.finish()
}

impl HealthPolicy {
    pub fn check_repeated_action(&mut self, action_hash: u64) {
        if Some(action_hash) == self.last_action_hash {
            self.repeated_action_count += 1;
            if self.repeated_action_count >= 3 {
                self.mark_stuck("Same action repeated 3+ times");
            }
        } else {
            self.last_action_hash = Some(action_hash);
            self.repeated_action_count = 1;
        }
    }

    fn mark_stuck(&mut self, reason: &str) {
        self.status = HealthStatus::Degraded;
        self.recovery_attempts += 1;
        tracing::warn!(event = "stuck.detected", reason = reason, attempts = self.recovery_attempts);

        if self.recovery_attempts > 3 {
            let _ = self.escalate(reason, "Agent stuck, human intervention needed");
        }
    }
}
```

**No Progress Heuristic:**
Track net context growth per turn. At turn start, record `context_tokens_at_turn_start`. At turn end, record `context_tokens_at_turn_end`. Progress = end - start.

"Low progress" means <100 tokens of net growth. This catches:
- Agent producing output that gets discarded
- Agent in a loop that doesn't accumulate useful context

If 3+ consecutive turns show low progress, mark as stuck.

```rust
impl HealthPolicy {
    pub fn record_turn_tokens(&mut self, start_tokens: u64, end_tokens: u64) {
        self.context_tokens_at_turn_start = start_tokens;
        self.context_tokens_at_turn_end = end_tokens;

        let progress = end_tokens.saturating_sub(start_tokens);
        if progress < 100 {
            self.low_progress_turns += 1;
            if self.low_progress_turns >= 3 {
                self.mark_stuck("No progress: low token growth for 3+ turns");
            }
        } else {
            self.low_progress_turns = 0;
        }
    }
}
```

**Token Churn:**
If context grows >10k tokens in a single turn without settling, may be in a loop generating content that will be discarded. This is detected via the same token tracking.

When stuck is detected:
1. Set status to `Degraded`
2. Increment `recovery_attempts`
3. If recovery_attempts > 3, escalate to `NeedsAttention`

### 5. Escalation & ATTENTION.md

When `NeedsAttention` status is reached, write to `ATTENTION.md` in data directory:

```rust
// src/policy.rs

pub fn escalate(&mut self, reason: &str, context: &str) -> std::io::Result<()> {
    self.status = HealthStatus::NeedsAttention;

    let attention_path = self.data_dir.join("ATTENTION.md");
    let content = format!(
        "# Attention Required\n\n\
         **Agent:** {}\n\
         **Time:** {}\n\
         **Reason:** {}\n\n\
         ## Context\n\n\
         {}\n\n\
         ## To Clear\n\n\
         Delete this file after addressing the issue.\n",
        self.agent_name,
        Utc::now().to_rfc3339(),
        reason,
        context
    );

    std::fs::write(&attention_path, content)?;
    tracing::error!(
        event = "escalation",
        reason = reason,
        file = %attention_path.display(),
        "Agent requires attention"
    );

    Ok(())
}
```

The `/health` endpoint includes ATTENTION.md presence:

```json
{
  "status": "needs_attention",
  "attention": {
    "file": "/data/thomas/ATTENTION.md",
    "reason": "Stuck: same action repeated 5 times",
    "since": "2026-03-23T14:30:00Z"
  }
}
```

Recovery: when ATTENTION.md is deleted (human acknowledged), status can return to `Healthy` on next clean turn.

### 6. Systemd Watchdog

Separate background task pings systemd watchdog every 30s (watchdog timeout: 60s):

```rust
// src/watchdog.rs

use sd_notify::NotifyState;

pub fn spawn_watchdog_task() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(e) = sd_notify::notify(false, &[NotifyState::Watchdog]) {
                tracing::warn!(error = %e, "Failed to ping systemd watchdog");
            }
        }
    })
}
```

Systemd service configuration:

```ini
# river-thomas-gateway.service
[Service]
Type=notify
WatchdogSec=60
Restart=on-failure
RestartSec=5
```

The watchdog task runs independently of the agent loop. If the process hangs completely (e.g., deadlock), it stops pinging and systemd restarts.

**Important:** The agent keeps running when degraded. The watchdog proves the process is alive, not that it's making progress. Stuck detection handles progress monitoring.

### 7. Loop Integration

The policy module integrates at key points in `AgentLoop`:

```rust
// src/loop/mod.rs

pub struct AgentLoop {
    // ... existing fields
    policy: Arc<RwLock<HealthPolicy>>,
}

impl AgentLoop {
    async fn wake_phase(&mut self) {
        // Check if ATTENTION.md was cleared by human
        self.policy.write().await.check_attention_cleared();

        // Record context tokens at turn start
        let start_tokens = self.context_tokens();
        self.policy.write().await.context_tokens_at_turn_start = start_tokens;

        // ... existing wake logic ...
    }

    async fn think_phase(&mut self) -> Result<ModelResponse, LoopError> {
        // Check backoff before calling model
        let delay = self.policy.read().await.error_backoff();
        if !delay.is_zero() {
            tracing::info!(
                event = "backoff.wait",
                delay_secs = delay.as_secs(),
                "Waiting before next model call"
            );
            tokio::time::sleep(delay).await;
        }

        // ... existing model call logic ...

        // On success, check for repeated actions
        let action_hash = compute_action_hash(&response.tool_calls);
        self.policy.write().await.check_repeated_action(action_hash);

        Ok(response)
    }

    async fn act_phase(&mut self, tool_call: ToolCall) -> ToolResult {
        // Check tool-specific backoff
        let delay = self.policy.read().await.tool_backoff(&tool_call.name);
        if !delay.is_zero() {
            tracing::info!(
                event = "tool.backoff",
                tool = %tool_call.name,
                delay_secs = delay.as_secs(),
                "Waiting before retrying tool"
            );
            tokio::time::sleep(delay).await;
        }

        let result = self.executor.execute(&tool_call).await;

        // Notify policy of result
        self.policy.write().await.on_tool_result(
            &tool_call.name,
            result.is_ok(),
            result.duration,
        );

        result
    }

    async fn settle_phase(&mut self) {
        // Record context tokens at turn end for progress tracking
        let start = self.policy.read().await.context_tokens_at_turn_start;
        let end = self.context_tokens();
        self.policy.write().await.record_turn_tokens(start, end);

        // Mark turn complete
        let had_errors = self.turn_had_errors;
        self.policy.write().await.on_turn_complete(had_errors);

        // ... existing settle logic ...
    }
}
```

**Behavior at NeedsAttention:**
The agent loop **continues running** even at `NeedsAttention` status. The watchdog keeps pinging systemd, and the agent will:
- Continue processing incoming messages
- Apply maximum backoff (15 min) between turns
- Check for ATTENTION.md deletion at each wake

This allows the agent to remain responsive if a human clears the issue, rather than requiring a restart.

### 8. Health Endpoint Updates

Extend `/health` response with policy status:

```rust
// src/api/routes.rs

#[derive(Serialize)]
struct HealthResponse {
    status: HealthStatus,  // Changed from &'static str
    // ... existing fields ...

    policy: PolicyInfo,
}

#[derive(Serialize)]
struct PolicyInfo {
    consecutive_errors: u32,
    current_backoff_secs: u64,
    recovery_attempts: u32,
    attention_file: Option<String>,
}
```

---

## Files Changed

**New files:**
- `crates/river-gateway/src/policy.rs` — `HealthPolicy`, `HealthStatus`, stuck detection, escalation
- `crates/river-gateway/src/watchdog.rs` — systemd watchdog ping task

**Modified files:**
- `crates/river-gateway/src/lib.rs` — export new modules
- `crates/river-gateway/src/state.rs` — add `policy: Arc<RwLock<HealthPolicy>>` to `AppState`
- `crates/river-gateway/src/server.rs` — spawn watchdog task, create policy
- `crates/river-gateway/src/api/routes.rs` — include policy status in health response
- `crates/river-gateway/src/loop/mod.rs` — integrate policy at phase transitions
- `crates/river-gateway/src/metrics.rs` — update `HealthStatus` to use policy's status
- `crates/river-gateway/Cargo.toml` — add `sd-notify` dependency

---

## Testing

1. **Unit tests** — `HealthPolicy` state transitions, backoff calculations, stuck detection thresholds
2. **Integration test** — simulate tool failures, verify backoff delays applied
3. **Integration test** — simulate stuck pattern, verify ATTENTION.md created
4. **Manual test** — run with systemd, verify watchdog restart on process kill -STOP

```rust
#[test]
fn test_error_backoff_exponential() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::new());

    assert_eq!(policy.error_backoff(), Duration::ZERO);

    policy.on_turn_complete(true); // 1 error
    assert_eq!(policy.error_backoff(), Duration::from_secs(60));

    policy.on_turn_complete(true); // 2 errors
    assert_eq!(policy.error_backoff(), Duration::from_secs(120));

    policy.on_turn_complete(true); // 3 errors
    assert_eq!(policy.error_backoff(), Duration::from_secs(240));
}

#[test]
fn test_stuck_detection_repeated_action() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::new());
    let hash = 12345u64;

    policy.check_repeated_action(hash);
    assert_eq!(policy.status(), HealthStatus::Healthy);

    policy.check_repeated_action(hash);
    assert_eq!(policy.status(), HealthStatus::Healthy);

    policy.check_repeated_action(hash); // 3rd repeat
    assert_eq!(policy.status(), HealthStatus::Degraded);
}

#[test]
fn test_escalation_creates_attention_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = HealthPolicy::new("test", dir.path().to_path_buf());

    policy.escalate("Test reason", "Test context").unwrap();

    assert!(dir.path().join("ATTENTION.md").exists());
    assert_eq!(policy.status(), HealthStatus::NeedsAttention);
}
```

---

## What This Enables

- Agents recover from transient failures without intervention
- Stuck agents escalate rather than loop forever
- Health endpoint shows degraded state for monitoring
- Systemd restarts truly hung processes
- Humans notified via ATTENTION.md when intervention needed

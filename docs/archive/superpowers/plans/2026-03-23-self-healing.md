# Self-Healing Agent Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement self-healing behaviors so River agents recover from transient failures without human intervention.

**Architecture:** Policy module (`HealthPolicy`) tracks errors, detects stuck states, and manages recovery. Integrates with the existing `AgentLoop` at phase transitions. Watchdog task pings systemd independently. Escalation via `ATTENTION.md` file with bidirectional human communication.

**Tech Stack:** Rust, tokio, chrono, serde, sd-notify (new dependency)

**Spec:** `docs/superpowers/specs/2026-03-23-self-healing-design.md`

---

## File Structure

**New files:**
- `crates/river-gateway/src/policy.rs` — `HealthPolicy`, `HealthStatus`, stuck detection, escalation, recovery memory
- `crates/river-gateway/src/watchdog.rs` — systemd watchdog ping task

**Modified files:**
- `crates/river-gateway/src/lib.rs` — export `policy` and `watchdog` modules
- `crates/river-gateway/src/state.rs` — add `policy: Arc<RwLock<HealthPolicy>>` to `AppState`
- `crates/river-gateway/src/server.rs` — spawn watchdog task, create policy
- `crates/river-gateway/src/api/routes.rs` — include policy status in health response, return 503 for NeedsAttention
- `crates/river-gateway/src/loop/mod.rs` — integrate policy at phase transitions
- `crates/river-gateway/Cargo.toml` — add `sd-notify` dependency

---

### Task 1: HealthStatus Enum and HealthPolicy Struct

**Files:**
- Create: `crates/river-gateway/src/policy.rs`
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/river-gateway/src/policy.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_health_status_default() {
        let policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));
        assert_eq!(policy.status(), HealthStatus::Healthy);
    }

    #[test]
    fn test_health_status_serializes_snake_case() {
        let json = serde_json::to_string(&HealthStatus::NeedsAttention).unwrap();
        assert_eq!(json, "\"needs_attention\"");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-gateway policy::tests --no-run 2>&1 | head -20`
Expected: Compilation error - module not found

- [ ] **Step 3: Write minimal implementation**

```rust
// crates/river-gateway/src/policy.rs

//! Self-healing agent policy

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Health status reflecting agent capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    #[default]
    Healthy,
    Degraded,
    NeedsAttention,
}

/// Central decision-making for self-healing
pub struct HealthPolicy {
    // Identity & config
    agent_name: String,
    data_dir: PathBuf,

    // Current status
    status: HealthStatus,

    // Error tracking
    consecutive_errors: u32,
    last_error: Option<DateTime<Utc>>,
    // pub(crate) for test access to mock time decay
    pub(crate) tool_failures: HashMap<String, (u32, DateTime<Utc>)>,

    // Stuck detection
    last_action_hash: Option<u64>,
    repeated_action_count: u32,
    pub(crate) context_tokens_at_turn_start: u64,
    context_tokens_at_turn_end: u64,
    low_progress_turns: u32,
    pending_user_messages: u32,
    is_heartbeat_turn: bool,

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
            pending_user_messages: 0,
            is_heartbeat_turn: false,
            recovery_attempts: 0,
            last_recovery: None,
            attention_created_at: None,
        }
    }

    /// Get current health status
    pub fn status(&self) -> HealthStatus {
        self.status
    }

    /// Get agent name
    pub fn agent_name(&self) -> &str {
        &self.agent_name
    }

    /// Get data directory
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Get consecutive error count
    pub fn consecutive_errors(&self) -> u32 {
        self.consecutive_errors
    }

    /// Get recovery attempts count
    pub fn recovery_attempts(&self) -> u32 {
        self.recovery_attempts
    }

    /// Mark turn as heartbeat (excluded from progress tracking)
    pub fn set_heartbeat_turn(&mut self, is_heartbeat: bool) {
        self.is_heartbeat_turn = is_heartbeat;
    }

    /// Track pending user messages for stuck detection context
    pub fn set_pending_messages(&mut self, count: u32) {
        self.pending_user_messages = count;
    }

    /// Set context tokens at turn start for progress tracking
    pub fn set_turn_start_tokens(&mut self, tokens: u64) {
        self.context_tokens_at_turn_start = tokens;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_default() {
        let policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));
        assert_eq!(policy.status(), HealthStatus::Healthy);
    }

    #[test]
    fn test_health_status_serializes_snake_case() {
        let json = serde_json::to_string(&HealthStatus::NeedsAttention).unwrap();
        assert_eq!(json, "\"needs_attention\"");
    }
}
```

- [ ] **Step 4: Add module to lib.rs**

```rust
// Add to crates/river-gateway/src/lib.rs
pub mod policy;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests`
Expected: 2 tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/policy.rs crates/river-gateway/src/lib.rs
git commit -m "feat(policy): add HealthStatus enum and HealthPolicy struct"
```

---

### Task 2: Tool Backoff with Decay

**Files:**
- Modify: `crates/river-gateway/src/policy.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// Add to policy.rs tests module

#[test]
fn test_tool_backoff_escalates() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    // No failures = no backoff
    assert_eq!(policy.tool_backoff("bash"), Duration::ZERO);

    // 1st failure = no backoff
    policy.on_tool_result("bash", false, Duration::from_millis(100));
    assert_eq!(policy.tool_backoff("bash"), Duration::ZERO);

    // 2nd failure = 1 min
    policy.on_tool_result("bash", false, Duration::from_millis(100));
    assert_eq!(policy.tool_backoff("bash"), Duration::from_secs(60));

    // 3rd failure = 2 min
    policy.on_tool_result("bash", false, Duration::from_millis(100));
    assert_eq!(policy.tool_backoff("bash"), Duration::from_secs(120));
}

#[test]
fn test_tool_backoff_caps_at_8_minutes() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    for _ in 0..10 {
        policy.on_tool_result("bash", false, Duration::from_millis(100));
    }

    assert_eq!(policy.tool_backoff("bash"), Duration::from_secs(480));
}

#[test]
fn test_tool_success_clears_failures() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    policy.on_tool_result("bash", false, Duration::from_millis(100));
    policy.on_tool_result("bash", false, Duration::from_millis(100));
    assert_eq!(policy.tool_backoff("bash"), Duration::from_secs(60));

    policy.on_tool_result("bash", true, Duration::from_millis(100));
    assert_eq!(policy.tool_backoff("bash"), Duration::ZERO);
}

#[test]
fn test_tool_failure_decay_after_one_hour() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    // Simulate 5 failures
    for _ in 0..5 {
        policy.on_tool_result("bash", false, Duration::from_millis(100));
    }
    assert_eq!(policy.tool_backoff("bash"), Duration::from_secs(480));

    // Manually set last_fail to 2 hours ago
    if let Some(entry) = policy.tool_failures.get_mut("bash") {
        entry.1 = Utc::now() - chrono::Duration::hours(2);
    }

    // After decay, backoff should be zero
    assert_eq!(policy.tool_backoff("bash"), Duration::ZERO);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway policy::tests::test_tool_backoff`
Expected: Compilation error - methods not found

- [ ] **Step 3: Implement tool backoff methods**

```rust
// Add to HealthPolicy impl block in policy.rs

const TOOL_FAILURE_DECAY_SECS: i64 = 3600; // 1 hour

impl HealthPolicy {
    // ... existing methods ...

    /// Get backoff duration for a specific tool based on failure count with decay
    pub fn tool_backoff(&self, tool: &str) -> Duration {
        if let Some((count, last_fail)) = self.tool_failures.get(tool) {
            // Decay: if no failures for 1 hour, reset
            if Utc::now().signed_duration_since(*last_fail).num_seconds() > TOOL_FAILURE_DECAY_SECS {
                return Duration::ZERO;
            }
            match count {
                0 | 1 => Duration::ZERO,
                2 => Duration::from_secs(60),
                3 => Duration::from_secs(120),
                4 => Duration::from_secs(240),
                _ => Duration::from_secs(480), // 8 min cap
            }
        } else {
            Duration::ZERO
        }
    }

    /// Called after each tool execution
    pub fn on_tool_result(&mut self, tool: &str, success: bool, _duration: Duration) {
        if success {
            self.tool_failures.remove(tool);
        } else {
            let entry = self.tool_failures.entry(tool.to_string()).or_insert((0, Utc::now()));
            entry.0 += 1;
            entry.1 = Utc::now();
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests::test_tool`
Expected: 4 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/policy.rs
git commit -m "feat(policy): add tool backoff with 1-hour decay"
```

---

### Task 3: Error Backoff and Turn Completion with Success Ratio

**Files:**
- Modify: `crates/river-gateway/src/policy.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// Add to policy.rs tests module

#[test]
fn test_error_backoff_escalates() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    assert_eq!(policy.error_backoff(), Duration::ZERO);

    // 100% failure (1/1) counts as +2 errors
    policy.on_turn_complete(1, 1);
    assert_eq!(policy.consecutive_errors(), 2);
    assert_eq!(policy.error_backoff(), Duration::from_secs(120)); // 2^1 * 60

    // Another 100% failure
    policy.on_turn_complete(1, 1);
    assert_eq!(policy.consecutive_errors(), 4);
}

#[test]
fn test_error_backoff_caps_at_15_minutes() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    // Many failures
    for _ in 0..20 {
        policy.on_turn_complete(1, 1);
    }

    assert_eq!(policy.error_backoff(), Duration::from_secs(900)); // 15 min cap
}

#[test]
fn test_success_ratio_recovery() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    // Degrade the agent
    policy.on_turn_complete(1, 1);
    policy.on_turn_complete(1, 1);
    assert_eq!(policy.status(), HealthStatus::Degraded);

    // 80% success rate (4/5 succeed) should recover
    policy.on_turn_complete(5, 1);
    assert_eq!(policy.status(), HealthStatus::Healthy);
    assert_eq!(policy.consecutive_errors(), 0);
}

#[test]
fn test_needs_attention_after_six_errors() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    // 3 turns of 100% failure = 6 errors
    policy.on_turn_complete(1, 1);
    policy.on_turn_complete(1, 1);
    policy.on_turn_complete(1, 1);

    assert_eq!(policy.status(), HealthStatus::NeedsAttention);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway policy::tests::test_error`
Expected: Compilation error - method not found

- [ ] **Step 3: Implement error backoff and turn completion**

```rust
// Add to HealthPolicy impl block in policy.rs

const BACKOFF_CAP_SECS: u64 = 900; // 15 minutes

impl HealthPolicy {
    // ... existing methods ...

    /// Get global error backoff based on consecutive errors
    pub fn error_backoff(&self) -> Duration {
        if self.consecutive_errors == 0 {
            return Duration::ZERO;
        }
        let base = 60u64; // 1 minute
        let multiplier = 2u64.saturating_pow(self.consecutive_errors.saturating_sub(1));
        Duration::from_secs(std::cmp::min(base.saturating_mul(multiplier), BACKOFF_CAP_SECS))
    }

    /// Called at end of turn with call counts (not binary had_errors)
    pub fn on_turn_complete(&mut self, total_calls: u32, failed_calls: u32) {
        let success_ratio = if total_calls == 0 {
            1.0 // No calls = clean turn
        } else {
            (total_calls.saturating_sub(failed_calls)) as f64 / total_calls as f64
        };

        if success_ratio >= 0.8 {
            // 80%+ success = "clean enough" to recover
            self.consecutive_errors = 0;
            self.low_progress_turns = 0;
            self.repeated_action_count = 0;

            if self.status == HealthStatus::Degraded {
                self.status = HealthStatus::Healthy;
                self.recovery_attempts = 0;
                self.last_recovery = Some(Utc::now());
                tracing::info!(event = "recovery", "Agent recovered to healthy");
            }
        } else if success_ratio < 0.5 {
            // 50%+ failure = escalate faster (counts as 2 errors)
            self.consecutive_errors = self.consecutive_errors.saturating_add(2);
            self.last_error = Some(Utc::now());
            self.update_status_from_errors();
        } else {
            // Between 50-80% success = normal error accumulation
            self.consecutive_errors = self.consecutive_errors.saturating_add(1);
            self.last_error = Some(Utc::now());
            self.update_status_from_errors();
        }
    }

    fn update_status_from_errors(&mut self) {
        if self.consecutive_errors >= 6 {
            self.status = HealthStatus::NeedsAttention;
        } else if self.consecutive_errors >= 2 {
            self.status = HealthStatus::Degraded;
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests::test_error && cargo test -p river-gateway policy::tests::test_success && cargo test -p river-gateway policy::tests::test_needs`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/policy.rs
git commit -m "feat(policy): add error backoff with success ratio recovery"
```

---

### Task 4: Model Error Handling with 401 Immediate Escalation

**Files:**
- Modify: `crates/river-gateway/src/policy.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// Add to policy.rs tests module

#[test]
fn test_401_immediate_escalation() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = HealthPolicy::new("test".to_string(), dir.path().to_path_buf());

    let action = policy.on_model_error(401);
    assert!(matches!(action, ModelErrorAction::Escalated));
    assert_eq!(policy.status(), HealthStatus::NeedsAttention);
    assert!(dir.path().join("ATTENTION.md").exists());
}

#[test]
fn test_403_immediate_escalation() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = HealthPolicy::new("test".to_string(), dir.path().to_path_buf());

    let action = policy.on_model_error(403);
    assert!(matches!(action, ModelErrorAction::Escalated));
    assert_eq!(policy.status(), HealthStatus::NeedsAttention);
}

#[test]
fn test_429_returns_retry_after() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    let action = policy.on_model_error(429);
    assert!(matches!(action, ModelErrorAction::RetryAfter(_)));
}

#[test]
fn test_500_returns_backoff() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    let action = policy.on_model_error(500);
    assert!(matches!(action, ModelErrorAction::RetryWithBackoff(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway policy::tests::test_401`
Expected: Compilation error - ModelErrorAction not found

- [ ] **Step 3: Implement model error handling**

```rust
// Add to policy.rs

/// Action to take after model error
#[derive(Debug, Clone, PartialEq)]
pub enum ModelErrorAction {
    RetryAfter(Duration),
    RetryWithBackoff(Duration),
    NoRetry,
    Escalated,
}

impl HealthPolicy {
    // ... existing methods ...

    /// Handle model API error, returning appropriate action
    pub fn on_model_error(&mut self, status_code: u16) -> ModelErrorAction {
        match status_code {
            401 | 403 => {
                // Auth errors escalate immediately
                let _ = self.escalate(
                    &format!("Authentication error: HTTP {}", status_code),
                    "API key may be invalid or expired. Check credentials.",
                );
                ModelErrorAction::Escalated
            }
            429 => {
                // Rate limit - use default 60s retry
                ModelErrorAction::RetryAfter(Duration::from_secs(60))
            }
            500..=599 => {
                // Server error - use exponential backoff
                self.consecutive_errors = self.consecutive_errors.saturating_add(1);
                self.last_error = Some(Utc::now());
                ModelErrorAction::RetryWithBackoff(self.error_backoff())
            }
            _ => {
                // Other client errors - don't retry
                self.status = HealthStatus::Degraded;
                ModelErrorAction::NoRetry
            }
        }
    }

    /// Escalate to NeedsAttention and write ATTENTION.md
    pub fn escalate(&mut self, reason: &str, context: &str) -> std::io::Result<()> {
        self.status = HealthStatus::NeedsAttention;
        self.attention_created_at = Some(Utc::now());

        let attention_path = self.data_dir.join("ATTENTION.md");
        let content = format!(
            "# Attention Required\n\n\
             **Agent:** {}\n\
             **Time:** {}\n\
             **Reason:** {}\n\n\
             ## Context\n\n\
             {}\n\n\
             ## To Clear\n\n\
             Delete this file after addressing the issue.\n\n\
             ---\n\n\
             ## Response\n\n\
             (Add your response here)\n",
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
}
```

- [ ] **Step 4: Add tempfile to dev-dependencies**

```toml
# Ensure in Cargo.toml [dev-dependencies]
tempfile = "3.10"
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests::test_4`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/policy.rs
git commit -m "feat(policy): add model error handling with 401 immediate escalation"
```

---

### Task 5: Stuck Detection - Repeated Action

**Files:**
- Modify: `crates/river-gateway/src/policy.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// Add to policy.rs tests module

#[test]
fn test_repeated_action_detection() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));
    let hash = 12345u64;

    policy.check_repeated_action(hash);
    assert_eq!(policy.status(), HealthStatus::Healthy);

    policy.check_repeated_action(hash);
    assert_eq!(policy.status(), HealthStatus::Healthy);

    policy.check_repeated_action(hash); // 3rd repeat
    assert_eq!(policy.status(), HealthStatus::Degraded);
}

#[test]
fn test_different_actions_reset_count() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    policy.check_repeated_action(111);
    policy.check_repeated_action(111);
    policy.check_repeated_action(222); // Different action
    policy.check_repeated_action(222);
    policy.check_repeated_action(222); // 3rd of this action

    assert_eq!(policy.status(), HealthStatus::Degraded);
}

#[test]
fn test_compute_action_hash() {
    use crate::tools::ToolCall;

    let call1 = ToolCall {
        id: "1".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({"path": "/foo/bar.txt"}),
    };
    let call2 = ToolCall {
        id: "2".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({"path": "/foo/bar.txt"}),
    };
    let call3 = ToolCall {
        id: "3".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({"path": "/different.txt"}),
    };

    let hash1 = compute_action_hash(&[call1]);
    let hash2 = compute_action_hash(&[call2]);
    let hash3 = compute_action_hash(&[call3]);

    assert_eq!(hash1, hash2); // Same tool + path = same hash
    assert_ne!(hash1, hash3); // Different path = different hash
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway policy::tests::test_repeated`
Expected: Compilation error - method not found

- [ ] **Step 3: Implement stuck detection**

```rust
// Add to policy.rs

use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use crate::tools::ToolCall;

/// Compute a hash of the action for stuck detection
pub fn compute_action_hash(tool_calls: &[ToolCall]) -> u64 {
    let mut hasher = DefaultHasher::new();

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
    // ... existing methods ...

    /// Check if action is repeated, mark stuck if 3+ times
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
        tracing::warn!(
            event = "stuck.detected",
            reason = reason,
            attempts = self.recovery_attempts,
            "Agent may be stuck"
        );

        if self.recovery_attempts > 3 {
            let _ = self.escalate(reason, "Agent stuck, human intervention needed");
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests::test_repeated && cargo test -p river-gateway policy::tests::test_different && cargo test -p river-gateway policy::tests::test_compute`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/policy.rs
git commit -m "feat(policy): add stuck detection for repeated actions"
```

---

### Task 6: Stuck Detection - Low Progress with Heartbeat Exclusion

**Files:**
- Modify: `crates/river-gateway/src/policy.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// Add to policy.rs tests module

#[test]
fn test_low_progress_detection() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));
    policy.set_pending_messages(1); // User is waiting

    // 3 turns with <100 token progress
    policy.record_turn_tokens(1000, 1050);
    assert_eq!(policy.status(), HealthStatus::Healthy);

    policy.record_turn_tokens(1050, 1080);
    assert_eq!(policy.status(), HealthStatus::Healthy);

    policy.record_turn_tokens(1080, 1090);
    assert_eq!(policy.status(), HealthStatus::Degraded);
}

#[test]
fn test_heartbeat_excluded_from_progress() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));
    policy.set_pending_messages(1);
    policy.set_heartbeat_turn(true);

    // Heartbeat turns don't count toward stuck detection
    policy.record_turn_tokens(1000, 1010);
    policy.record_turn_tokens(1010, 1020);
    policy.record_turn_tokens(1020, 1030);

    assert_eq!(policy.status(), HealthStatus::Healthy);
}

#[test]
fn test_no_stuck_without_pending_messages() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));
    // No pending messages - user isn't waiting

    policy.record_turn_tokens(1000, 1010);
    policy.record_turn_tokens(1010, 1020);
    policy.record_turn_tokens(1020, 1030);

    assert_eq!(policy.status(), HealthStatus::Healthy);
}

#[test]
fn test_good_progress_resets_counter() {
    let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));
    policy.set_pending_messages(1);

    policy.record_turn_tokens(1000, 1050);
    policy.record_turn_tokens(1050, 1080);
    // Good progress resets
    policy.record_turn_tokens(1080, 1500);
    policy.record_turn_tokens(1500, 1520);
    policy.record_turn_tokens(1520, 1540);

    assert_eq!(policy.status(), HealthStatus::Healthy);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway policy::tests::test_low_progress`
Expected: Compilation error - method not found

- [ ] **Step 3: Implement low progress detection**

```rust
// Add to HealthPolicy impl block in policy.rs

impl HealthPolicy {
    // ... existing methods ...

    /// Record token counts for progress tracking
    pub fn record_turn_tokens(&mut self, start_tokens: u64, end_tokens: u64) {
        self.context_tokens_at_turn_start = start_tokens;
        self.context_tokens_at_turn_end = end_tokens;

        // Skip progress tracking for heartbeat turns
        if self.is_heartbeat_turn {
            self.is_heartbeat_turn = false; // Reset for next turn
            return;
        }

        let progress = end_tokens.saturating_sub(start_tokens);
        if progress < 100 {
            self.low_progress_turns += 1;
            // Only mark stuck if user is actually waiting
            if self.low_progress_turns >= 3 && self.pending_user_messages > 0 {
                self.mark_stuck("No progress: low token growth for 3+ turns while user waiting");
            }
        } else {
            self.low_progress_turns = 0;
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests::test_low && cargo test -p river-gateway policy::tests::test_heartbeat && cargo test -p river-gateway policy::tests::test_no_stuck && cargo test -p river-gateway policy::tests::test_good`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/policy.rs
git commit -m "feat(policy): add low progress stuck detection with heartbeat exclusion"
```

---

### Task 7: Bidirectional ATTENTION.md - Parse Human Response

**Files:**
- Modify: `crates/river-gateway/src/policy.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// Add to policy.rs tests module

#[test]
fn test_attention_cleared_on_file_delete() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = HealthPolicy::new("test".to_string(), dir.path().to_path_buf());

    // Escalate to create ATTENTION.md
    policy.escalate("Test", "Context").unwrap();
    assert_eq!(policy.status(), HealthStatus::NeedsAttention);

    // File exists, no response yet
    assert!(policy.check_attention_cleared().is_none());
    assert_eq!(policy.status(), HealthStatus::NeedsAttention);

    // Delete the file
    std::fs::remove_file(dir.path().join("ATTENTION.md")).unwrap();

    // Now it should clear
    assert!(policy.check_attention_cleared().is_none());
    assert_eq!(policy.status(), HealthStatus::Degraded);
}

#[test]
fn test_parse_human_response() {
    let content = r#"# Attention Required

**Agent:** test
**Time:** 2026-03-23T14:30:00Z
**Reason:** Test

## Context

Test context

## To Clear

Delete this file.

---

## Response

I rotated the API key. Try again.

— Cassie
"#;

    let response = HealthPolicy::parse_human_response(content);
    assert!(response.is_some());
    assert!(response.unwrap().contains("I rotated the API key"));
}

#[test]
fn test_check_attention_returns_response() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = HealthPolicy::new("test".to_string(), dir.path().to_path_buf());

    // First escalate to set status to NeedsAttention
    policy.escalate("Test", "Context").unwrap();

    // Overwrite ATTENTION.md with a response
    let content = "# Attention\n\n## Response\n\nFixed the issue.\n";
    std::fs::write(dir.path().join("ATTENTION.md"), content).unwrap();

    let response = policy.check_attention_cleared();
    assert!(response.is_some());
    assert!(response.unwrap().contains("Fixed the issue"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway policy::tests::test_attention`
Expected: Compilation error - method not found

- [ ] **Step 3: Implement attention checking with response parsing**

```rust
// Add to HealthPolicy impl block in policy.rs

impl HealthPolicy {
    // ... existing methods ...

    /// Check ATTENTION.md for clearance or human response
    pub fn check_attention_cleared(&mut self) -> Option<String> {
        if self.status != HealthStatus::NeedsAttention {
            return None;
        }

        let attention_path = self.data_dir.join("ATTENTION.md");
        if !attention_path.exists() {
            // Human deleted the file — allow recovery on next clean turn
            self.status = HealthStatus::Degraded;
            self.attention_created_at = None;
            tracing::info!(event = "attention.cleared", "ATTENTION.md removed, allowing recovery");
            return None;
        }

        // Check for human response in the file
        if let Ok(content) = std::fs::read_to_string(&attention_path) {
            if let Some(response) = Self::parse_human_response(&content) {
                tracing::info!(
                    event = "attention.response",
                    response = %response,
                    "Human responded to ATTENTION.md"
                );
                return Some(response);
            }
        }
        None
    }

    /// Parse human response from ATTENTION.md
    pub fn parse_human_response(content: &str) -> Option<String> {
        // Look for "## Response" section added by human
        if let Some(idx) = content.find("## Response") {
            let response_section = &content[idx..];
            let response: String = response_section
                .lines()
                .skip(1) // Skip "## Response" header
                .take_while(|line| !line.starts_with("##"))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();

            // Only return if there's actual content beyond the placeholder
            if !response.is_empty() && !response.contains("(Add your response here)") {
                return Some(response);
            }
        }
        None
    }

    /// Get ATTENTION.md path if it exists
    pub fn attention_file_path(&self) -> Option<String> {
        let path = self.data_dir.join("ATTENTION.md");
        if path.exists() {
            Some(path.to_string_lossy().to_string())
        } else {
            None
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests::test_attention && cargo test -p river-gateway policy::tests::test_parse && cargo test -p river-gateway policy::tests::test_check`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/policy.rs
git commit -m "feat(policy): add bidirectional ATTENTION.md with response parsing"
```

---

### Task 8: Recovery Memory (recovery.jsonl)

**Files:**
- Modify: `crates/river-gateway/src/policy.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// Add to policy.rs tests module

#[test]
fn test_recovery_logged_to_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = HealthPolicy::new("test".to_string(), dir.path().to_path_buf());

    // Degrade then recover
    policy.on_turn_complete(1, 1);
    policy.on_turn_complete(1, 1);
    assert_eq!(policy.status(), HealthStatus::Degraded);

    policy.on_turn_complete(5, 0); // Clean turn
    assert_eq!(policy.status(), HealthStatus::Healthy);

    // Check recovery.jsonl exists and has content
    let log_path = dir.path().join("recovery.jsonl");
    assert!(log_path.exists());

    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(content.contains("recovery"));
    assert!(content.contains("test")); // agent name
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway policy::tests::test_recovery_logged`
Expected: Test fails - no recovery.jsonl created

- [ ] **Step 3: Implement recovery logging**

```rust
// Add to HealthPolicy impl block in policy.rs

use std::io::Write;

impl HealthPolicy {
    // ... existing methods ...

    fn log_recovery(&self, reason: &str) {
        let recovery_log = self.data_dir.join("recovery.jsonl");

        let entry = serde_json::json!({
            "timestamp": Utc::now().to_rfc3339(),
            "agent": self.agent_name,
            "previous_status": format!("{:?}", self.status),
            "recovery_reason": reason,
            "context": {
                "consecutive_errors_before": self.consecutive_errors,
                "recovery_attempts": self.recovery_attempts,
                "last_error": self.last_error.map(|t| t.to_rfc3339()),
                "tool_failures": self.tool_failures.keys().collect::<Vec<_>>(),
            }
        });

        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&recovery_log)
        {
            let _ = writeln!(file, "{}", entry);
        }

        tracing::info!(
            event = "recovery.complete",
            reason = reason,
            attempts = self.recovery_attempts,
            "Agent recovered"
        );
    }
}
```

- [ ] **Step 4: Update on_turn_complete to call log_recovery**

```rust
// Modify the recovery branch in on_turn_complete:

if self.status == HealthStatus::Degraded {
    self.log_recovery("Turn completed with 80%+ success rate");
    self.status = HealthStatus::Healthy;
    self.recovery_attempts = 0;
    self.last_recovery = Some(Utc::now());
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests::test_recovery_logged`
Expected: Test passes

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/policy.rs
git commit -m "feat(policy): add recovery memory logging to recovery.jsonl"
```

---

### Task 9: Proactive Context Rotation

**Files:**
- Modify: `crates/river-gateway/src/policy.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// Add to policy.rs tests module

#[test]
fn test_proactive_rotation_at_80_percent() {
    let policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

    assert_eq!(policy.on_context_warning(70.0), ContextAction::Continue);
    assert_eq!(policy.on_context_warning(80.0), ContextAction::RotateNow);
    assert_eq!(policy.on_context_warning(85.0), ContextAction::RotateNow);
    assert_eq!(policy.on_context_warning(90.0), ContextAction::Continue); // 90%+ handled elsewhere
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway policy::tests::test_proactive`
Expected: Compilation error - ContextAction not found

- [ ] **Step 3: Implement proactive rotation**

```rust
// Add to policy.rs

/// Action for context management
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Continue,
    RotateNow,
}

impl HealthPolicy {
    // ... existing methods ...

    /// Called when context usage is checked, returns action to take
    pub fn on_context_warning(&self, usage_percent: f64) -> ContextAction {
        if usage_percent >= 80.0 && usage_percent < 90.0 {
            tracing::info!(
                event = "context.proactive_rotation",
                usage_percent = usage_percent,
                "Triggering proactive context rotation at 80%"
            );
            ContextAction::RotateNow
        } else {
            ContextAction::Continue
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-gateway policy::tests::test_proactive`
Expected: Test passes

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/policy.rs
git commit -m "feat(policy): add proactive context rotation at 80%"
```

---

### Task 10: Watchdog Module

**Files:**
- Create: `crates/river-gateway/src/watchdog.rs`
- Modify: `crates/river-gateway/src/lib.rs`
- Modify: `crates/river-gateway/Cargo.toml`

- [ ] **Step 1: Add sd-notify dependency**

```toml
# Add to Cargo.toml [dependencies]
sd-notify = "0.4"
```

- [ ] **Step 2: Create watchdog module**

```rust
// crates/river-gateway/src/watchdog.rs

//! Systemd watchdog integration

use std::time::Duration;
use tokio::task::JoinHandle;

/// Spawn a background task that pings the systemd watchdog
///
/// This runs independently of the agent loop. If the process hangs
/// completely (e.g., deadlock), it stops pinging and systemd restarts.
pub fn spawn_watchdog_task(interval_secs: u64) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]) {
                tracing::warn!(error = %e, "Failed to ping systemd watchdog");
            } else {
                tracing::trace!(event = "watchdog.ping", "Pinged systemd watchdog");
            }
        }
    })
}

/// Notify systemd that the service is ready
pub fn notify_ready() {
    if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]) {
        tracing::debug!(error = %e, "Failed to notify systemd ready (may not be running under systemd)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_watchdog_task_spawns() {
        let handle = spawn_watchdog_task(1);
        // Let it run briefly
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        // Should complete without panic
    }
}
```

- [ ] **Step 3: Add module to lib.rs**

```rust
// Add to crates/river-gateway/src/lib.rs
pub mod watchdog;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-gateway watchdog::tests`
Expected: Test passes

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/watchdog.rs crates/river-gateway/src/lib.rs crates/river-gateway/Cargo.toml
git commit -m "feat(watchdog): add systemd watchdog integration"
```

---

### Task 11: State Integration

**Files:**
- Modify: `crates/river-gateway/src/state.rs`

- [ ] **Step 1: Add policy to AppState**

```rust
// Modify crates/river-gateway/src/state.rs

// Add import at top:
use crate::policy::HealthPolicy;

// Add field to AppState struct:
pub struct AppState {
    // ... existing fields ...
    /// Health policy for self-healing
    pub policy: Arc<RwLock<HealthPolicy>>,
}

// Update AppState::new to accept policy parameter:
impl AppState {
    pub fn new(
        config: GatewayConfig,
        db: Arc<Mutex<Database>>,
        registry: ToolRegistry,
        embedding_client: Option<EmbeddingClient>,
        redis_client: Option<RedisClient>,
        loop_tx: mpsc::Sender<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        auth_token: Option<String>,
        subagent_manager: Arc<RwLock<SubagentManager>>,
        metrics: Arc<RwLock<AgentMetrics>>,
        policy: Arc<RwLock<HealthPolicy>>,
    ) -> Self {
        // ... existing code ...
        Self {
            // ... existing fields ...
            policy,
        }
    }
}
```

- [ ] **Step 2: Update test helper to include policy**

```rust
// Update test_state() and test_state_with_auth() in state.rs tests:

use crate::policy::HealthPolicy;

// In test functions, add:
let policy = Arc::new(RwLock::new(HealthPolicy::new(
    "test".to_string(),
    PathBuf::from("/tmp/test"),
)));

// Pass to AppState::new
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p river-gateway state::tests`
Expected: Tests pass (may need to update other test files too)

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/state.rs
git commit -m "feat(state): add HealthPolicy to AppState"
```

---

### Task 12: Server Integration

**Files:**
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Update server to create policy and spawn watchdog**

```rust
// Modify crates/river-gateway/src/server.rs

// Add imports:
use crate::policy::HealthPolicy;
use crate::watchdog::{spawn_watchdog_task, notify_ready};

// In run() function, after creating metrics:

// Create health policy
let policy = Arc::new(RwLock::new(HealthPolicy::new(
    config.agent_name.clone(),
    config.data_dir.clone(),
)));

// Update AppState::new call to pass policy

// Before starting the listener, spawn watchdog:
let _watchdog_handle = spawn_watchdog_task(30); // 30s interval

// After listener starts:
notify_ready();
```

- [ ] **Step 2: Run tests to verify compilation**

Run: `cargo build -p river-gateway`
Expected: Builds successfully

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/server.rs
git commit -m "feat(server): create policy and spawn watchdog on startup"
```

---

### Task 13: Health Endpoint Updates

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Update health response to include policy and return correct status code**

```rust
// Modify crates/river-gateway/src/api/routes.rs

use crate::policy::HealthStatus;
use axum::response::IntoResponse;

// Add PolicyInfo struct:
#[derive(Serialize)]
struct PolicyInfo {
    health_status: HealthStatus,
    consecutive_errors: u32,
    current_backoff_secs: u64,
    recovery_attempts: u32,
    attention_file: Option<String>,
}

// Update HealthResponse:
#[derive(Serialize)]
struct HealthResponse {
    status: HealthStatus, // Changed from &'static str
    version: &'static str,
    uptime_seconds: u64,
    agent: AgentInfo,
    loop_state: LoopInfo,
    context: ContextInfo,
    resources: ResourceInfo,
    counters: CounterInfo,
    policy: PolicyInfo,
}

// Update health_check handler:
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let metrics = state.metrics.read().await;
    let policy = state.policy.read().await;

    // Get current DB size
    let db_size = std::fs::metadata(state.config.db_path())
        .map(|m| m.len())
        .unwrap_or(0);

    let rss = get_rss_bytes();

    let uptime = Utc::now()
        .signed_duration_since(metrics.start_time)
        .num_seconds()
        .max(0) as u64;

    let health_status = policy.status();

    let response = HealthResponse {
        status: health_status,
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: uptime,
        agent: AgentInfo {
            name: metrics.agent_name.clone(),
            birth: metrics.agent_birth,
        },
        loop_state: LoopInfo {
            state: metrics.loop_state,
            last_wake: metrics.last_wake,
            last_settle: metrics.last_settle,
            turns_since_restart: metrics.turns_since_restart,
        },
        context: ContextInfo {
            current_tokens: metrics.context_tokens,
            limit_tokens: metrics.context_limit,
            usage_percent: metrics.context_usage_percent(),
            context_id: metrics.context_id.clone(),
            rotations: metrics.rotations_since_restart,
        },
        resources: ResourceInfo {
            db_size_bytes: db_size,
            rss_bytes: rss,
        },
        counters: CounterInfo {
            model_calls: metrics.model_calls,
            tool_calls: metrics.tool_calls,
            tool_errors: metrics.tool_errors,
        },
        policy: PolicyInfo {
            health_status,
            consecutive_errors: policy.consecutive_errors(),
            current_backoff_secs: policy.error_backoff().as_secs(),
            recovery_attempts: policy.recovery_attempts(),
            attention_file: policy.attention_file_path(),
        },
    };

    // Return 503 for NeedsAttention
    let status_code = match health_status {
        HealthStatus::Healthy | HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::NeedsAttention => StatusCode::SERVICE_UNAVAILABLE,
    };

    (status_code, Json(response))
}
```

- [ ] **Step 2: Update tests**

Update route tests to handle new response format and policy field.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p river-gateway api::routes::tests`
Expected: Tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/api/routes.rs
git commit -m "feat(health): include policy status and return 503 for NeedsAttention"
```

---

### Task 14: Loop Integration

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Add policy to AgentLoop**

```rust
// Add import:
use crate::policy::{HealthPolicy, compute_action_hash, ContextAction};

// Add field to AgentLoop struct:
pub struct AgentLoop {
    // ... existing fields ...
    policy: Arc<RwLock<HealthPolicy>>,
    turn_total_calls: u32,
    turn_failed_calls: u32,
}

// Update constructor to accept policy
```

- [ ] **Step 2: Integrate policy at wake phase**

```rust
// In wake phase, add:

// Check if ATTENTION.md was cleared by human
if let Some(human_response) = self.policy.write().await.check_attention_cleared() {
    // Add human guidance to context for next turn
    self.pending_notifications.push(format!(
        "Human responded to your ATTENTION request: {}",
        human_response
    ));
}

// Set pending messages count
let pending = self.message_queue.len().await as u32;
self.policy.write().await.set_pending_messages(pending);

// Record context tokens at turn start
let start_tokens = self.last_prompt_tokens;
self.policy.write().await.set_turn_start_tokens(start_tokens);

// Reset turn counters
self.turn_total_calls = 0;
self.turn_failed_calls = 0;
```

- [ ] **Step 3: Integrate policy at think phase**

```rust
// Before model call, check backoff:
let delay = self.policy.read().await.error_backoff();
if !delay.is_zero() {
    tracing::info!(
        event = "backoff.wait",
        delay_secs = delay.as_secs(),
        "Waiting before next model call"
    );
    tokio::time::sleep(delay).await;
}

// After successful model call:
let action_hash = compute_action_hash(&response.tool_calls);
self.policy.write().await.check_repeated_action(action_hash);
```

- [ ] **Step 4: Integrate policy at act phase**

```rust
// Before tool execution:
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

// After tool execution:
self.turn_total_calls += 1;
if result.is_err() {
    self.turn_failed_calls += 1;
}
self.policy.write().await.on_tool_result(
    &tool_call.name,
    result.is_ok(),
    result.duration,
);
```

- [ ] **Step 5: Integrate policy at settle phase**

```rust
// Record context tokens at turn end
let start = self.policy.read().await.context_tokens_at_turn_start;
let end = self.last_prompt_tokens;
self.policy.write().await.record_turn_tokens(start, end);

// Mark turn complete with call counts
self.policy.write().await.on_turn_complete(
    self.turn_total_calls,
    self.turn_failed_calls,
);

// Check for proactive context rotation
let usage = self.metrics.read().await.context_usage_percent();
if self.policy.read().await.on_context_warning(usage) == ContextAction::RotateNow {
    // Trigger rotation
    self.context_rotation.request_rotation("proactive: 80% threshold");
}
```

- [ ] **Step 6: Run tests to verify compilation**

Run: `cargo build -p river-gateway`
Expected: Builds successfully

- [ ] **Step 7: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "feat(loop): integrate health policy at all phase transitions"
```

---

### Task 15: Final Integration Test

**Files:**
- All modified files

- [ ] **Step 1: Run full test suite**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p river-gateway -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Test manually**

Run: `cargo run -p river-gateway -- --help`
Expected: Shows help without errors

- [ ] **Step 4: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix: address integration issues from testing"
```

---

## Summary

This plan implements the complete self-healing agent policy in 15 tasks:

1. **Tasks 1-3:** Core policy types, tool backoff, error backoff
2. **Tasks 4-6:** Model error handling, stuck detection (repeated action + low progress)
3. **Tasks 7-8:** ATTENTION.md escalation and bidirectional communication
4. **Tasks 9-10:** Recovery memory and proactive context rotation
5. **Tasks 11-14:** Integration (state, server, health endpoint, loop)
6. **Task 15:** Final testing

Each task is self-contained with tests, implementation, and commit.

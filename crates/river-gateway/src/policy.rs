//! Self-healing agent policy

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Decay period for tool failures (1 hour)
const TOOL_FAILURE_DECAY_SECS: i64 = 3600;

/// Maximum backoff for consecutive errors (15 minutes)
const BACKOFF_CAP_SECS: u64 = 900;

/// Health status reflecting agent capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    #[default]
    Healthy,
    Degraded,
    NeedsAttention,
}

/// Action to take after model error
#[derive(Debug, Clone, PartialEq)]
pub enum ModelErrorAction {
    RetryAfter(Duration),
    RetryWithBackoff(Duration),
    NoRetry,
    Escalated,
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
    pub fn data_dir(&self) -> &Path {
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

        // 4th failure = 4 min
        policy.on_tool_result("bash", false, Duration::from_millis(100));
        assert_eq!(policy.tool_backoff("bash"), Duration::from_secs(240));
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

    #[test]
    fn test_zero_calls_is_clean_turn() {
        let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

        // Degrade the agent first
        policy.on_turn_complete(1, 1);
        policy.on_turn_complete(1, 1);
        assert_eq!(policy.status(), HealthStatus::Degraded);

        // Zero calls is treated as 100% success (clean turn)
        policy.on_turn_complete(0, 0);
        assert_eq!(policy.status(), HealthStatus::Healthy);
        assert_eq!(policy.consecutive_errors(), 0);
    }

    #[test]
    fn test_partial_failure_accumulates_one_error() {
        let mut policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

        // 60% success (3/5 succeed) = between 50-80%, adds 1 error
        policy.on_turn_complete(5, 2);
        assert_eq!(policy.consecutive_errors(), 1);
        assert_eq!(policy.status(), HealthStatus::Healthy); // Still healthy with 1 error

        // Another 60% success adds 1 more, now 2 errors = Degraded
        policy.on_turn_complete(5, 2);
        assert_eq!(policy.consecutive_errors(), 2);
        assert_eq!(policy.status(), HealthStatus::Degraded);
    }

    // Task 4: Model error handling tests

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
}

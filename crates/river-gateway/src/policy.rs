//! Self-healing agent policy

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// Action for context management
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Continue,
    RotateNow,
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
    ///
    /// Called in wake phase. Heartbeats are self-initiated check-ins that
    /// correctly produce minimal output, so they shouldn't trigger stuck detection.
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
    ///
    /// The `duration` parameter is reserved for future performance anomaly detection
    /// (e.g., detecting tools that are taking unusually long). Currently unused.
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
                self.log_recovery("Turn completed with 80%+ success rate");
                self.status = HealthStatus::Healthy;
                self.recovery_attempts = 0;
                self.last_recovery = Some(Utc::now());
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

    fn log_recovery(&self, reason: &str) {
        let recovery_log = self.data_dir.join("recovery.jsonl");

        let entry = serde_json::json!({
            "timestamp": Utc::now().to_rfc3339(),
            "agent": self.agent_name,
            "previous_status": self.status,
            "recovery_reason": reason,
            "context": {
                "consecutive_errors_before": self.consecutive_errors,
                "recovery_attempts": self.recovery_attempts,
                "last_error": self.last_error.map(|t| t.to_rfc3339()),
                "tool_failures": self.tool_failures.keys().collect::<Vec<_>>(),
            }
        });

        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&recovery_log)
        {
            Ok(mut file) => {
                if let Err(e) = writeln!(file, "{}", entry) {
                    tracing::warn!(
                        event = "recovery.log_write_failed",
                        error = %e,
                        "Failed to write recovery log"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    event = "recovery.log_open_failed",
                    path = %recovery_log.display(),
                    error = %e,
                    "Failed to open recovery log file"
                );
            }
        }

        tracing::info!(
            event = "recovery.complete",
            reason = reason,
            attempts = self.recovery_attempts,
            "Agent recovered"
        );
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

    /// Record token counts for progress tracking
    ///
    /// This is called once per wake-settle cycle (not per think-act iteration).
    /// A single wake may involve multiple think→act loops before settling,
    /// but this method is only invoked in the settle phase to record final progress.
    pub fn record_turn_tokens(&mut self, start_tokens: u64, end_tokens: u64) {
        self.context_tokens_at_turn_start = start_tokens;
        self.context_tokens_at_turn_end = end_tokens;

        // Skip progress tracking for heartbeat turns - a heartbeat producing
        // minimal tokens is correct behavior, not stuck behavior
        if self.is_heartbeat_turn {
            self.is_heartbeat_turn = false; // Reset for next wake-settle cycle
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
        match std::fs::read_to_string(&attention_path) {
            Ok(content) => {
                if let Some(response) = Self::parse_human_response(&content) {
                    tracing::info!(
                        event = "attention.response",
                        response = %response,
                        "Human responded to ATTENTION.md"
                    );
                    return Some(response);
                }
            }
            Err(e) => {
                tracing::warn!(
                    event = "attention.read_error",
                    error = %e,
                    path = %attention_path.display(),
                    "Failed to read ATTENTION.md"
                );
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

    // Task 5: Stuck detection - repeated action tests

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

    #[test]
    fn test_stuck_escalation_after_four_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = HealthPolicy::new("test".to_string(), dir.path().to_path_buf());
        let hash = 12345u64;

        // mark_stuck is called on every check after count >= 3
        // So: 1st call: count=1, 2nd: count=2, 3rd: count=3 -> mark_stuck (recovery_attempts=1)
        // 4th: count=4 -> mark_stuck (recovery_attempts=2), etc.
        // We need recovery_attempts > 3, so at least 4 calls to mark_stuck
        // That means 6 repeated actions: 3 to reach threshold, 3 more to trigger 3 more mark_stuck
        // Actually: calls 3,4,5,6 each trigger mark_stuck = 4 times, recovery_attempts=4 > 3 triggers escalate
        for _ in 0..6 {
            policy.check_repeated_action(hash);
        }

        assert!(policy.recovery_attempts() > 3);
        assert_eq!(policy.status(), HealthStatus::NeedsAttention);
        assert!(dir.path().join("ATTENTION.md").exists());
    }

    // Task 6: Low progress stuck detection tests

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

    // Task 7: Bidirectional ATTENTION.md tests

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

    // Task 8: Recovery memory logging tests

    // Task 9: Proactive context rotation tests

    #[test]
    fn test_proactive_rotation_at_80_percent() {
        let policy = HealthPolicy::new("test".to_string(), PathBuf::from("/tmp"));

        assert_eq!(policy.on_context_warning(70.0), ContextAction::Continue);
        assert_eq!(policy.on_context_warning(80.0), ContextAction::RotateNow);
        assert_eq!(policy.on_context_warning(85.0), ContextAction::RotateNow);
        assert_eq!(policy.on_context_warning(90.0), ContextAction::Continue); // 90%+ handled elsewhere
    }

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
}

//! Self-healing agent policy

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Health status reflecting agent capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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

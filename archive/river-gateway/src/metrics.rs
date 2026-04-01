//! Shared agent metrics for observability

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Observable loop state (no associated data, for serialization)
#[derive(Debug, Clone, Copy, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LoopStateLabel {
    #[default]
    Sleeping,
    Waking,
    Thinking,
    Acting,
    Settling,
}

/// Shared metrics updated by AgentLoop, read by health endpoint
#[derive(Debug, Clone, Serialize)]
pub struct AgentMetrics {
    // Identity
    pub agent_name: String,
    pub agent_birth: DateTime<Utc>,
    pub start_time: DateTime<Utc>,

    // Loop state
    pub loop_state: LoopStateLabel,
    pub last_wake: Option<DateTime<Utc>>,
    pub last_settle: Option<DateTime<Utc>>,
    pub turns_since_restart: u64,

    // Context
    pub context_tokens: u64,
    pub context_limit: u64,
    pub context_id: Option<String>,
    pub rotations_since_restart: u64,

    // Resources (updated on health check, not continuously)
    pub db_size_bytes: u64,
    pub rss_bytes: u64,

    // Counters (reset on restart)
    pub model_calls: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
}

impl AgentMetrics {
    pub fn new(agent_name: String, agent_birth: DateTime<Utc>, context_limit: u64) -> Self {
        Self {
            agent_name,
            agent_birth,
            start_time: Utc::now(),
            loop_state: LoopStateLabel::default(),
            last_wake: None,
            last_settle: None,
            turns_since_restart: 0,
            context_tokens: 0,
            context_limit,
            context_id: None,
            rotations_since_restart: 0,
            db_size_bytes: 0,
            rss_bytes: 0,
            model_calls: 0,
            tool_calls: 0,
            tool_errors: 0,
        }
    }

    /// Get context usage as percentage
    pub fn context_usage_percent(&self) -> f64 {
        if self.context_limit == 0 {
            0.0
        } else {
            (self.context_tokens as f64 / self.context_limit as f64) * 100.0
        }
    }
}

/// Get resident set size in bytes (Linux only)
pub fn get_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map(|pages| pages * 4096)
            .unwrap_or(0)
    }

    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_state_label_default() {
        assert_eq!(LoopStateLabel::default(), LoopStateLabel::Sleeping);
    }

    #[test]
    fn test_loop_state_label_serializes_snake_case() {
        let json = serde_json::to_string(&LoopStateLabel::Thinking).unwrap();
        assert_eq!(json, "\"thinking\"");
    }

    #[test]
    fn test_agent_metrics_new() {
        let birth = Utc::now();
        let metrics = AgentMetrics::new("test".to_string(), birth, 100000);

        assert_eq!(metrics.agent_name, "test");
        assert_eq!(metrics.context_limit, 100000);
        assert_eq!(metrics.turns_since_restart, 0);
        assert_eq!(metrics.loop_state, LoopStateLabel::Sleeping);
    }

    #[test]
    fn test_context_usage_percent() {
        let birth = Utc::now();
        let mut metrics = AgentMetrics::new("test".to_string(), birth, 100000);
        metrics.context_tokens = 25000;

        assert!((metrics.context_usage_percent() - 25.0).abs() < 0.01);
    }

    #[test]
    fn test_context_usage_percent_zero_limit() {
        let birth = Utc::now();
        let metrics = AgentMetrics::new("test".to_string(), birth, 0);

        assert_eq!(metrics.context_usage_percent(), 0.0);
    }

    #[test]
    fn test_get_rss_bytes_returns_value() {
        let rss = get_rss_bytes();
        // On Linux, should be non-zero; elsewhere, 0
        #[cfg(target_os = "linux")]
        assert!(rss > 0, "RSS should be positive on Linux");

        #[cfg(not(target_os = "linux"))]
        assert_eq!(rss, 0, "RSS should be 0 on non-Linux");
    }
}

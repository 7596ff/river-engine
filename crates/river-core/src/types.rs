//! Shared types for River Engine
//!
//! This module contains common types used throughout River Engine,
//! including priority levels, subagent types, and context status.

use serde::{Deserialize, Serialize};

/// Task execution priority levels.
///
/// Priority determines the order in which tasks are executed when
/// resources are constrained. Higher priority tasks preempt lower
/// priority ones.
///
/// # Ordering
///
/// `Interactive` > `Scheduled` > `Background`
///
/// # Examples
///
/// ```
/// use river_core::Priority;
///
/// let p1 = Priority::Background;
/// let p2 = Priority::Interactive;
/// assert!(p2 > p1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Lowest priority - for background maintenance tasks
    Background = 0,
    /// Medium priority - for scheduled/automated tasks
    Scheduled = 1,
    /// Highest priority - for user-interactive tasks
    Interactive = 2,
}

impl Default for Priority {
    fn default() -> Self {
        Self::Background
    }
}

/// Types of subagents that can be spawned by the orchestrator.
///
/// Subagents are specialized agents that handle specific types of work.
/// The type determines how the orchestrator manages the subagent's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentType {
    /// A task worker subagent that processes discrete tasks
    ///
    /// Task workers are spawned to handle individual tasks and
    /// terminate when the task is complete.
    TaskWorker,
    /// A long-running subagent that stays active
    ///
    /// Long-running subagents maintain state and handle multiple
    /// requests over their lifetime.
    LongRunning,
}

/// Current context window usage status.
///
/// Tracks how much of the agent's context window has been used,
/// allowing for proactive context management before hitting limits.
///
/// # Examples
///
/// ```
/// use river_core::ContextStatus;
///
/// let status = ContextStatus { used: 90_000, limit: 100_000 };
/// assert_eq!(status.percent(), 90.0);
/// assert!(status.is_near_limit());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextStatus {
    /// Number of tokens currently used
    pub used: u64,
    /// Maximum tokens allowed in context
    pub limit: u64,
}

impl ContextStatus {
    /// Creates a new ContextStatus with the given used and limit values.
    pub fn new(used: u64, limit: u64) -> Self {
        Self { used, limit }
    }

    /// Returns the percentage of context used (0.0 to 100.0+).
    ///
    /// Returns 0.0 if limit is 0 to avoid division by zero.
    pub fn percent(&self) -> f64 {
        if self.limit == 0 {
            return 0.0;
        }
        (self.used as f64 / self.limit as f64) * 100.0
    }

    /// Returns true if context usage is at or above 90%.
    ///
    /// This threshold indicates the agent should consider
    /// context compaction or handoff.
    pub fn is_near_limit(&self) -> bool {
        self.percent() >= 90.0
    }

    /// Returns the number of tokens remaining.
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod priority_tests {
        use super::*;

        #[test]
        fn test_priority_ordering() {
            assert!(Priority::Interactive > Priority::Scheduled);
            assert!(Priority::Scheduled > Priority::Background);
            assert!(Priority::Interactive > Priority::Background);
        }

        #[test]
        fn test_priority_default() {
            assert_eq!(Priority::default(), Priority::Background);
        }

        #[test]
        fn test_priority_serialize() {
            let bg = Priority::Background;
            let json = serde_json::to_string(&bg).unwrap();
            assert_eq!(json, "\"background\"");

            let sched = Priority::Scheduled;
            let json = serde_json::to_string(&sched).unwrap();
            assert_eq!(json, "\"scheduled\"");

            let inter = Priority::Interactive;
            let json = serde_json::to_string(&inter).unwrap();
            assert_eq!(json, "\"interactive\"");
        }

        #[test]
        fn test_priority_deserialize() {
            let bg: Priority = serde_json::from_str("\"background\"").unwrap();
            assert_eq!(bg, Priority::Background);

            let sched: Priority = serde_json::from_str("\"scheduled\"").unwrap();
            assert_eq!(sched, Priority::Scheduled);

            let inter: Priority = serde_json::from_str("\"interactive\"").unwrap();
            assert_eq!(inter, Priority::Interactive);
        }

        #[test]
        fn test_priority_roundtrip() {
            for priority in [
                Priority::Background,
                Priority::Scheduled,
                Priority::Interactive,
            ] {
                let json = serde_json::to_string(&priority).unwrap();
                let deserialized: Priority = serde_json::from_str(&json).unwrap();
                assert_eq!(priority, deserialized);
            }
        }

        #[test]
        fn test_priority_values() {
            assert_eq!(Priority::Background as u8, 0);
            assert_eq!(Priority::Scheduled as u8, 1);
            assert_eq!(Priority::Interactive as u8, 2);
        }
    }

    mod subagent_type_tests {
        use super::*;

        #[test]
        fn test_subagent_type_serialize() {
            let tw = SubagentType::TaskWorker;
            let json = serde_json::to_string(&tw).unwrap();
            assert_eq!(json, "\"task_worker\"");

            let lr = SubagentType::LongRunning;
            let json = serde_json::to_string(&lr).unwrap();
            assert_eq!(json, "\"long_running\"");
        }

        #[test]
        fn test_subagent_type_deserialize() {
            let tw: SubagentType = serde_json::from_str("\"task_worker\"").unwrap();
            assert_eq!(tw, SubagentType::TaskWorker);

            let lr: SubagentType = serde_json::from_str("\"long_running\"").unwrap();
            assert_eq!(lr, SubagentType::LongRunning);
        }

        #[test]
        fn test_subagent_type_roundtrip() {
            for subagent_type in [SubagentType::TaskWorker, SubagentType::LongRunning] {
                let json = serde_json::to_string(&subagent_type).unwrap();
                let deserialized: SubagentType = serde_json::from_str(&json).unwrap();
                assert_eq!(subagent_type, deserialized);
            }
        }
    }

    mod context_status_tests {
        use super::*;

        #[test]
        fn test_context_status_percent() {
            let status = ContextStatus::new(50_000, 100_000);
            assert_eq!(status.percent(), 50.0);

            let status = ContextStatus::new(90_000, 100_000);
            assert_eq!(status.percent(), 90.0);

            let status = ContextStatus::new(100_000, 100_000);
            assert_eq!(status.percent(), 100.0);
        }

        #[test]
        fn test_context_status_percent_zero_limit() {
            let status = ContextStatus::new(0, 0);
            assert_eq!(status.percent(), 0.0);

            let status = ContextStatus::new(100, 0);
            assert_eq!(status.percent(), 0.0);
        }

        #[test]
        fn test_context_status_is_near_limit() {
            let status = ContextStatus::new(89_999, 100_000);
            assert!(!status.is_near_limit());

            let status = ContextStatus::new(90_000, 100_000);
            assert!(status.is_near_limit());

            let status = ContextStatus::new(95_000, 100_000);
            assert!(status.is_near_limit());

            let status = ContextStatus::new(100_000, 100_000);
            assert!(status.is_near_limit());
        }

        #[test]
        fn test_context_status_remaining() {
            let status = ContextStatus::new(50_000, 100_000);
            assert_eq!(status.remaining(), 50_000);

            let status = ContextStatus::new(100_000, 100_000);
            assert_eq!(status.remaining(), 0);

            // Test saturating subtraction (should not underflow)
            let status = ContextStatus::new(150_000, 100_000);
            assert_eq!(status.remaining(), 0);
        }

        #[test]
        fn test_context_status_serde_roundtrip() {
            let status = ContextStatus::new(75_000, 100_000);
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: ContextStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }
}

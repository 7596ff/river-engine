//! Respawn policy and wake timers.

use river_adapter::Side;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Worker exit status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExitStatus {
    Done {
        wake_after_minutes: Option<u64>,
    },
    ContextExhausted,
    Error {
        message: String,
    },
}

/// Worker output sent to orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerOutput {
    pub dyad: String,
    pub side: Side,
    pub status: ExitStatus,
    pub summary: String,
}

/// Response to worker output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputAck {
    pub acknowledged: bool,
}

/// Respawn state for a worker.
#[derive(Debug, Clone)]
pub struct RespawnState {
    pub summary: Option<String>,
    pub wake_at: Option<Instant>,
    pub start_sleeping: bool,
}

/// Worker key for respawn state.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct WorkerKey {
    pub dyad: String,
    pub side: Side,
}

/// Respawn manager.
#[derive(Debug, Default)]
pub struct RespawnManager {
    states: HashMap<WorkerKey, RespawnState>,
}

impl RespawnManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process worker output and determine respawn behavior.
    pub fn process_output(&mut self, output: &WorkerOutput) -> RespawnAction {
        let key = WorkerKey {
            dyad: output.dyad.clone(),
            side: output.side.clone(),
        };

        match &output.status {
            ExitStatus::Done { wake_after_minutes: None } => {
                // Respawn immediately with start_sleeping: true
                self.states.insert(
                    key,
                    RespawnState {
                        summary: None, // Don't need summary, worker will sleep
                        wake_at: None,
                        start_sleeping: true,
                    },
                );
                RespawnAction::ImmediateWithSleep
            }
            ExitStatus::Done { wake_after_minutes: Some(minutes) } => {
                // Wait, then respawn with summary
                let wake_at = Instant::now() + Duration::from_secs(minutes * 60);
                self.states.insert(
                    key,
                    RespawnState {
                        summary: Some(output.summary.clone()),
                        wake_at: Some(wake_at),
                        start_sleeping: false,
                    },
                );
                RespawnAction::WaitThenRespawn { minutes: *minutes }
            }
            ExitStatus::ContextExhausted => {
                // Respawn immediately with summary
                self.states.insert(
                    key,
                    RespawnState {
                        summary: Some(output.summary.clone()),
                        wake_at: None,
                        start_sleeping: false,
                    },
                );
                RespawnAction::ImmediateWithSummary
            }
            ExitStatus::Error { .. } => {
                // Respawn immediately, worker loads from JSONL
                self.states.remove(&key);
                RespawnAction::ImmediateFromJSONL
            }
        }
    }

    /// Get respawn info for a worker.
    pub fn get_respawn_info(&self, dyad: &str, side: &Side) -> Option<&RespawnState> {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        self.states.get(&key)
    }

    /// Clear respawn state after successful respawn.
    pub fn clear(&mut self, dyad: &str, side: &Side) {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        self.states.remove(&key);
    }

    /// Get workers ready to wake up.
    pub fn ready_to_wake(&self) -> Vec<WorkerKey> {
        let now = Instant::now();
        self.states
            .iter()
            .filter_map(|(k, s)| {
                if let Some(wake_at) = s.wake_at {
                    if now >= wake_at {
                        return Some(k.clone());
                    }
                }
                None
            })
            .collect()
    }

    /// Get next wake time for scheduling.
    pub fn next_wake_time(&self) -> Option<Instant> {
        self.states
            .values()
            .filter_map(|s| s.wake_at)
            .min()
    }
}

/// What to do when a worker exits.
#[derive(Debug, Clone)]
pub enum RespawnAction {
    /// Respawn immediately with start_sleeping: true
    ImmediateWithSleep,
    /// Wait N minutes, then respawn with initial_message
    WaitThenRespawn { minutes: u64 },
    /// Respawn immediately with initial_message set to summary
    ImmediateWithSummary,
    /// Respawn immediately, worker loads from JSONL
    ImmediateFromJSONL,
}

/// Thread-safe respawn manager.
pub type SharedRespawnManager = Arc<RwLock<RespawnManager>>;

pub fn new_shared_respawn_manager() -> SharedRespawnManager {
    Arc::new(RwLock::new(RespawnManager::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_done_no_wake() {
        let mut mgr = RespawnManager::new();
        let output = WorkerOutput {
            dyad: "test".into(),
            side: Side::Left,
            status: ExitStatus::Done { wake_after_minutes: None },
            summary: "done".into(),
        };
        let action = mgr.process_output(&output);
        assert!(matches!(action, RespawnAction::ImmediateWithSleep));

        let state = mgr.get_respawn_info("test", &Side::Left).unwrap();
        assert!(state.start_sleeping);
    }

    #[test]
    fn test_context_exhausted() {
        let mut mgr = RespawnManager::new();
        let output = WorkerOutput {
            dyad: "test".into(),
            side: Side::Left,
            status: ExitStatus::ContextExhausted,
            summary: "ran out of context".into(),
        };
        let action = mgr.process_output(&output);
        assert!(matches!(action, RespawnAction::ImmediateWithSummary));

        let state = mgr.get_respawn_info("test", &Side::Left).unwrap();
        assert_eq!(state.summary.as_deref(), Some("ran out of context"));
    }
}

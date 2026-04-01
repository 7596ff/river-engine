//! Loop state machine types

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Tool call as returned by the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub r#type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Events that can wake or signal the loop
#[derive(Debug, Clone)]
pub enum LoopEvent {
    /// New messages written to inbox files
    InboxUpdate(Vec<PathBuf>),
    /// Heartbeat timer fired
    Heartbeat,
    /// Graceful shutdown requested
    Shutdown,
}

/// What caused the agent to wake
#[derive(Debug, Clone)]
pub enum WakeTrigger {
    /// New messages in inbox files
    Inbox(Vec<PathBuf>),
    /// Scheduled heartbeat
    Heartbeat,
}

/// The agent's current phase in the cycle
#[derive(Debug, Clone, Default)]
pub enum LoopState {
    /// Waiting for next event
    #[default]
    Sleeping,
    /// Woke up, assembling context
    Waking { trigger: WakeTrigger },
    /// Model is generating
    Thinking,
    /// Executing tool calls
    Acting { pending: Vec<ToolCallRequest> },
    /// Cycle complete, committing state
    Settling,
}

impl LoopState {
    /// Check if loop is in a phase where messages should be queued
    pub fn should_queue_messages(&self) -> bool {
        matches!(self, LoopState::Thinking | LoopState::Acting { .. })
    }

    /// Check if loop is sleeping
    pub fn is_sleeping(&self) -> bool {
        matches!(self, LoopState::Sleeping)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_queue_messages() {
        assert!(!LoopState::Sleeping.should_queue_messages());
        assert!(!LoopState::Settling.should_queue_messages());
        assert!(LoopState::Thinking.should_queue_messages());
        assert!(LoopState::Acting { pending: vec![] }.should_queue_messages());
    }

    #[test]
    fn test_should_queue_messages_waking() {
        let waking = LoopState::Waking {
            trigger: WakeTrigger::Heartbeat,
        };
        // Waking state should not queue - messages should be processed
        assert!(!waking.should_queue_messages());
    }

    #[test]
    fn test_is_sleeping() {
        assert!(LoopState::Sleeping.is_sleeping());
        assert!(!LoopState::Thinking.is_sleeping());
        assert!(!LoopState::Acting { pending: vec![] }.is_sleeping());
    }

    #[test]
    fn test_default_state_is_sleeping() {
        assert!(LoopState::default().is_sleeping());
    }

    #[test]
    fn test_loop_event_heartbeat() {
        let event = LoopEvent::Heartbeat;
        assert!(matches!(event, LoopEvent::Heartbeat));
    }

    #[test]
    fn test_loop_event_shutdown() {
        let event = LoopEvent::Shutdown;
        assert!(matches!(event, LoopEvent::Shutdown));
    }

    #[test]
    fn test_wake_trigger_heartbeat() {
        let trigger = WakeTrigger::Heartbeat;
        assert!(matches!(trigger, WakeTrigger::Heartbeat));
    }

    #[test]
    fn test_all_states_have_defined_behavior() {
        // Verify every state variant has explicit should_queue_messages behavior
        let states = vec![
            LoopState::Sleeping,
            LoopState::Waking { trigger: WakeTrigger::Heartbeat },
            LoopState::Waking { trigger: WakeTrigger::Inbox(vec![]) },
            LoopState::Thinking,
            LoopState::Acting { pending: vec![] },
            LoopState::Settling,
        ];

        let queue_states: Vec<bool> = states.iter()
            .map(|s| s.should_queue_messages())
            .collect();

        // Only Thinking and Acting should queue
        assert_eq!(queue_states, vec![false, false, false, true, true, false]);
    }

    #[test]
    fn test_loop_event_inbox_update() {
        let paths = vec![PathBuf::from("/inbox/discord/123/456.txt")];
        let event = LoopEvent::InboxUpdate(paths.clone());
        match event {
            LoopEvent::InboxUpdate(p) => {
                assert_eq!(p.len(), 1);
                assert_eq!(p[0], PathBuf::from("/inbox/discord/123/456.txt"));
            }
            _ => panic!("Expected InboxUpdate event"),
        }
    }

    #[test]
    fn test_wake_trigger_inbox() {
        let paths = vec![PathBuf::from("/inbox/test.txt")];
        let trigger = WakeTrigger::Inbox(paths);
        match trigger {
            WakeTrigger::Inbox(p) => {
                assert_eq!(p.len(), 1);
            }
            _ => panic!("Expected Inbox trigger"),
        }
    }

    #[test]
    fn test_waking_with_inbox_trigger() {
        let paths = vec![PathBuf::from("/inbox/test.txt")];
        let state = LoopState::Waking {
            trigger: WakeTrigger::Inbox(paths),
        };
        assert!(!state.is_sleeping());
        assert!(!state.should_queue_messages());
    }
}

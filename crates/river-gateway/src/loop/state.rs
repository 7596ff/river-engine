//! Loop state machine types

use crate::api::IncomingMessage;

/// Events that can wake or signal the loop
#[derive(Debug, Clone)]
pub enum LoopEvent {
    /// Message from communication adapter
    Message(IncomingMessage),
    /// Heartbeat timer fired
    Heartbeat,
    /// Graceful shutdown requested
    Shutdown,
}

/// What caused the agent to wake
#[derive(Debug, Clone)]
pub enum WakeTrigger {
    /// User or external message
    Message(IncomingMessage),
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
    Acting,
    /// Cycle complete, committing state
    Settling,
}

impl LoopState {
    /// Check if loop is in a phase where messages should be queued
    pub fn should_queue_messages(&self) -> bool {
        matches!(self, LoopState::Thinking | LoopState::Acting)
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
        assert!(LoopState::Acting.should_queue_messages());
    }

    #[test]
    fn test_is_sleeping() {
        assert!(LoopState::Sleeping.is_sleeping());
        assert!(!LoopState::Thinking.is_sleeping());
        assert!(!LoopState::Acting.is_sleeping());
    }

    #[test]
    fn test_default_state_is_sleeping() {
        assert!(LoopState::default().is_sleeping());
    }
}

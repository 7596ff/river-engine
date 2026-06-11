//! The naive in-memory rolling context (barebones harness). A
//! bounded message buffer with turn tags: oldest whole turns fall off
//! when the buffer exceeds its cap. Swapped for the persistent
//! context object (wall ch. 03) in a later card — turn-atomicity is
//! already honored here so the swap changes the machinery, not the
//! semantics.

use std::collections::VecDeque;

use crate::model::{ChatMessage, Role};

const DEFAULT_MAX_MESSAGES: usize = 200;

#[derive(Debug)]
pub struct Entry {
    pub turn: u64,
    pub message: ChatMessage,
}

#[derive(Debug)]
pub struct RollingContext {
    entries: VecDeque<Entry>,
    max_messages: usize,
}

impl RollingContext {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_MESSAGES)
    }

    pub fn with_capacity(max_messages: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_messages: max_messages.max(1),
        }
    }

    pub fn push(&mut self, turn: u64, role: Role, content: impl Into<String>) {
        self.entries.push_back(Entry {
            turn,
            message: ChatMessage {
                role,
                content: content.into(),
            },
        });
        self.roll();
    }

    /// Drop oldest whole turns until within the cap. The newest turn
    /// is never dropped, even if it alone exceeds the cap.
    fn roll(&mut self) {
        while self.entries.len() > self.max_messages {
            let oldest_turn = self.entries.front().expect("non-empty").turn;
            let newest_turn = self.entries.back().expect("non-empty").turn;
            if oldest_turn == newest_turn {
                break;
            }
            while self
                .entries
                .front()
                .is_some_and(|e| e.turn == oldest_turn)
            {
                self.entries.pop_front();
            }
        }
    }

    pub fn messages(&self) -> Vec<ChatMessage> {
        self.entries.iter().map(|e| e.message.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_in_order() {
        let mut ctx = RollingContext::new();
        ctx.push(1, Role::User, "[local_main] cass: hello");
        ctx.push(1, Role::Assistant, "hi");
        let messages = ctx.messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::Assistant);
    }

    #[test]
    fn rolls_whole_turns() {
        let mut ctx = RollingContext::with_capacity(4);
        ctx.push(1, Role::User, "a");
        ctx.push(1, Role::Assistant, "b");
        ctx.push(2, Role::User, "c");
        ctx.push(2, Role::Assistant, "d");
        ctx.push(3, Role::User, "e");
        // Cap exceeded: turn 1 drops as a unit; turn 2 stays whole.
        let messages = ctx.messages();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "c");
    }

    #[test]
    fn newest_turn_never_drops() {
        let mut ctx = RollingContext::with_capacity(2);
        ctx.push(7, Role::User, "a");
        ctx.push(7, Role::Assistant, "b");
        ctx.push(7, Role::User, "c");
        assert_eq!(ctx.len(), 3);
    }
}

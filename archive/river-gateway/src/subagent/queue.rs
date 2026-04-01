//! Internal message queue for subagent communication
//!
//! Provides bidirectional communication between parent agent and subagents.
//! Each subagent has its own queue instance.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

/// An internal message between parent and subagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalMessage {
    /// Message content
    pub content: String,
    /// Timestamp (Unix seconds)
    pub timestamp: i64,
    /// Direction: true = to_subagent, false = to_parent
    pub to_subagent: bool,
}

impl InternalMessage {
    /// Create a message to send to subagent
    pub fn to_subagent(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            to_subagent: true,
        }
    }

    /// Create a message to send to parent
    pub fn to_parent(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            to_subagent: false,
        }
    }
}

/// Thread-safe bidirectional queue for parent-subagent communication
pub struct InternalQueue {
    /// Messages from parent to subagent
    to_subagent: Mutex<VecDeque<InternalMessage>>,
    /// Messages from subagent to parent
    to_parent: Mutex<VecDeque<InternalMessage>>,
}

impl InternalQueue {
    pub fn new() -> Self {
        Self {
            to_subagent: Mutex::new(VecDeque::new()),
            to_parent: Mutex::new(VecDeque::new()),
        }
    }

    /// Parent sends a message to subagent
    pub fn send_to_subagent(&self, content: impl Into<String>) {
        let msg = InternalMessage::to_subagent(content);
        let mut queue = self.to_subagent.lock().unwrap();
        queue.push_back(msg);
    }

    /// Subagent sends a message to parent
    pub fn send_to_parent(&self, content: impl Into<String>) {
        let msg = InternalMessage::to_parent(content);
        let mut queue = self.to_parent.lock().unwrap();
        queue.push_back(msg);
    }

    /// Subagent drains all messages sent to it
    pub fn drain_for_subagent(&self) -> Vec<InternalMessage> {
        let mut queue = self.to_subagent.lock().unwrap();
        queue.drain(..).collect()
    }

    /// Parent drains all messages sent to it
    pub fn drain_for_parent(&self) -> Vec<InternalMessage> {
        let mut queue = self.to_parent.lock().unwrap();
        queue.drain(..).collect()
    }

    /// Check if there are messages waiting for subagent
    pub fn has_messages_for_subagent(&self) -> bool {
        let queue = self.to_subagent.lock().unwrap();
        !queue.is_empty()
    }

    /// Check if there are messages waiting for parent
    pub fn has_messages_for_parent(&self) -> bool {
        let queue = self.to_parent.lock().unwrap();
        !queue.is_empty()
    }

    /// Get count of messages waiting for subagent
    pub fn messages_for_subagent_count(&self) -> usize {
        let queue = self.to_subagent.lock().unwrap();
        queue.len()
    }

    /// Get count of messages waiting for parent
    pub fn messages_for_parent_count(&self) -> usize {
        let queue = self.to_parent.lock().unwrap();
        queue.len()
    }
}

impl Default for InternalQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_new_queue_empty() {
        let queue = InternalQueue::new();
        assert!(!queue.has_messages_for_subagent());
        assert!(!queue.has_messages_for_parent());
        assert_eq!(queue.messages_for_subagent_count(), 0);
        assert_eq!(queue.messages_for_parent_count(), 0);
    }

    #[test]
    fn test_send_to_subagent() {
        let queue = InternalQueue::new();

        queue.send_to_subagent("Hello subagent");
        assert!(queue.has_messages_for_subagent());
        assert_eq!(queue.messages_for_subagent_count(), 1);

        let messages = queue.drain_for_subagent();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello subagent");
        assert!(messages[0].to_subagent);

        assert!(!queue.has_messages_for_subagent());
    }

    #[test]
    fn test_send_to_parent() {
        let queue = InternalQueue::new();

        queue.send_to_parent("Hello parent");
        assert!(queue.has_messages_for_parent());
        assert_eq!(queue.messages_for_parent_count(), 1);

        let messages = queue.drain_for_parent();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello parent");
        assert!(!messages[0].to_subagent);

        assert!(!queue.has_messages_for_parent());
    }

    #[test]
    fn test_bidirectional() {
        let queue = InternalQueue::new();

        queue.send_to_subagent("msg1");
        queue.send_to_parent("msg2");
        queue.send_to_subagent("msg3");

        assert_eq!(queue.messages_for_subagent_count(), 2);
        assert_eq!(queue.messages_for_parent_count(), 1);

        let to_sub = queue.drain_for_subagent();
        assert_eq!(to_sub.len(), 2);
        assert_eq!(to_sub[0].content, "msg1");
        assert_eq!(to_sub[1].content, "msg3");

        let to_parent = queue.drain_for_parent();
        assert_eq!(to_parent.len(), 1);
        assert_eq!(to_parent[0].content, "msg2");
    }

    #[test]
    fn test_drain_empty() {
        let queue = InternalQueue::new();
        let messages = queue.drain_for_subagent();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_thread_safety() {
        let queue = Arc::new(InternalQueue::new());
        let mut handles = vec![];

        // Spawn threads sending to subagent
        for i in 0..5 {
            let q = queue.clone();
            handles.push(thread::spawn(move || {
                q.send_to_subagent(format!("to_sub_{}", i));
            }));
        }

        // Spawn threads sending to parent
        for i in 0..5 {
            let q = queue.clone();
            handles.push(thread::spawn(move || {
                q.send_to_parent(format!("to_parent_{}", i));
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(queue.messages_for_subagent_count(), 5);
        assert_eq!(queue.messages_for_parent_count(), 5);
    }

    #[test]
    fn test_internal_message_timestamp() {
        let msg = InternalMessage::to_subagent("test");
        assert!(msg.timestamp > 0);
        assert!(msg.to_subagent);

        let msg2 = InternalMessage::to_parent("test2");
        assert!(msg2.timestamp > 0);
        assert!(!msg2.to_subagent);
    }

    #[test]
    fn test_internal_message_serde() {
        let msg = InternalMessage::to_subagent("hello");
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: InternalMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.content, "hello");
        assert!(parsed.to_subagent);
    }
}

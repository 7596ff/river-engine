//! Thread-safe message queue for mid-generation messages
//!
//! Note: This queue uses `unwrap()` on mutex locks. Mutex poisoning only occurs
//! when a thread panics while holding the lock, which would indicate a severe
//! bug elsewhere. In that case, propagating the panic is the correct behavior.

use crate::api::IncomingMessage;
use std::collections::VecDeque;
use std::sync::Mutex;

/// Thread-safe queue for messages arriving mid-generation
pub struct MessageQueue {
    inner: Mutex<VecDeque<IncomingMessage>>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Add a message to the queue
    pub fn push(&self, msg: IncomingMessage) {
        let mut queue = self.inner.lock().unwrap();
        queue.push_back(msg);
    }

    /// Drain all messages from the queue
    pub fn drain(&self) -> Vec<IncomingMessage> {
        let mut queue = self.inner.lock().unwrap();
        queue.drain(..).collect()
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        let queue = self.inner.lock().unwrap();
        queue.is_empty()
    }

    /// Get queue length
    pub fn len(&self) -> usize {
        let queue = self.inner.lock().unwrap();
        queue.len()
    }
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Author;

    fn test_message(content: &str) -> IncomingMessage {
        IncomingMessage {
            adapter: "test".to_string(),
            event_type: "message".to_string(),
            channel: "general".to_string(),
            author: Author {
                id: "user1".to_string(),
                name: "Test User".to_string(),
            },
            content: content.to_string(),
            message_id: None,
            metadata: None,
        }
    }

    #[test]
    fn test_new_queue_is_empty() {
        let queue = MessageQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_push_and_drain() {
        let queue = MessageQueue::new();

        queue.push(test_message("hello"));
        queue.push(test_message("world"));

        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 2);

        let messages = queue.drain();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].content, "world");

        assert!(queue.is_empty());
    }

    #[test]
    fn test_drain_empty_queue() {
        let queue = MessageQueue::new();
        let messages = queue.drain();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let queue = Arc::new(MessageQueue::new());
        let mut handles = vec![];

        // Spawn threads that push messages
        for i in 0..10 {
            let q = queue.clone();
            handles.push(thread::spawn(move || {
                q.push(test_message(&format!("msg{}", i)));
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(queue.len(), 10);
    }
}

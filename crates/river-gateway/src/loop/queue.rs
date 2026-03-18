//! Thread-safe priority message queue for mid-generation messages
//!
//! Messages are ordered by priority: Interactive > Scheduled > Background.
//! Within the same priority level, messages are processed in FIFO order.
//!
//! Note: This queue uses `unwrap()` on mutex locks. Mutex poisoning only occurs
//! when a thread panics while holding the lock, which would indicate a severe
//! bug elsewhere. In that case, propagating the panic is the correct behavior.

use crate::api::IncomingMessage;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Mutex;

/// Wrapper for IncomingMessage that implements ordering by priority
///
/// Higher priority messages come first. Within same priority, earlier
/// sequence numbers (FIFO) come first.
struct PrioritizedMessage {
    message: IncomingMessage,
    sequence: u64, // For FIFO within same priority
}

impl PartialEq for PrioritizedMessage {
    fn eq(&self, other: &Self) -> bool {
        self.message.priority == other.message.priority && self.sequence == other.sequence
    }
}

impl Eq for PrioritizedMessage {}

impl PartialOrd for PrioritizedMessage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedMessage {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority first (reverse order for BinaryHeap max-heap)
        match self.message.priority.cmp(&other.message.priority) {
            Ordering::Equal => {
                // Lower sequence number first (earlier messages) - reverse for max-heap
                other.sequence.cmp(&self.sequence)
            }
            other_ord => other_ord,
        }
    }
}

/// Thread-safe priority queue for messages arriving mid-generation
///
/// Messages are ordered by priority (Interactive > Scheduled > Background).
/// Within the same priority level, messages maintain FIFO order.
pub struct MessageQueue {
    inner: Mutex<BinaryHeap<PrioritizedMessage>>,
    sequence: AtomicU64,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BinaryHeap::new()),
            sequence: AtomicU64::new(0),
        }
    }

    /// Add a message to the queue
    pub fn push(&self, msg: IncomingMessage) {
        let sequence = self.sequence.fetch_add(1, AtomicOrdering::SeqCst);
        let prioritized = PrioritizedMessage {
            message: msg,
            sequence,
        };
        let mut queue = self.inner.lock().unwrap();
        queue.push(prioritized);
    }

    /// Drain all messages from the queue, ordered by priority
    pub fn drain(&self) -> Vec<IncomingMessage> {
        let mut queue = self.inner.lock().unwrap();
        let mut messages = Vec::with_capacity(queue.len());
        while let Some(pm) = queue.pop() {
            messages.push(pm.message);
        }
        messages
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
    use river_core::Priority;

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
            priority: Priority::Interactive,
        }
    }

    fn test_message_with_priority(content: &str, priority: Priority) -> IncomingMessage {
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
            priority,
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
        // Same priority, so FIFO order
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
    fn test_priority_ordering() {
        let queue = MessageQueue::new();

        // Push in order: background, scheduled, interactive
        queue.push(test_message_with_priority("bg1", Priority::Background));
        queue.push(test_message_with_priority("sched1", Priority::Scheduled));
        queue.push(test_message_with_priority("inter1", Priority::Interactive));
        queue.push(test_message_with_priority("bg2", Priority::Background));
        queue.push(test_message_with_priority("inter2", Priority::Interactive));

        let messages = queue.drain();
        assert_eq!(messages.len(), 5);

        // Interactive first (FIFO within priority)
        assert_eq!(messages[0].content, "inter1");
        assert_eq!(messages[1].content, "inter2");
        // Then scheduled
        assert_eq!(messages[2].content, "sched1");
        // Then background (FIFO within priority)
        assert_eq!(messages[3].content, "bg1");
        assert_eq!(messages[4].content, "bg2");
    }

    #[test]
    fn test_fifo_within_same_priority() {
        let queue = MessageQueue::new();

        queue.push(test_message_with_priority("first", Priority::Interactive));
        queue.push(test_message_with_priority("second", Priority::Interactive));
        queue.push(test_message_with_priority("third", Priority::Interactive));

        let messages = queue.drain();
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].content, "second");
        assert_eq!(messages[2].content, "third");
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

    #[test]
    fn test_interactive_preempts_background() {
        let queue = MessageQueue::new();

        // Many background messages, then one interactive
        for i in 0..100 {
            queue.push(test_message_with_priority(&format!("bg{}", i), Priority::Background));
        }
        queue.push(test_message_with_priority("urgent", Priority::Interactive));

        let messages = queue.drain();
        // Interactive should be first
        assert_eq!(messages[0].content, "urgent");
        assert_eq!(messages[0].priority, Priority::Interactive);
    }
}

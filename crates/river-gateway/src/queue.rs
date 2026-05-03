//! Thread-safe notification queue
//!
//! Carries lightweight notifications (channel + snowflake ID) to wake the agent.
//! The agent reads the actual message content from the channel log.

use std::collections::VecDeque;
use std::sync::Mutex;

/// A lightweight notification that a channel has a new message
#[derive(Debug, Clone)]
pub struct ChannelNotification {
    /// Channel identifier (e.g., "discord_general")
    pub channel: String,
    /// Snowflake ID of the new message
    pub snowflake_id: String,
}

/// Thread-safe notification queue
///
/// Notifications are processed in FIFO order.
pub struct MessageQueue {
    inner: Mutex<VecDeque<ChannelNotification>>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Push a notification
    pub fn push(&self, notification: ChannelNotification) {
        let mut queue = self.inner.lock().unwrap();
        queue.push_back(notification);
    }

    /// Drain all notifications
    pub fn drain(&self) -> Vec<ChannelNotification> {
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

    #[test]
    fn test_new_queue_is_empty() {
        let queue = MessageQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_push_and_drain() {
        let queue = MessageQueue::new();

        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            snowflake_id: "001".to_string(),
        });
        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            snowflake_id: "002".to_string(),
        });

        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 2);

        let notifications = queue.drain();
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].snowflake_id, "001");
        assert_eq!(notifications[1].snowflake_id, "002");

        assert!(queue.is_empty());
    }

    #[test]
    fn test_drain_empty_queue() {
        let queue = MessageQueue::new();
        let notifications = queue.drain();
        assert!(notifications.is_empty());
    }

    #[test]
    fn test_fifo_order() {
        let queue = MessageQueue::new();

        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            snowflake_id: "first".to_string(),
        });
        queue.push(ChannelNotification {
            channel: "discord_dm".to_string(),
            snowflake_id: "second".to_string(),
        });
        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            snowflake_id: "third".to_string(),
        });

        let notifications = queue.drain();
        assert_eq!(notifications[0].snowflake_id, "first");
        assert_eq!(notifications[1].snowflake_id, "second");
        assert_eq!(notifications[2].snowflake_id, "third");
    }

    #[test]
    fn test_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let queue = Arc::new(MessageQueue::new());
        let mut handles = vec![];

        for i in 0..10 {
            let q = queue.clone();
            handles.push(thread::spawn(move || {
                q.push(ChannelNotification {
                    channel: "test".to_string(),
                    snowflake_id: format!("{}", i),
                });
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(queue.len(), 10);
    }
}

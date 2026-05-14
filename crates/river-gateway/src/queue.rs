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
    pub id: river_core::Snowflake,
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
    use river_core::{AgentBirth, Snowflake, SnowflakeType};

    fn test_snowflake() -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(0, birth, SnowflakeType::Message, 0)
    }

    fn test_snowflake_seq(seq: u32) -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(seq as u64 * 1_000_000, birth, SnowflakeType::Message, seq)
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

        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            id: test_snowflake_seq(1),
        });
        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            id: test_snowflake_seq(2),
        });

        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 2);

        let notifications = queue.drain();
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].id, test_snowflake_seq(1));
        assert_eq!(notifications[1].id, test_snowflake_seq(2));

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
            id: test_snowflake_seq(1),
        });
        queue.push(ChannelNotification {
            channel: "discord_dm".to_string(),
            id: test_snowflake_seq(2),
        });
        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            id: test_snowflake_seq(3),
        });

        let notifications = queue.drain();
        assert_eq!(notifications[0].id, test_snowflake_seq(1));
        assert_eq!(notifications[1].id, test_snowflake_seq(2));
        assert_eq!(notifications[2].id, test_snowflake_seq(3));
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
                    id: test_snowflake_seq(i as u32),
                });
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(queue.len(), 10);
    }
}

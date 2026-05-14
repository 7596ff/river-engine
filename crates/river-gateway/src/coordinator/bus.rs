//! Event bus for peer task communication

use super::events::CoordinatorEvent;
use tokio::sync::broadcast;

const DEFAULT_CAPACITY: usize = 256;

/// Event bus for coordinator communication
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<CoordinatorEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_CAPACITY);
        Self { sender }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publish an event to all subscribers
    pub fn publish(&self, event: CoordinatorEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<CoordinatorEvent> {
        self.sender.subscribe()
    }

    /// Number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::events::*;

    #[tokio::test]
    async fn test_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: "general".into(),
            turn_number: 1,
            timestamp: chrono::Utc::now(),
        }));

        let event = rx.recv().await.unwrap();
        assert!(matches!(
            event,
            CoordinatorEvent::Agent(AgentEvent::TurnStarted { .. })
        ));
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.publish(CoordinatorEvent::Shutdown);

        assert!(matches!(
            rx1.recv().await.unwrap(),
            CoordinatorEvent::Shutdown
        ));
        assert!(matches!(
            rx2.recv().await.unwrap(),
            CoordinatorEvent::Shutdown
        ));
    }

    #[test]
    fn test_subscriber_count() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);

        let _rx1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);

        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
    }
}

//! Shared state between TUI and HTTP server

use chrono::{DateTime, Local};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// A single line in the chat display
#[derive(Debug, Clone)]
pub struct ChatLine {
    pub timestamp: DateTime<Local>,
    pub sender: String,
    pub content: String,
    pub is_agent: bool,
}

/// Shared state between the TUI task and HTTP server task
#[derive(Clone)]
pub struct SharedState {
    /// Message buffer — append-only
    pub messages: Arc<Mutex<Vec<ChatLine>>>,
    /// Notify the TUI to re-render when a new message arrives
    pub notify: Arc<Notify>,
    /// Gateway connection status
    pub gateway_connected: Arc<std::sync::atomic::AtomicBool>,
    /// HTTP server status
    pub server_healthy: Arc<std::sync::atomic::AtomicBool>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            gateway_connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            server_healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    /// Push a message and notify the TUI to re-render
    pub fn push_message(&self, line: ChatLine) {
        self.messages.lock().unwrap().push(line);
        self.notify.notify_one();
    }

    /// Get a snapshot of all messages
    pub fn get_messages(&self) -> Vec<ChatLine> {
        self.messages.lock().unwrap().clone()
    }

    pub fn is_gateway_connected(&self) -> bool {
        self.gateway_connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_gateway_connected(&self, connected: bool) {
        self.gateway_connected.store(connected, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_server_healthy(&self) -> bool {
        self.server_healthy.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_server_healthy(&self, healthy: bool) {
        self.server_healthy.store(healthy, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_get_messages() {
        let state = SharedState::new();
        state.push_message(ChatLine {
            timestamp: Local::now(),
            sender: "user".into(),
            content: "hello".into(),
            is_agent: false,
        });
        let msgs = state.get_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello");
    }

    #[test]
    fn test_gateway_connected_default() {
        let state = SharedState::new();
        assert!(!state.is_gateway_connected());
    }

    #[test]
    fn test_server_healthy_default() {
        let state = SharedState::new();
        assert!(state.is_server_healthy());
    }
}

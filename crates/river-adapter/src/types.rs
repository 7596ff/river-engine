//! Core message types for adapter communication

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Incoming event from adapter to gateway
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingEvent {
    pub adapter: String,
    pub event_type: EventType,
    pub channel: String,
    pub channel_name: Option<String>,
    pub author: Author,
    pub content: String,
    pub message_id: String,
    pub timestamp: DateTime<Utc>,
    /// Native platform structure (opaque to gateway)
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventType {
    MessageCreate,
    MessageUpdate,
    MessageDelete,
    ReactionAdd,
    ReactionRemove,
    Identify(String),
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub is_bot: bool,
}

/// Outgoing message from gateway to adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRequest {
    pub channel: String,
    pub content: String,
    #[serde(default)]
    pub options: SendOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SendOptions {
    pub reply_to: Option<String>,
    pub thread_id: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResponse {
    pub success: bool,
    pub message_id: Option<String>,
    pub error: Option<String>,
}

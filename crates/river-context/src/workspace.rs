//! Workspace types for context assembly.

use river_adapter::Author;
use serde::{Deserialize, Serialize};

/// A moment summarizing a range of moves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Moment {
    pub id: String,
    pub content: String,
    /// (start_move_id, end_move_id)
    pub move_range: (String, String),
}

/// A move summarizing a range of messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Move {
    pub id: String,
    pub content: String,
    /// (start_message_id, end_message_id)
    pub message_range: (String, String),
}

/// A chat message from a channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub timestamp: String,
    pub author: Author,
    pub content: String,
}

/// A flash message between workers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Flash {
    pub id: String,
    /// Sender worker name.
    pub from: String,
    pub content: String,
    /// ISO8601 expiration time.
    pub expires_at: String,
}

/// An embedding result from search.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Embedding {
    pub id: String,
    pub content: String,
    /// Source reference (e.g., "notes/api.md:15-42").
    pub source: String,
    /// ISO8601 expiration time.
    pub expires_at: String,
}

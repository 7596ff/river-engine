//! Workspace types for context assembly.

use river_protocol::Author;
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

/// An inbox item recording a tool result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboxItem {
    /// Unique ID (filename stem).
    pub id: String,
    /// ISO8601 timestamp.
    pub timestamp: String,
    /// Tool name (read_channel, create_move, etc).
    pub tool: String,
    /// Channel adapter.
    pub channel_adapter: String,
    /// Channel ID.
    pub channel_id: String,
    /// Human-readable summary (e.g., "msg1150-msg1200").
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inbox_item_serialization() {
        let item = InboxItem {
            id: "discord_chan123_2026-04-01T07-28-00Z_read_channel".into(),
            timestamp: "2026-04-01T07:28:00Z".into(),
            tool: "read_channel".into(),
            channel_adapter: "discord".into(),
            channel_id: "chan123".into(),
            summary: "msg1150-msg1200".into(),
        };

        let json = serde_json::to_string(&item).unwrap();
        let parsed: InboxItem = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tool, "read_channel");
        assert_eq!(parsed.summary, "msg1150-msg1200");
    }
}

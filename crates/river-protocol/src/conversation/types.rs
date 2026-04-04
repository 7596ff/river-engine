//! Conversation types for message tracking.

use crate::Author;
use serde::{Deserialize, Serialize};

/// Message direction/status in a conversation file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDirection {
    /// [ ] - incoming, not yet read
    Unread,
    /// [x] - incoming, read
    Read,
    /// [>] - sent by this worker
    Outgoing,
    /// [!] - failed to send
    Failed,
}

/// A reaction on a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<String>,
    pub unknown_count: usize,
}

/// A message in the conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub direction: MessageDirection,
    pub timestamp: String,
    pub id: String,
    pub author: Author,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
}

/// A line in a conversation file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Line {
    Message(Message),
    ReadReceipt {
        timestamp: String,
        message_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_direction_serde() {
        assert_eq!(
            serde_json::to_string(&MessageDirection::Unread).unwrap(),
            r#""unread""#
        );
        assert_eq!(
            serde_json::to_string(&MessageDirection::Read).unwrap(),
            r#""read""#
        );
        assert_eq!(
            serde_json::to_string(&MessageDirection::Outgoing).unwrap(),
            r#""outgoing""#
        );
        assert_eq!(
            serde_json::to_string(&MessageDirection::Failed).unwrap(),
            r#""failed""#
        );
    }

    #[test]
    fn test_reaction_serde_roundtrip() {
        let reaction = Reaction {
            emoji: "👍".to_string(),
            users: vec!["alice".to_string(), "bob".to_string()],
            unknown_count: 2,
        };
        let json = serde_json::to_string(&reaction).unwrap();
        let parsed: Reaction = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reaction);
    }

    #[test]
    fn test_message_serde_roundtrip() {
        let msg = Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg123".to_string(),
            author: Author {
                id: "user1".to_string(),
                name: "Alice".to_string(),
                bot: false,
            },
            content: "Hello world".to_string(),
            reactions: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn test_line_message_variant() {
        let line = Line::Message(Message {
            direction: MessageDirection::Outgoing,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author {
                id: "bot".to_string(),
                name: "River".to_string(),
                bot: true,
            },
            content: "Hi!".to_string(),
            reactions: vec![],
        });
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains(r#""type":"message""#));
    }

    #[test]
    fn test_line_read_receipt_variant() {
        let line = Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg123".to_string(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains(r#""type":"read_receipt""#));
        let parsed: Line = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, line);
    }
}

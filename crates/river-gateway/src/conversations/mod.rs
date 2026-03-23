//! Bidirectional conversation tracking
//!
//! This module provides types and functionality for tracking both incoming and outgoing
//! messages in conversations, replacing the old inbox system with richer functionality.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const CONVERSATIONS_DIR: &str = "conversations";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageDirection {
    Unread,   // [ ]
    Read,     // [x]
    Outgoing, // [>]
    Failed,   // [!]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<String>,
    pub unknown_count: usize,
}

impl Reaction {
    pub fn merge(&mut self, other: &Reaction) {
        for user in &other.users {
            if !self.users.contains(user) {
                self.users.push(user.clone());
            }
        }
        let total_other = other.users.len() + other.unknown_count;
        if total_other > self.users.len() {
            self.unknown_count = total_other - self.users.len();
        }
    }

    pub fn count(&self) -> usize {
        self.users.len() + self.unknown_count
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub direction: MessageDirection,
    pub timestamp: String,
    pub id: String,
    pub author: Author,
    pub content: String,
    pub reactions: Vec<Reaction>,
}

impl Message {
    pub fn outgoing(id: &str, author: Author, content: &str) -> Self {
        Self {
            direction: MessageDirection::Outgoing,
            timestamp: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            id: id.to_string(),
            author,
            content: content.to_string(),
            reactions: vec![],
        }
    }

    pub fn failed(author: Author, error: &str, content: &str) -> Self {
        Self {
            direction: MessageDirection::Failed,
            timestamp: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            id: "-".to_string(),
            author,
            content: format!("(failed: {}) {}", error, content),
            reactions: vec![],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone)]
pub enum WriteOp {
    Message { path: PathBuf, msg: Message },
    ReactionAdd { path: PathBuf, message_id: String, emoji: String, user: String },
    ReactionRemove { path: PathBuf, message_id: String, emoji: String, user: String },
    ReactionCount { path: PathBuf, message_id: String, emoji: String, count: usize },
}

impl WriteOp {
    pub fn path(&self) -> &PathBuf {
        match self {
            WriteOp::Message { path, .. } => path,
            WriteOp::ReactionAdd { path, .. } => path,
            WriteOp::ReactionRemove { path, .. } => path,
            WriteOp::ReactionCount { path, .. } => path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_direction_equality() {
        assert_eq!(MessageDirection::Unread, MessageDirection::Unread);
        assert_eq!(MessageDirection::Read, MessageDirection::Read);
        assert_eq!(MessageDirection::Outgoing, MessageDirection::Outgoing);
        assert_eq!(MessageDirection::Failed, MessageDirection::Failed);

        assert_ne!(MessageDirection::Unread, MessageDirection::Read);
        assert_ne!(MessageDirection::Outgoing, MessageDirection::Failed);
    }

    #[test]
    fn test_reaction_count() {
        let reaction = Reaction {
            emoji: "👍".to_string(),
            users: vec!["user1".to_string(), "user2".to_string()],
            unknown_count: 3,
        };

        assert_eq!(reaction.count(), 5); // 2 users + 3 unknown
    }

    #[test]
    fn test_reaction_merge_adds_users() {
        let mut reaction1 = Reaction {
            emoji: "👍".to_string(),
            users: vec!["user1".to_string()],
            unknown_count: 0,
        };

        let reaction2 = Reaction {
            emoji: "👍".to_string(),
            users: vec!["user2".to_string(), "user3".to_string()],
            unknown_count: 2,
        };

        reaction1.merge(&reaction2);

        // Should have user1, user2, user3 in users
        assert_eq!(reaction1.users.len(), 3);
        assert!(reaction1.users.contains(&"user1".to_string()));
        assert!(reaction1.users.contains(&"user2".to_string()));
        assert!(reaction1.users.contains(&"user3".to_string()));

        // Total count from other is 4 (2 users + 2 unknown)
        // We now have 3 users, so unknown_count should be 1
        assert_eq!(reaction1.unknown_count, 1);
        assert_eq!(reaction1.count(), 4);
    }

    #[test]
    fn test_reaction_merge_no_duplicate_users() {
        let mut reaction1 = Reaction {
            emoji: "👍".to_string(),
            users: vec!["user1".to_string(), "user2".to_string()],
            unknown_count: 0,
        };

        let reaction2 = Reaction {
            emoji: "👍".to_string(),
            users: vec!["user2".to_string(), "user3".to_string()],
            unknown_count: 0,
        };

        reaction1.merge(&reaction2);

        // Should have user1, user2, user3 (no duplicate user2)
        assert_eq!(reaction1.users.len(), 3);
        assert_eq!(reaction1.count(), 3);
    }

    #[test]
    fn test_message_outgoing_constructor() {
        let author = Author {
            name: "Agent".to_string(),
            id: "agent123".to_string(),
        };

        let msg = Message::outgoing("msg123", author.clone(), "Hello, world!");

        assert_eq!(msg.direction, MessageDirection::Outgoing);
        assert_eq!(msg.id, "msg123");
        assert_eq!(msg.author.name, "Agent");
        assert_eq!(msg.author.id, "agent123");
        assert_eq!(msg.content, "Hello, world!");
        assert!(msg.reactions.is_empty());
        assert!(!msg.timestamp.is_empty());
    }

    #[test]
    fn test_message_failed_constructor() {
        let author = Author {
            name: "Agent".to_string(),
            id: "agent123".to_string(),
        };

        let msg = Message::failed(author.clone(), "network error", "Hello, world!");

        assert_eq!(msg.direction, MessageDirection::Failed);
        assert_eq!(msg.id, "-");
        assert_eq!(msg.author.name, "Agent");
        assert_eq!(msg.author.id, "agent123");
        assert_eq!(msg.content, "(failed: network error) Hello, world!");
        assert!(msg.reactions.is_empty());
        assert!(!msg.timestamp.is_empty());
    }

    #[test]
    fn test_conversation_default() {
        let convo = Conversation::default();
        assert!(convo.messages.is_empty());
    }

    #[test]
    fn test_write_op_path() {
        let path = PathBuf::from("/tmp/test.txt");

        let op1 = WriteOp::Message {
            path: path.clone(),
            msg: Message::outgoing("1", Author { name: "A".to_string(), id: "1".to_string() }, "test"),
        };
        assert_eq!(op1.path(), &path);

        let op2 = WriteOp::ReactionAdd {
            path: path.clone(),
            message_id: "msg1".to_string(),
            emoji: "👍".to_string(),
            user: "user1".to_string(),
        };
        assert_eq!(op2.path(), &path);

        let op3 = WriteOp::ReactionRemove {
            path: path.clone(),
            message_id: "msg1".to_string(),
            emoji: "👍".to_string(),
            user: "user1".to_string(),
        };
        assert_eq!(op3.path(), &path);

        let op4 = WriteOp::ReactionCount {
            path: path.clone(),
            message_id: "msg1".to_string(),
            emoji: "👍".to_string(),
            count: 5,
        };
        assert_eq!(op4.path(), &path);
    }

    #[test]
    fn test_serialization() {
        let author = Author {
            name: "Test User".to_string(),
            id: "user123".to_string(),
        };

        let msg = Message::outgoing("msg1", author, "Test message");

        // Should be able to serialize and deserialize
        let json = serde_json::to_string(&msg).expect("Failed to serialize");
        let deserialized: Message = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(msg.id, deserialized.id);
        assert_eq!(msg.content, deserialized.content);
        assert_eq!(msg.direction, deserialized.direction);
    }
}

//! Bidirectional conversation tracking
//!
//! This module provides types and functionality for tracking both incoming and outgoing
//! messages in conversations, replacing the old inbox system with richer functionality.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod format;
pub mod path;
pub mod writer;

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

    pub fn merge(&mut self, other: &Message) {
        // Content: take newer if different (edits)
        if !other.content.is_empty() && other.content != self.content {
            self.content = other.content.clone();
        }

        // Reactions: merge per-emoji
        for other_reaction in &other.reactions {
            if let Some(existing) = self.reactions.iter_mut().find(|r| r.emoji == other_reaction.emoji) {
                existing.merge(other_reaction);
            } else {
                self.reactions.push(other_reaction.clone());
            }
        }

        // Direction: only escalate Unread → Read
        if other.direction == MessageDirection::Read && self.direction == MessageDirection::Unread {
            self.direction = MessageDirection::Read;
        }
    }

    pub fn add_reaction(&mut self, emoji: &str, user: &str) {
        if let Some(r) = self.reactions.iter_mut().find(|r| r.emoji == emoji) {
            if !r.users.contains(&user.to_string()) {
                r.users.push(user.to_string());
            }
        } else {
            self.reactions.push(Reaction {
                emoji: emoji.to_string(),
                users: vec![user.to_string()],
                unknown_count: 0,
            });
        }
    }

    pub fn remove_reaction(&mut self, emoji: &str, user: &str) {
        if let Some(r) = self.reactions.iter_mut().find(|r| r.emoji == emoji) {
            r.users.retain(|u| u != user);
            if r.users.is_empty() && r.unknown_count == 0 {
                self.reactions.retain(|r| r.emoji != emoji);
            }
        }
    }

    pub fn update_reaction_count(&mut self, emoji: &str, count: usize) {
        if let Some(r) = self.reactions.iter_mut().find(|r| r.emoji == emoji) {
            if count > r.users.len() {
                r.unknown_count = count - r.users.len();
            }
        } else {
            self.reactions.push(Reaction {
                emoji: emoji.to_string(),
                users: vec![],
                unknown_count: count,
            });
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    pub messages: Vec<Message>,
}

impl Conversation {
    /// Serialize conversation to custom human-readable format
    pub fn to_string(&self) -> String {
        self.messages
            .iter()
            .map(|msg| format::format_message(msg))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Parse conversation from custom format
    pub fn from_str(s: &str) -> Result<Self, ParseError> {
        let mut messages = Vec::new();
        let mut current_message: Option<Message> = None;

        for line in s.lines() {
            if line.trim().is_empty() {
                continue;
            }

            // Check if this is a message line or a reaction line
            if line.starts_with("    ") {
                // Reaction line
                if let Some(ref mut msg) = current_message {
                    if let Some(reaction) = format::parse_reaction_line(line) {
                        msg.reactions.push(reaction);
                    } else {
                        return Err(ParseError(format!("Invalid reaction line: {}", line)));
                    }
                } else {
                    return Err(ParseError(
                        "Reaction line without preceding message".to_string(),
                    ));
                }
            } else {
                // Message line - save previous message if any
                if let Some(msg) = current_message.take() {
                    messages.push(msg);
                }

                // Parse new message
                current_message = Some(
                    format::parse_message_line(line)
                        .ok_or_else(|| ParseError(format!("Invalid message line: {}", line)))?,
                );
            }
        }

        // Don't forget the last message
        if let Some(msg) = current_message {
            messages.push(msg);
        }

        Ok(Conversation { messages })
    }

    pub fn apply_message(&mut self, msg: Message) {
        if let Some(existing) = self.messages.iter_mut().find(|m| m.id == msg.id) {
            existing.merge(&msg);
        } else {
            self.messages.push(msg);
            self.messages.sort_by(|a, b| a.id.cmp(&b.id));
        }
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Message> {
        self.messages.iter_mut().find(|m| m.id == id)
    }

    pub fn load(path: &std::path::Path) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        Self::from_str(&content).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.0)
        })
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_string())
    }
}

#[derive(Debug)]
pub struct ParseError(pub String);

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

    #[test]
    fn test_conversation_roundtrip() {
        // Create a conversation with multiple messages and reactions
        let mut conversation = Conversation::default();

        conversation.messages.push(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-03-23 14:30:00".to_string(),
            id: "msg123".to_string(),
            author: Author {
                name: "alice".to_string(),
                id: "111".to_string(),
            },
            content: "hey, can you help?".to_string(),
            reactions: vec![
                Reaction {
                    emoji: "👍".to_string(),
                    users: vec!["bob".to_string(), "charlie".to_string()],
                    unknown_count: 0,
                },
                Reaction {
                    emoji: "❤️".to_string(),
                    users: vec![],
                    unknown_count: 3,
                },
            ],
        });

        conversation.messages.push(Message {
            direction: MessageDirection::Outgoing,
            timestamp: "2026-03-23 14:30:15".to_string(),
            id: "msg124".to_string(),
            author: Author {
                name: "river".to_string(),
                id: "999".to_string(),
            },
            content: "Sure! What do you need?".to_string(),
            reactions: vec![],
        });

        conversation.messages.push(Message {
            direction: MessageDirection::Read,
            timestamp: "2026-03-23 14:30:30".to_string(),
            id: "msg125".to_string(),
            author: Author {
                name: "alice".to_string(),
                id: "111".to_string(),
            },
            content: "I'm trying to deploy...".to_string(),
            reactions: vec![Reaction {
                emoji: "🎉".to_string(),
                users: vec!["river".to_string()],
                unknown_count: 2,
            }],
        });

        conversation.messages.push(Message {
            direction: MessageDirection::Failed,
            timestamp: "2026-03-23 14:31:00".to_string(),
            id: "-".to_string(),
            author: Author {
                name: "river".to_string(),
                id: "999".to_string(),
            },
            content: "(failed: Connection timeout) Original message".to_string(),
            reactions: vec![],
        });

        // Serialize
        let serialized = conversation.to_string();

        // Parse back
        let parsed = Conversation::from_str(&serialized).expect("Failed to parse conversation");

        // Verify equality
        assert_eq!(parsed.messages.len(), conversation.messages.len());

        for (original, parsed) in conversation.messages.iter().zip(parsed.messages.iter()) {
            assert_eq!(original.direction, parsed.direction);
            assert_eq!(original.timestamp, parsed.timestamp);
            assert_eq!(original.id, parsed.id);
            assert_eq!(original.author.name, parsed.author.name);
            assert_eq!(original.author.id, parsed.author.id);
            assert_eq!(original.content, parsed.content);
            assert_eq!(original.reactions.len(), parsed.reactions.len());

            for (orig_react, parsed_react) in original.reactions.iter().zip(parsed.reactions.iter())
            {
                assert_eq!(orig_react.emoji, parsed_react.emoji);
                assert_eq!(orig_react.users, parsed_react.users);
                assert_eq!(orig_react.unknown_count, parsed_react.unknown_count);
            }
        }
    }
}

//! Conversation file handling.
//!
//! Provides types and functions for reading/writing conversation files
//! with YAML frontmatter and line-based message format.

mod format;
mod meta;
mod types;

pub use format::{
    direction_to_marker, format_line, format_message, format_reaction,
    parse_direction_marker, parse_message_line, parse_reaction_line,
    parse_read_receipt_line, FRONTMATTER_DELIMITER,
};
pub use meta::ConversationMeta;
pub use types::{Line, Message, MessageDirection, Reaction};

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Conversation parsing and handling errors.
#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("Invalid message format on line {line_number}: {reason}")]
    InvalidMessageLine { line_number: usize, reason: String },

    #[error("Invalid reaction format: {0}")]
    InvalidReactionFormat(String),

    #[error("YAML frontmatter error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    #[error("Frontmatter delimiter mismatch")]
    FrontmatterDelimiterMismatch,
}

/// A conversation file with optional metadata and lines.
#[derive(Clone, Debug, Default)]
pub struct Conversation {
    pub meta: Option<ConversationMeta>,
    pub lines: Vec<Line>,
}

impl Conversation {
    /// Load conversation from file.
    pub fn load(path: &Path) -> Result<Self, io::Error> {
        let content = fs::read_to_string(path)?;
        Self::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Save conversation to file.
    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_string())
    }

    /// Parse conversation from string.
    pub fn from_str(s: &str) -> Result<Self, ConversationError> {
        let (meta, body) = Self::split_frontmatter(s)?;

        let mut lines = Vec::new();
        let mut current_message: Option<Message> = None;

        for line in body.lines() {
            if line.trim().is_empty() {
                continue;
            }

            if line.starts_with("    ") {
                // Reaction line
                if let Some(ref mut msg) = current_message {
                    if let Some(reaction) = parse_reaction_line(line) {
                        msg.reactions.push(reaction);
                    }
                }
            } else if line.starts_with("[r] ") {
                // Read receipt - save current message first
                if let Some(msg) = current_message.take() {
                    lines.push(Line::Message(msg));
                }
                if let Some(receipt) = parse_read_receipt_line(line) {
                    lines.push(receipt);
                }
            } else {
                // Message line - save previous message if any
                if let Some(msg) = current_message.take() {
                    lines.push(Line::Message(msg));
                }
                current_message = parse_message_line(line);
            }
        }

        // Don't forget the last message
        if let Some(msg) = current_message {
            lines.push(Line::Message(msg));
        }

        Ok(Conversation { meta, lines })
    }

    /// Serialize conversation to string.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        let mut result = String::new();

        if let Some(ref meta) = self.meta {
            result.push_str(FRONTMATTER_DELIMITER);
            result.push('\n');
            result.push_str(&serde_yaml::to_string(meta).unwrap_or_default());
            result.push_str(FRONTMATTER_DELIMITER);
            result.push('\n');
        }

        let formatted: Vec<String> = self.lines.iter().map(format_line).collect();
        result.push_str(&formatted.join("\n"));
        if !self.lines.is_empty() {
            result.push('\n');
        }

        result
    }

    /// Append a line to a file (without loading full conversation).
    pub fn append_line(path: &Path, line: &Line) -> Result<(), io::Error> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        writeln!(file, "{}", format_line(line))?;
        Ok(())
    }

    /// Check if compaction is needed.
    pub fn needs_compaction(&self) -> bool {
        self.lines.len() > 100
            || self.lines.iter().any(|l| matches!(l, Line::ReadReceipt { .. }))
    }

    /// Compact: apply read receipts to messages, sort by timestamp, remove receipts, dedupe by ID.
    pub fn compact(&mut self) {
        // 1. Collect all read receipt message IDs
        let read_ids: HashSet<String> = self
            .lines
            .iter()
            .filter_map(|line| match line {
                Line::ReadReceipt { message_id, .. } => Some(message_id.clone()),
                _ => None,
            })
            .collect();

        // 2. Filter to messages, apply read status, dedupe by ID
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut messages: Vec<Message> = self
            .lines
            .iter()
            .filter_map(|line| match line {
                Line::Message(msg) => {
                    // Skip duplicates (first occurrence wins)
                    if seen_ids.contains(&msg.id) {
                        return None;
                    }
                    seen_ids.insert(msg.id.clone());

                    let mut msg = msg.clone();
                    if read_ids.contains(&msg.id) && msg.direction == MessageDirection::Unread {
                        msg.direction = MessageDirection::Read;
                    }
                    Some(msg)
                }
                _ => None,
            })
            .collect();

        // 3. Sort by timestamp
        messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        // 4. Replace lines with compacted messages
        self.lines = messages.into_iter().map(Line::Message).collect();
    }

    fn split_frontmatter(s: &str) -> Result<(Option<ConversationMeta>, &str), ConversationError> {
        let trimmed = s.trim_start();

        if !trimmed.starts_with(FRONTMATTER_DELIMITER) {
            return Ok((None, s));
        }

        let after_first = &trimmed[FRONTMATTER_DELIMITER.len()..];
        let after_first = after_first.trim_start_matches('\n');

        if let Some(end_idx) = after_first.find(&format!("\n{}", FRONTMATTER_DELIMITER)) {
            let yaml_content = &after_first[..end_idx];
            let body_start = end_idx + FRONTMATTER_DELIMITER.len() + 1;
            let body = if body_start < after_first.len() {
                after_first[body_start..].trim_start_matches('\n')
            } else {
                ""
            };

            let meta: ConversationMeta = serde_yaml::from_str(yaml_content)?;

            Ok((Some(meta), body))
        } else {
            Err(ConversationError::FrontmatterDelimiterMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Author;

    #[test]
    fn test_conversation_empty() {
        let convo = Conversation::default();
        assert!(convo.meta.is_none());
        assert!(convo.lines.is_empty());
    }

    #[test]
    fn test_conversation_roundtrip_no_frontmatter() {
        let input = "[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey\n";
        let convo = Conversation::from_str(input).unwrap();
        assert!(convo.meta.is_none());
        assert_eq!(convo.lines.len(), 1);

        let output = convo.to_string();
        let reparsed = Conversation::from_str(&output).unwrap();
        assert_eq!(reparsed.lines.len(), 1);
    }

    #[test]
    fn test_conversation_with_frontmatter() {
        let input = r#"---
adapter: discord
channel_id: "789012"
channel_name: general
---
[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey
[>] 2026-04-03 14:30:15 msg124 <river:999> Sure!
"#;
        let convo = Conversation::from_str(input).unwrap();
        assert!(convo.meta.is_some());
        let meta = convo.meta.as_ref().unwrap();
        assert_eq!(meta.adapter, "discord");
        assert_eq!(meta.channel_id, "789012");
        assert_eq!(convo.lines.len(), 2);
    }

    #[test]
    fn test_conversation_with_reactions() {
        let input = r#"[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey
    👍 bob, charlie
    ❤️ 3
"#;
        let convo = Conversation::from_str(input).unwrap();
        assert_eq!(convo.lines.len(), 1);
        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.reactions.len(), 2);
            assert_eq!(msg.reactions[0].emoji, "👍");
            assert_eq!(msg.reactions[1].unknown_count, 3);
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_conversation_with_read_receipts() {
        let input = r#"[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey
[r] 2026-04-03 14:30:05 msg123
"#;
        let convo = Conversation::from_str(input).unwrap();
        assert_eq!(convo.lines.len(), 2);
        assert!(matches!(convo.lines[0], Line::Message(_)));
        assert!(matches!(convo.lines[1], Line::ReadReceipt { .. }));
    }

    #[test]
    fn test_needs_compaction_with_receipts() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "hi".to_string(),
            reactions: vec![],
        }));
        assert!(!convo.needs_compaction());

        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg1".to_string(),
        });
        assert!(convo.needs_compaction());
    }

    #[test]
    fn test_needs_compaction_over_100_lines() {
        let mut convo = Conversation::default();
        for i in 0..101 {
            convo.lines.push(Line::Message(Message {
                direction: MessageDirection::Outgoing,
                timestamp: format!("2026-04-03 14:{:02}:00", i % 60),
                id: format!("msg{}", i),
                author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
                content: "hi".to_string(),
                reactions: vec![],
            }));
        }
        assert!(convo.needs_compaction());
    }

    #[test]
    fn test_compact_single_read_receipt() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "hi".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg1".to_string(),
        });

        convo.compact();

        assert_eq!(convo.lines.len(), 1);
        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.direction, MessageDirection::Read);
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_compact_multiple_receipts_same_message() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "hi".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg1".to_string(),
        });
        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:10".to_string(),
            message_id: "msg1".to_string(),
        });

        convo.compact();
        assert_eq!(convo.lines.len(), 1);
    }

    #[test]
    fn test_compact_receipt_for_nonexistent_message() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "hi".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "nonexistent".to_string(),
        });

        convo.compact();
        assert_eq!(convo.lines.len(), 1);
        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.direction, MessageDirection::Unread);
        }
    }

    #[test]
    fn test_compact_does_not_affect_outgoing() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Outgoing,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "hi".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg1".to_string(),
        });

        convo.compact();
        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.direction, MessageDirection::Outgoing);
        }
    }

    #[test]
    fn test_compact_does_not_affect_failed() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Failed,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "hi".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg1".to_string(),
        });

        convo.compact();
        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.direction, MessageDirection::Failed);
        }
    }

    #[test]
    fn test_compact_sorts_by_timestamp() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Outgoing,
            timestamp: "2026-04-03 14:35:00".to_string(),
            id: "msg2".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "second".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "b".to_string(), id: "2".to_string(), bot: false },
            content: "first".to_string(),
            reactions: vec![],
        }));

        convo.compact();

        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.content, "first");
        }
        if let Line::Message(msg) = &convo.lines[1] {
            assert_eq!(msg.content, "second");
        }
    }

    #[test]
    fn test_compact_preserves_reactions() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "hi".to_string(),
            reactions: vec![Reaction {
                emoji: "👍".to_string(),
                users: vec!["bob".to_string()],
                unknown_count: 0,
            }],
        }));
        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg1".to_string(),
        });

        convo.compact();

        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.reactions.len(), 1);
            assert_eq!(msg.reactions[0].emoji, "👍");
        }
    }

    #[test]
    fn test_compact_empty_conversation() {
        let mut convo = Conversation::default();
        convo.compact();
        assert!(convo.lines.is_empty());
    }

    #[test]
    fn test_compact_only_read_receipts() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg1".to_string(),
        });

        convo.compact();
        assert!(convo.lines.is_empty());
    }

    #[test]
    fn test_compact_already_compacted() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Read,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "hi".to_string(),
            reactions: vec![],
        }));

        let original_len = convo.lines.len();
        convo.compact();
        assert_eq!(convo.lines.len(), original_len);
    }

    #[test]
    fn test_unclosed_frontmatter_error() {
        let input = "---\nadapter: discord\n[ ] msg";
        let result = Conversation::from_str(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().0.contains("Unclosed frontmatter"));
    }

    #[test]
    fn test_compact_dedupes_by_message_id() {
        let mut convo = Conversation::default();
        // Add same message ID twice with different content
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "first".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:01".to_string(),
            id: "msg1".to_string(), // same ID
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "duplicate".to_string(),
            reactions: vec![],
        }));

        convo.compact();

        assert_eq!(convo.lines.len(), 1);
        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.content, "first"); // first occurrence wins
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_compact_keeps_unique_messages() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "first".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:01".to_string(),
            id: "msg2".to_string(), // different ID
            author: Author { name: "b".to_string(), id: "2".to_string(), bot: false },
            content: "second".to_string(),
            reactions: vec![],
        }));

        convo.compact();

        assert_eq!(convo.lines.len(), 2);
    }
}

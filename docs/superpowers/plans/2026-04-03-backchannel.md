# Backchannel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable workers in a dyad to communicate via a shared text file using the conversation format, with river-tui debugging support.

**Architecture:** Port conversation file handling from archive to river-protocol, wire it up in river-worker for all conversation files, add special "backchannel" channel routing to a shared file, and update river-tui to tail/write the backchannel.

**Tech Stack:** Rust, serde_yaml (frontmatter), chrono (timestamps), tokio (async file I/O)

---

## File Structure

```
river-protocol/src/
  conversation/
    mod.rs        # Conversation struct, re-exports
    types.rs      # MessageDirection, Message, Reaction, Line
    format.rs     # parse/format functions
    meta.rs       # ConversationMeta (YAML frontmatter)
  lib.rs          # Add pub mod conversation

river-worker/src/
  conversation.rs # Path helpers, compaction runner
  http.rs         # Modify: write incoming messages
  tools.rs        # Modify: speak writes outgoing, backchannel routing
  main.rs         # Modify: compaction on startup

river-tui/src/
  adapter.rs      # Modify: add backchannel state
  main.rs         # Modify: tail backchannel file
  tui.rs          # Modify: display backchannel messages
```

---

## Part 1: Conversation Module in river-protocol

### Task 1: Add serde_yaml to workspace

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/river-protocol/Cargo.toml`

- [ ] **Step 1: Add serde_yaml to workspace dependencies**

In `/home/cassie/river-engine/Cargo.toml`, add to `[workspace.dependencies]`:

```toml
serde_yaml = "0.9"
```

- [ ] **Step 2: Add dependencies to river-protocol**

In `/home/cassie/river-engine/crates/river-protocol/Cargo.toml`:

```toml
[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }
chrono = { workspace = true }
utoipa = { workspace = true }
```

- [ ] **Step 3: Verify build**

```bash
cargo check -p river-protocol
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/river-protocol/Cargo.toml
git commit -m "chore(river-protocol): add serde_yaml and chrono dependencies"
```

---

### Task 2: Create conversation types

**Files:**
- Create: `crates/river-protocol/src/conversation/types.rs`

- [ ] **Step 1: Write tests for MessageDirection**

Create `crates/river-protocol/src/conversation/types.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they pass**

```bash
cargo test -p river-protocol conversation::types
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-protocol/src/conversation/types.rs
git commit -m "feat(river-protocol): add conversation types"
```

---

### Task 3: Create ConversationMeta

**Files:**
- Create: `crates/river-protocol/src/conversation/meta.rs`

- [ ] **Step 1: Create meta.rs with tests**

Create `crates/river-protocol/src/conversation/meta.rs`:

```rust
//! Conversation metadata (YAML frontmatter).

use serde::{Deserialize, Serialize};

/// Routing metadata stored in conversation file frontmatter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub adapter: String,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_yaml_full() {
        let yaml = r#"
adapter: discord
channel_id: "789012345678901234"
channel_name: general
guild_id: "123456789012345678"
guild_name: myserver
"#;
        let meta: ConversationMeta = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.adapter, "discord");
        assert_eq!(meta.channel_id, "789012345678901234");
        assert_eq!(meta.channel_name, Some("general".to_string()));
        assert_eq!(meta.guild_id, Some("123456789012345678".to_string()));
        assert_eq!(meta.guild_name, Some("myserver".to_string()));
        assert_eq!(meta.thread_id, None);
    }

    #[test]
    fn test_meta_yaml_minimal() {
        let yaml = r#"
adapter: slack
channel_id: C12345
"#;
        let meta: ConversationMeta = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.adapter, "slack");
        assert_eq!(meta.channel_id, "C12345");
        assert_eq!(meta.channel_name, None);
        assert_eq!(meta.guild_id, None);
    }

    #[test]
    fn test_meta_yaml_roundtrip() {
        let meta = ConversationMeta {
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: None,
            guild_name: None,
            thread_id: None,
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let parsed: ConversationMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(meta, parsed);
    }

    #[test]
    fn test_meta_backchannel() {
        let meta = ConversationMeta {
            adapter: "backchannel".to_string(),
            channel_id: "dyad".to_string(),
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        assert!(yaml.contains("adapter: backchannel"));
        assert!(yaml.contains("channel_id: dyad"));
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p river-protocol conversation::meta
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-protocol/src/conversation/meta.rs
git commit -m "feat(river-protocol): add ConversationMeta for YAML frontmatter"
```

---

### Task 4: Create format parsing functions

**Files:**
- Create: `crates/river-protocol/src/conversation/format.rs`

- [ ] **Step 1: Create format.rs with direction marker functions**

Create `crates/river-protocol/src/conversation/format.rs`:

```rust
//! Conversation file format parsing and serialization.

use super::types::{Line, Message, MessageDirection, Reaction};
use crate::Author;

/// YAML frontmatter delimiter.
pub const FRONTMATTER_DELIMITER: &str = "---";

/// Parse a direction marker from a string.
pub fn parse_direction_marker(s: &str) -> Option<MessageDirection> {
    match s {
        "[ ]" => Some(MessageDirection::Unread),
        "[x]" => Some(MessageDirection::Read),
        "[>]" => Some(MessageDirection::Outgoing),
        "[!]" => Some(MessageDirection::Failed),
        _ => None,
    }
}

/// Convert a direction to its marker string.
pub fn direction_to_marker(d: MessageDirection) -> &'static str {
    match d {
        MessageDirection::Unread => "[ ]",
        MessageDirection::Read => "[x]",
        MessageDirection::Outgoing => "[>]",
        MessageDirection::Failed => "[!]",
    }
}

/// Parse a reaction line (indented with 4 spaces).
/// Formats:
/// - `    👍 bob, charlie` — usernames known
/// - `    👍 3` — count only (no usernames)
/// - `    👍 bob, charlie +1` — mixed
pub fn parse_reaction_line(line: &str) -> Option<Reaction> {
    if !line.starts_with("    ") {
        return None;
    }

    let content = line[4..].trim();
    let space_idx = content.find(' ')?;
    let emoji = content[..space_idx].to_string();
    let rest = content[space_idx + 1..].trim();

    if let Some(plus_idx) = rest.find(" +") {
        // Format: "users +N"
        let users_part = &rest[..plus_idx];
        let count_part = &rest[plus_idx + 2..];
        let users: Vec<String> = users_part.split(',').map(|s| s.trim().to_string()).collect();
        let unknown_count = count_part.parse::<usize>().ok()?;
        Some(Reaction { emoji, users, unknown_count })
    } else if rest.chars().all(|c| c.is_ascii_digit()) {
        // Format: "N" (count only)
        let unknown_count = rest.parse::<usize>().ok()?;
        Some(Reaction { emoji, users: vec![], unknown_count })
    } else {
        // Format: "users"
        let users: Vec<String> = rest.split(',').map(|s| s.trim().to_string()).collect();
        Some(Reaction { emoji, users, unknown_count: 0 })
    }
}

/// Format a reaction as a string.
pub fn format_reaction(r: &Reaction) -> String {
    if r.users.is_empty() {
        format!("    {} {}", r.emoji, r.unknown_count)
    } else if r.unknown_count > 0 {
        format!("    {} {} +{}", r.emoji, r.users.join(", "), r.unknown_count)
    } else {
        format!("    {} {}", r.emoji, r.users.join(", "))
    }
}

/// Parse a message line.
/// Format: `[marker] timestamp id <author_name:author_id> content`
pub fn parse_message_line(line: &str) -> Option<Message> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let (direction, rest) = if line.starts_with("[!] ") {
        (MessageDirection::Failed, &line[4..])
    } else if line.starts_with("[>] ") {
        (MessageDirection::Outgoing, &line[4..])
    } else if line.starts_with("[x] ") {
        (MessageDirection::Read, &line[4..])
    } else if line.starts_with("[ ] ") {
        (MessageDirection::Unread, &line[4..])
    } else {
        return None;
    };

    // Split: date time id <author> content
    let mut parts = rest.splitn(4, ' ');
    let date = parts.next()?;
    let time = parts.next()?;
    let timestamp = format!("{} {}", date, time);
    let id = parts.next()?.to_string();
    let remainder = parts.next()?;

    if !remainder.starts_with('<') {
        return None;
    }

    let author_end = remainder.find('>')?;
    let author_part = &remainder[1..author_end];
    let content_start = author_end + 2;

    let (author_name, author_id) = author_part.rsplit_once(':')?;

    let content = if content_start < remainder.len() {
        remainder[content_start..].to_string()
    } else {
        String::new()
    };

    Some(Message {
        direction,
        timestamp,
        id,
        author: Author {
            name: author_name.to_string(),
            id: author_id.to_string(),
            bot: false, // Default, not stored in file format
        },
        content,
        reactions: vec![],
    })
}

/// Parse a read receipt line.
/// Format: `[r] timestamp message_id`
pub fn parse_read_receipt_line(line: &str) -> Option<Line> {
    let line = line.trim();
    if !line.starts_with("[r] ") {
        return None;
    }

    let rest = &line[4..];
    let mut parts = rest.splitn(3, ' ');
    let date = parts.next()?;
    let time = parts.next()?;
    let timestamp = format!("{} {}", date, time);
    let message_id = parts.next()?.to_string();

    Some(Line::ReadReceipt { timestamp, message_id })
}

/// Format a message as a string (including reactions).
pub fn format_message(msg: &Message) -> String {
    let marker = direction_to_marker(msg.direction);
    let mut result = format!(
        "{} {} {} <{}:{}> {}",
        marker, msg.timestamp, msg.id, msg.author.name, msg.author.id, msg.content
    );

    for reaction in &msg.reactions {
        result.push('\n');
        result.push_str(&format_reaction(reaction));
    }

    result
}

/// Format a line (message or read receipt).
pub fn format_line(line: &Line) -> String {
    match line {
        Line::Message(msg) => format_message(msg),
        Line::ReadReceipt { timestamp, message_id } => {
            format!("[r] {} {}", timestamp, message_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_direction_marker() {
        assert_eq!(parse_direction_marker("[ ]"), Some(MessageDirection::Unread));
        assert_eq!(parse_direction_marker("[x]"), Some(MessageDirection::Read));
        assert_eq!(parse_direction_marker("[>]"), Some(MessageDirection::Outgoing));
        assert_eq!(parse_direction_marker("[!]"), Some(MessageDirection::Failed));
        assert_eq!(parse_direction_marker("???"), None);
    }

    #[test]
    fn test_direction_to_marker() {
        assert_eq!(direction_to_marker(MessageDirection::Unread), "[ ]");
        assert_eq!(direction_to_marker(MessageDirection::Read), "[x]");
        assert_eq!(direction_to_marker(MessageDirection::Outgoing), "[>]");
        assert_eq!(direction_to_marker(MessageDirection::Failed), "[!]");
    }

    #[test]
    fn test_parse_reaction_known_users() {
        let r = parse_reaction_line("    👍 bob, charlie").unwrap();
        assert_eq!(r.emoji, "👍");
        assert_eq!(r.users, vec!["bob", "charlie"]);
        assert_eq!(r.unknown_count, 0);
    }

    #[test]
    fn test_parse_reaction_count_only() {
        let r = parse_reaction_line("    ❤️ 3").unwrap();
        assert_eq!(r.emoji, "❤️");
        assert!(r.users.is_empty());
        assert_eq!(r.unknown_count, 3);
    }

    #[test]
    fn test_parse_reaction_mixed() {
        let r = parse_reaction_line("    🎉 river +2").unwrap();
        assert_eq!(r.emoji, "🎉");
        assert_eq!(r.users, vec!["river"]);
        assert_eq!(r.unknown_count, 2);
    }

    #[test]
    fn test_parse_reaction_not_indented() {
        assert!(parse_reaction_line("👍 bob").is_none());
        assert!(parse_reaction_line("  👍 bob").is_none());
    }

    #[test]
    fn test_format_reaction_roundtrip() {
        let reactions = vec![
            Reaction { emoji: "👍".to_string(), users: vec!["bob".to_string()], unknown_count: 0 },
            Reaction { emoji: "❤️".to_string(), users: vec![], unknown_count: 3 },
            Reaction { emoji: "🎉".to_string(), users: vec!["river".to_string()], unknown_count: 2 },
        ];
        for r in reactions {
            let formatted = format_reaction(&r);
            let parsed = parse_reaction_line(&formatted).unwrap();
            assert_eq!(parsed, r);
        }
    }

    #[test]
    fn test_parse_message_line_unread() {
        let line = "[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey, can you help?";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.direction, MessageDirection::Unread);
        assert_eq!(msg.timestamp, "2026-04-03 14:30:00");
        assert_eq!(msg.id, "msg123");
        assert_eq!(msg.author.name, "alice");
        assert_eq!(msg.author.id, "111");
        assert_eq!(msg.content, "hey, can you help?");
    }

    #[test]
    fn test_parse_message_line_outgoing() {
        let line = "[>] 2026-04-03 14:30:15 msg124 <river:999> Sure! What do you need?";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.direction, MessageDirection::Outgoing);
        assert_eq!(msg.author.name, "river");
    }

    #[test]
    fn test_parse_message_line_failed() {
        let line = "[!] 2026-04-03 14:31:00 - <river:999> (failed: error) Original message";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.direction, MessageDirection::Failed);
        assert_eq!(msg.id, "-");
    }

    #[test]
    fn test_parse_read_receipt_line() {
        let line = "[r] 2026-04-03 14:30:05 msg123";
        let receipt = parse_read_receipt_line(line).unwrap();
        match receipt {
            Line::ReadReceipt { timestamp, message_id } => {
                assert_eq!(timestamp, "2026-04-03 14:30:05");
                assert_eq!(message_id, "msg123");
            }
            _ => panic!("Expected ReadReceipt"),
        }
    }

    #[test]
    fn test_format_message_simple() {
        let msg = Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg123".to_string(),
            author: Author { name: "alice".to_string(), id: "111".to_string(), bot: false },
            content: "hey".to_string(),
            reactions: vec![],
        };
        let formatted = format_message(&msg);
        assert_eq!(formatted, "[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey");
    }

    #[test]
    fn test_format_message_with_reactions() {
        let msg = Message {
            direction: MessageDirection::Read,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg123".to_string(),
            author: Author { name: "alice".to_string(), id: "111".to_string(), bot: false },
            content: "hey".to_string(),
            reactions: vec![
                Reaction { emoji: "👍".to_string(), users: vec!["bob".to_string()], unknown_count: 0 },
            ],
        };
        let formatted = format_message(&msg);
        assert!(formatted.contains("[x] 2026-04-03 14:30:00 msg123 <alice:111> hey"));
        assert!(formatted.contains("\n    👍 bob"));
    }

    #[test]
    fn test_format_line_read_receipt() {
        let line = Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg123".to_string(),
        };
        assert_eq!(format_line(&line), "[r] 2026-04-03 14:30:05 msg123");
    }

    #[test]
    fn test_message_roundtrip() {
        let original = Message {
            direction: MessageDirection::Outgoing,
            timestamp: "2026-04-03 14:30:15".to_string(),
            id: "msg124".to_string(),
            author: Author { name: "river".to_string(), id: "999".to_string(), bot: true },
            content: "Sure! What do you need?".to_string(),
            reactions: vec![],
        };
        let formatted = format_message(&original);
        let parsed = parse_message_line(&formatted).unwrap();
        assert_eq!(parsed.direction, original.direction);
        assert_eq!(parsed.timestamp, original.timestamp);
        assert_eq!(parsed.id, original.id);
        assert_eq!(parsed.author.name, original.author.name);
        assert_eq!(parsed.author.id, original.author.id);
        assert_eq!(parsed.content, original.content);
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p river-protocol conversation::format
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-protocol/src/conversation/format.rs
git commit -m "feat(river-protocol): add conversation format parsing"
```

---

### Task 5: Create Conversation struct with compaction

**Files:**
- Create: `crates/river-protocol/src/conversation/mod.rs`
- Modify: `crates/river-protocol/src/lib.rs`

- [ ] **Step 1: Create mod.rs with Conversation struct**

Create `crates/river-protocol/src/conversation/mod.rs`:

```rust
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

/// Parse error for conversation files.
#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

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
        Self::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.0))
    }

    /// Save conversation to file.
    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_string())
    }

    /// Parse conversation from string.
    pub fn from_str(s: &str) -> Result<Self, ParseError> {
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

    /// Compact: apply read receipts to messages, sort by timestamp, remove receipts.
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

        // 2. Filter to messages, apply read status
        let mut messages: Vec<Message> = self
            .lines
            .iter()
            .filter_map(|line| match line {
                Line::Message(msg) => {
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

    fn split_frontmatter(s: &str) -> Result<(Option<ConversationMeta>, &str), ParseError> {
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

            let meta: ConversationMeta = serde_yaml::from_str(yaml_content)
                .map_err(|e| ParseError(format!("Invalid frontmatter YAML: {}", e)))?;

            Ok((Some(meta), body))
        } else {
            Err(ParseError(
                "Unclosed frontmatter (missing closing ---)".to_string(),
            ))
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
}
```

- [ ] **Step 2: Add conversation module to lib.rs**

In `/home/cassie/river-engine/crates/river-protocol/src/lib.rs`, add after the existing modules:

```rust
pub mod conversation;
```

- [ ] **Step 3: Run all tests**

```bash
cargo test -p river-protocol
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-protocol/src/conversation/ crates/river-protocol/src/lib.rs
git commit -m "feat(river-protocol): add Conversation struct with compaction"
```

---

## Part 2: Wire Up Conversation Management in river-worker

### Task 6: Add conversation path helpers to river-worker

**Files:**
- Create: `crates/river-worker/src/conversation.rs`
- Modify: `crates/river-worker/src/main.rs`
- Modify: `crates/river-worker/Cargo.toml`

- [ ] **Step 1: Add walkdir dependency**

In `/home/cassie/river-engine/Cargo.toml` workspace dependencies:

```toml
walkdir = "2"
```

In `/home/cassie/river-engine/crates/river-worker/Cargo.toml`:

```toml
walkdir = { workspace = true }
```

- [ ] **Step 2: Create conversation.rs**

Create `/home/cassie/river-engine/crates/river-worker/src/conversation.rs`:

```rust
//! Conversation file management for the worker.

use river_protocol::conversation::Conversation;
use river_protocol::Channel;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Sanitize a string for use in file paths.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Get the conversation file path for a channel.
pub fn conversation_path_for_channel(workspace: &Path, channel: &Channel) -> PathBuf {
    // For now, DMs and channels without guild go to dm/
    // Guild channels go to adapter/guild_id-guild_name/channel_id-channel_name.txt
    let channel_name = channel.name.as_deref().unwrap_or("unknown");

    workspace
        .join("conversations")
        .join(&channel.adapter)
        .join("dm")
        .join(format!("{}-{}.txt", channel.id, sanitize(channel_name)))
}

/// Get the backchannel file path.
pub fn backchannel_path(workspace: &Path) -> PathBuf {
    workspace.join("conversations").join("backchannel.txt")
}

/// Compact all conversation files in the workspace that need it.
pub fn compact_conversations(workspace: &Path) -> std::io::Result<()> {
    let conversations_dir = workspace.join("conversations");
    if !conversations_dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(&conversations_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "txt") {
            if let Ok(mut convo) = Conversation::load(path) {
                if convo.needs_compaction() {
                    tracing::info!(path = %path.display(), "Compacting conversation file");
                    convo.compact();
                    convo.save(path)?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("hello world"), "hello_world");
        assert_eq!(sanitize("general-chat"), "general-chat");
        assert_eq!(sanitize("my_channel"), "my_channel");
        assert_eq!(sanitize("test/path"), "test_path");
    }

    #[test]
    fn test_conversation_path_for_channel() {
        let workspace = Path::new("/workspace");
        let channel = Channel {
            adapter: "discord".to_string(),
            id: "123456".to_string(),
            name: Some("general".to_string()),
        };

        let path = conversation_path_for_channel(workspace, &channel);
        assert!(path.to_str().unwrap().contains("discord"));
        assert!(path.to_str().unwrap().contains("123456-general.txt"));
    }

    #[test]
    fn test_backchannel_path() {
        let workspace = Path::new("/workspace");
        let path = backchannel_path(workspace);
        assert_eq!(
            path,
            PathBuf::from("/workspace/conversations/backchannel.txt")
        );
    }
}
```

- [ ] **Step 3: Add module to main.rs**

In `/home/cassie/river-engine/crates/river-worker/src/main.rs`, add:

```rust
mod conversation;
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p river-worker conversation
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/river-worker/Cargo.toml crates/river-worker/src/conversation.rs crates/river-worker/src/main.rs
git commit -m "feat(river-worker): add conversation path helpers and compaction"
```

---

### Task 7: Add backchannel routing to speak tool

**Files:**
- Modify: `crates/river-worker/src/tools.rs`

- [ ] **Step 1: Read current tools.rs to find speak implementation**

Read the speak tool implementation to understand where to add backchannel routing.

- [ ] **Step 2: Add backchannel handling to speak tool**

In the speak tool handler (around line where channel is resolved), add before the adapter call:

```rust
// Check for backchannel special routing
if channel_id == "backchannel" || channel_name.as_deref() == Some("backchannel") {
    use river_protocol::conversation::{Conversation, Line, Message, MessageDirection};
    use crate::conversation::backchannel_path;

    let id = format!("{}", state.snowflake_generator.next_id()?);
    let msg = Message {
        direction: MessageDirection::Outgoing,
        timestamp: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        id: id.clone(),
        author: river_protocol::Author {
            name: state.dyad.clone(),
            id: state.baton.to_string(),
            bot: true,
        },
        content: content.clone(),
        reactions: vec![],
    };

    let path = backchannel_path(&state.workspace);
    Conversation::append_line(&path, &Line::Message(msg))?;

    return Ok(ToolResult::success(serde_json::json!({
        "message_id": id,
        "sent": true,
        "channel": "backchannel"
    })));
}
```

- [ ] **Step 3: Verify build**

```bash
cargo check -p river-worker
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-worker/src/tools.rs
git commit -m "feat(river-worker): route speak to backchannel file when channel is 'backchannel'"
```

---

### Task 8: Run compaction on worker startup

**Files:**
- Modify: `crates/river-worker/src/main.rs`

- [ ] **Step 1: Add compaction call in startup**

In the worker's main function, after workspace is set up but before the worker loop starts, add:

```rust
// Compact conversation files on startup
if let Err(e) = crate::conversation::compact_conversations(&workspace) {
    tracing::warn!(error = %e, "Failed to compact conversation files");
}
```

- [ ] **Step 2: Verify build**

```bash
cargo check -p river-worker
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-worker/src/main.rs
git commit -m "feat(river-worker): run conversation compaction on startup"
```

---

## Part 3: River-TUI Backchannel Integration

### Task 9: Add backchannel state to river-tui

**Files:**
- Modify: `crates/river-tui/src/adapter.rs`
- Modify: `crates/river-tui/Cargo.toml`

- [ ] **Step 1: Add river-protocol dependency to river-tui**

In `/home/cassie/river-engine/crates/river-tui/Cargo.toml`, add:

```toml
river-protocol = { path = "../river-protocol" }
```

- [ ] **Step 2: Add backchannel lines to AdapterState**

In `/home/cassie/river-engine/crates/river-tui/src/adapter.rs`, add field to `AdapterState`:

```rust
use river_protocol::conversation::Line as BackchannelLine;

// In AdapterState struct:
pub backchannel_lines: Vec<BackchannelLine>,
```

In the `new()` constructor, initialize:

```rust
backchannel_lines: Vec::new(),
```

Add method:

```rust
pub fn add_backchannel_line(&mut self, line: BackchannelLine) {
    self.backchannel_lines.push(line);
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p river-tui
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-tui/Cargo.toml crates/river-tui/src/adapter.rs
git commit -m "feat(river-tui): add backchannel state to AdapterState"
```

---

### Task 10: Add backchannel file tailing

**Files:**
- Modify: `crates/river-tui/src/main.rs`

- [ ] **Step 1: Add tail_backchannel function**

In `/home/cassie/river-engine/crates/river-tui/src/main.rs`, add function:

```rust
use river_protocol::conversation::Conversation;

async fn tail_backchannel(
    workspace: PathBuf,
    state: SharedState,
) {
    let path = workspace.join("conversations").join("backchannel.txt");
    let mut last_line_count = 0;

    loop {
        if path.exists() {
            if let Ok(convo) = Conversation::load(&path) {
                if convo.lines.len() > last_line_count {
                    let new_lines = &convo.lines[last_line_count..];
                    let mut s = state.write().await;
                    for line in new_lines {
                        s.add_backchannel_line(line.clone());
                    }
                    last_line_count = convo.lines.len();
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

- [ ] **Step 2: Spawn backchannel tailing task**

In main(), where other tailing tasks are spawned, add:

```rust
let backchannel_state = state.clone();
let backchannel_workspace = workspace.clone();
tokio::spawn(async move {
    tail_backchannel(backchannel_workspace, backchannel_state).await;
});
```

- [ ] **Step 3: Verify build**

```bash
cargo check -p river-tui
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-tui/src/main.rs
git commit -m "feat(river-tui): tail backchannel file for new messages"
```

---

### Task 11: Display backchannel messages in TUI

**Files:**
- Modify: `crates/river-tui/src/tui.rs`

- [ ] **Step 1: Add backchannel formatting function**

In `/home/cassie/river-engine/crates/river-tui/src/tui.rs`, add:

```rust
use river_protocol::conversation::{Line as BackchannelLine, Message as BackchannelMessage};
use ratatui::style::Color;

fn format_backchannel_line(line: &BackchannelLine) -> Vec<Line<'static>> {
    match line {
        BackchannelLine::Message(msg) => {
            let prefix = format!("[BC {}] ", msg.author.id);
            let style = Style::default().fg(Color::Cyan);
            vec![Line::from(Span::styled(
                format!("{}{}", prefix, msg.content),
                style,
            ))]
        }
        BackchannelLine::ReadReceipt { message_id, .. } => {
            vec![Line::from(Span::styled(
                format!("[BC read] {}", message_id),
                Style::default().fg(Color::DarkGray),
            ))]
        }
    }
}
```

- [ ] **Step 2: Add backchannel to conversation display**

In the `draw_conversation` function, after rendering context messages, add backchannel messages:

```rust
// Add backchannel messages (in cyan)
for line in &state.backchannel_lines {
    items.extend(format_backchannel_line(line));
}
```

- [ ] **Step 3: Verify build**

```bash
cargo check -p river-tui
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-tui/src/tui.rs
git commit -m "feat(river-tui): display backchannel messages in cyan"
```

---

### Task 12: Allow writing to backchannel from TUI

**Files:**
- Modify: `crates/river-tui/src/tui.rs`

- [ ] **Step 1: Add backchannel write handling**

In the input handling section where messages are sent, add a check for backchannel prefix:

```rust
// Check if message starts with "/bc " for backchannel
if input.starts_with("/bc ") {
    let content = input.strip_prefix("/bc ").unwrap().to_string();
    let msg = river_protocol::conversation::Message {
        direction: river_protocol::conversation::MessageDirection::Outgoing,
        timestamp: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        id: format!("tui-{}", chrono::Utc::now().timestamp_millis()),
        author: river_protocol::Author {
            name: "tui".to_string(),
            id: "debug".to_string(),
            bot: false,
        },
        content,
        reactions: vec![],
    };

    let path = workspace.join("conversations").join("backchannel.txt");
    if let Err(e) = river_protocol::conversation::Conversation::append_line(
        &path,
        &river_protocol::conversation::Line::Message(msg),
    ) {
        let mut s = state.write().await;
        s.add_system_message(&format!("Backchannel write failed: {}", e));
    }
    return;
}
```

- [ ] **Step 2: Update help text**

Update the input help text to mention `/bc`:

```rust
.title(" Type message (Enter=send, /bc=backchannel, Up/Down/PgUp/PgDn=scroll, Ctrl+C=quit) ")
```

- [ ] **Step 3: Verify build**

```bash
cargo check -p river-tui
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-tui/src/tui.rs
git commit -m "feat(river-tui): allow writing to backchannel with /bc prefix"
```

---

## Part 4: Final Verification

### Task 13: Run all tests and verify

**Files:** None (verification only)

- [ ] **Step 1: Run all tests**

```bash
cargo test --workspace
```

- [ ] **Step 2: Build all crates**

```bash
cargo build --workspace
```

- [ ] **Step 3: Commit any fixes**

If any issues found, fix and commit.

---

## Summary

| Task | Description | Key Files |
|------|-------------|-----------|
| 1 | Add serde_yaml dependency | Cargo.toml |
| 2 | Create conversation types | river-protocol/conversation/types.rs |
| 3 | Create ConversationMeta | river-protocol/conversation/meta.rs |
| 4 | Create format parsing | river-protocol/conversation/format.rs |
| 5 | Create Conversation with compaction | river-protocol/conversation/mod.rs |
| 6 | Add path helpers to worker | river-worker/conversation.rs |
| 7 | Add backchannel to speak tool | river-worker/tools.rs |
| 8 | Run compaction on startup | river-worker/main.rs |
| 9 | Add backchannel state to TUI | river-tui/adapter.rs |
| 10 | Tail backchannel file | river-tui/main.rs |
| 11 | Display backchannel messages | river-tui/tui.rs |
| 12 | Write to backchannel from TUI | river-tui/tui.rs |
| 13 | Final verification | - |

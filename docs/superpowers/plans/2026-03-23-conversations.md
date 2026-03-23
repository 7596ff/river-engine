# Bidirectional Conversations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform the one-directional inbox system into bidirectional conversations that capture both incoming and outgoing messages with reaction support.

**Architecture:** Create a new `conversations` module with custom serialization for a human-readable file format. A `ConversationWriter` task serializes all writes. Extend `SendMessageTool` to record outgoing messages. Add Discord `/read` endpoint and `sync_conversation` tool for history fetching.

**Tech Stack:** Rust, tokio (mpsc channels), serde, axum (Discord HTTP), chrono

**Spec:** `docs/superpowers/specs/2026-03-23-conversations-design.md`

---

## File Structure

| File | Purpose |
|------|---------|
| `crates/river-gateway/src/conversations/mod.rs` | Core types: `Message`, `Reaction`, `Conversation`, `WriteOp`, `MessageDirection` |
| `crates/river-gateway/src/conversations/format.rs` | Custom `to_string()` and `from_str()` for human-readable format |
| `crates/river-gateway/src/conversations/path.rs` | Path building helpers (reuse pattern from `inbox/writer.rs`) |
| `crates/river-gateway/src/conversations/writer.rs` | `ConversationWriter` single-writer task |
| `crates/river-gateway/src/tools/sync.rs` | `SyncConversationTool` |
| `crates/river-discord/src/outbound.rs` | Add `/read` endpoint |
| `crates/river-gateway/src/tools/communication.rs` | Modify `SendMessageTool` to record outgoing |
| `crates/river-gateway/src/server.rs` | Spawn writer, migration, wire up |
| `crates/river-gateway/src/lib.rs` | Add `pub mod conversations;` |

---

### Task 1: Core Types

**Files:**
- Create: `crates/river-gateway/src/conversations/mod.rs`
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Write test for MessageDirection**

```rust
// crates/river-gateway/src/conversations/mod.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_direction_equality() {
        assert_eq!(MessageDirection::Unread, MessageDirection::Unread);
        assert_ne!(MessageDirection::Unread, MessageDirection::Read);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-gateway conversations::tests::test_message_direction_equality`
Expected: FAIL - module doesn't exist

- [ ] **Step 3: Create module with MessageDirection**

```rust
// crates/river-gateway/src/conversations/mod.rs
//! Bidirectional conversation storage
//!
//! Conversations capture both incoming and outgoing messages in human-readable files.

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
```

- [ ] **Step 4: Add module to lib.rs**

```rust
// crates/river-gateway/src/lib.rs
pub mod conversations;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p river-gateway conversations::tests::test_message_direction_equality`
Expected: PASS

- [ ] **Step 6: Add Author and Reaction types with tests**

```rust
#[test]
fn test_reaction_count() {
    let r = Reaction {
        emoji: "👍".into(),
        users: vec!["bob".into(), "charlie".into()],
        unknown_count: 1,
    };
    assert_eq!(r.count(), 3);
}

#[test]
fn test_reaction_merge_adds_users() {
    let mut r1 = Reaction {
        emoji: "👍".into(),
        users: vec!["bob".into()],
        unknown_count: 0,
    };
    let r2 = Reaction {
        emoji: "👍".into(),
        users: vec!["charlie".into()],
        unknown_count: 0,
    };
    r1.merge(&r2);
    assert_eq!(r1.users, vec!["bob", "charlie"]);
}
```

- [ ] **Step 7: Implement Author and Reaction**

```rust
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
```

- [ ] **Step 8: Run tests**

Run: `cargo test -p river-gateway conversations::tests`
Expected: PASS

- [ ] **Step 9: Add Message and Conversation types**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub direction: MessageDirection,
    pub timestamp: String,
    pub id: String,
    pub author: Author,
    pub content: String,
    pub reactions: Vec<Reaction>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    pub messages: Vec<Message>,
}
```

- [ ] **Step 10: Add Message constructors with tests**

```rust
#[test]
fn test_message_outgoing_constructor() {
    let msg = Message::outgoing("123", Author { name: "river".into(), id: "999".into() }, "Hello!");
    assert_eq!(msg.direction, MessageDirection::Outgoing);
    assert_eq!(msg.id, "123");
    assert!(msg.reactions.is_empty());
}

#[test]
fn test_message_failed_constructor() {
    let msg = Message::failed(Author { name: "river".into(), id: "999".into() }, "timeout", "Hello!");
    assert_eq!(msg.direction, MessageDirection::Failed);
    assert_eq!(msg.id, "-");
    assert!(msg.content.contains("failed: timeout"));
    assert!(msg.content.contains("Hello!"));
}
```

- [ ] **Step 11: Implement Message constructors**

```rust
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
```

- [ ] **Step 12: Add WriteOp enum**

```rust
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
```

- [ ] **Step 13: Run all tests and commit**

Run: `cargo test -p river-gateway conversations`
Expected: PASS

```bash
git add crates/river-gateway/src/conversations/mod.rs crates/river-gateway/src/lib.rs
git commit -m "feat(conversations): add core types

MessageDirection, Author, Reaction, Message, Conversation, WriteOp"
```

---

### Task 2: Custom Serialization Format

**Files:**
- Create: `crates/river-gateway/src/conversations/format.rs`
- Modify: `crates/river-gateway/src/conversations/mod.rs`

- [ ] **Step 1: Write test for direction marker parsing**

```rust
// crates/river-gateway/src/conversations/format.rs
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
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-gateway conversations::format::tests::test_parse_direction_marker`
Expected: FAIL

- [ ] **Step 3: Implement parse_direction_marker**

```rust
// crates/river-gateway/src/conversations/format.rs
//! Custom serialization for human-readable conversation format

use super::{Author, Conversation, Message, MessageDirection, Reaction};

fn parse_direction_marker(s: &str) -> Option<MessageDirection> {
    match s {
        "[ ]" => Some(MessageDirection::Unread),
        "[x]" => Some(MessageDirection::Read),
        "[>]" => Some(MessageDirection::Outgoing),
        "[!]" => Some(MessageDirection::Failed),
        _ => None,
    }
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p river-gateway conversations::format::tests::test_parse_direction_marker`
Expected: PASS

- [ ] **Step 5: Add test for direction to marker**

```rust
#[test]
fn test_direction_to_marker() {
    assert_eq!(direction_to_marker(MessageDirection::Unread), "[ ]");
    assert_eq!(direction_to_marker(MessageDirection::Read), "[x]");
    assert_eq!(direction_to_marker(MessageDirection::Outgoing), "[>]");
    assert_eq!(direction_to_marker(MessageDirection::Failed), "[!]");
}
```

- [ ] **Step 6: Implement direction_to_marker**

```rust
fn direction_to_marker(d: MessageDirection) -> &'static str {
    match d {
        MessageDirection::Unread => "[ ]",
        MessageDirection::Read => "[x]",
        MessageDirection::Outgoing => "[>]",
        MessageDirection::Failed => "[!]",
    }
}
```

- [ ] **Step 7: Add test for reaction parsing**

```rust
#[test]
fn test_parse_reaction_line() {
    // Known users
    let r = parse_reaction_line("    👍 bob, charlie").unwrap();
    assert_eq!(r.emoji, "👍");
    assert_eq!(r.users, vec!["bob", "charlie"]);
    assert_eq!(r.unknown_count, 0);

    // Count only
    let r = parse_reaction_line("    ❤️ 3").unwrap();
    assert_eq!(r.emoji, "❤️");
    assert!(r.users.is_empty());
    assert_eq!(r.unknown_count, 3);

    // Mixed
    let r = parse_reaction_line("    🎉 river +2").unwrap();
    assert_eq!(r.emoji, "🎉");
    assert_eq!(r.users, vec!["river"]);
    assert_eq!(r.unknown_count, 2);
}
```

- [ ] **Step 8: Implement parse_reaction_line**

```rust
fn parse_reaction_line(line: &str) -> Option<Reaction> {
    let line = line.trim_start();
    if line.is_empty() {
        return None;
    }

    // Split emoji from rest
    let mut chars = line.chars();
    let emoji: String = chars.by_ref().take_while(|c| !c.is_whitespace()).collect();
    let rest: String = chars.collect();
    let rest = rest.trim();

    if rest.is_empty() {
        return None;
    }

    // Check for +N suffix (mixed format)
    if let Some(plus_pos) = rest.rfind(" +") {
        let users_part = &rest[..plus_pos];
        let count_str = &rest[plus_pos + 2..];
        if let Ok(unknown) = count_str.parse::<usize>() {
            let users: Vec<String> = users_part
                .split(", ")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return Some(Reaction { emoji, users, unknown_count: unknown });
        }
    }

    // Check for count only (single number)
    if let Ok(count) = rest.parse::<usize>() {
        return Some(Reaction { emoji, users: vec![], unknown_count: count });
    }

    // Users only
    let users: Vec<String> = rest
        .split(", ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Some(Reaction { emoji, users, unknown_count: 0 })
}
```

- [ ] **Step 9: Add test for reaction formatting**

```rust
#[test]
fn test_format_reaction() {
    // Users only
    let r = Reaction { emoji: "👍".into(), users: vec!["bob".into(), "charlie".into()], unknown_count: 0 };
    assert_eq!(format_reaction(&r), "    👍 bob, charlie");

    // Count only
    let r = Reaction { emoji: "❤️".into(), users: vec![], unknown_count: 3 };
    assert_eq!(format_reaction(&r), "    ❤️ 3");

    // Mixed
    let r = Reaction { emoji: "🎉".into(), users: vec!["river".into()], unknown_count: 2 };
    assert_eq!(format_reaction(&r), "    🎉 river +2");
}
```

- [ ] **Step 10: Implement format_reaction**

```rust
fn format_reaction(r: &Reaction) -> String {
    if r.users.is_empty() {
        format!("    {} {}", r.emoji, r.unknown_count)
    } else if r.unknown_count == 0 {
        format!("    {} {}", r.emoji, r.users.join(", "))
    } else {
        format!("    {} {} +{}", r.emoji, r.users.join(", "), r.unknown_count)
    }
}
```

- [ ] **Step 11: Add test for message parsing**

```rust
#[test]
fn test_parse_message_line() {
    let msg = parse_message_line("[ ] 2026-03-23 14:30:00 msg123 <alice:111> hello there").unwrap();
    assert_eq!(msg.direction, MessageDirection::Unread);
    assert_eq!(msg.timestamp, "2026-03-23 14:30:00");
    assert_eq!(msg.id, "msg123");
    assert_eq!(msg.author.name, "alice");
    assert_eq!(msg.author.id, "111");
    assert_eq!(msg.content, "hello there");
}

#[test]
fn test_parse_message_line_outgoing() {
    let msg = parse_message_line("[>] 2026-03-23 14:30:15 msg124 <river:999> Sure!").unwrap();
    assert_eq!(msg.direction, MessageDirection::Outgoing);
    assert_eq!(msg.author.name, "river");
}
```

- [ ] **Step 12: Implement parse_message_line**

```rust
fn parse_message_line(line: &str) -> Option<Message> {
    let line = line.trim();
    if line.len() < 4 {
        return None;
    }

    let direction = parse_direction_marker(&line[..3])?;
    let rest = &line[4..]; // Skip "[x] "

    // Split: timestamp (2 parts), id, <author>, content
    let mut parts = rest.splitn(4, ' ');
    let date = parts.next()?;
    let time = parts.next()?;
    let timestamp = format!("{} {}", date, time);

    let id = parts.next()?.to_string();
    let remainder = parts.next()?;

    // Parse <name:id> and content
    if !remainder.starts_with('<') {
        return None;
    }
    let author_end = remainder.find('>')?;
    let author_part = &remainder[1..author_end];
    let (author_name, author_id) = author_part.rsplit_once(':')?;

    let content = if author_end + 2 < remainder.len() {
        remainder[author_end + 2..].to_string()
    } else {
        String::new()
    };

    Some(Message {
        direction,
        timestamp,
        id,
        author: Author { name: author_name.to_string(), id: author_id.to_string() },
        content,
        reactions: vec![],
    })
}
```

- [ ] **Step 13: Add test for message formatting**

```rust
#[test]
fn test_format_message() {
    let msg = Message {
        direction: MessageDirection::Unread,
        timestamp: "2026-03-23 14:30:00".into(),
        id: "msg123".into(),
        author: Author { name: "alice".into(), id: "111".into() },
        content: "hello".into(),
        reactions: vec![],
    };
    assert_eq!(format_message(&msg), "[ ] 2026-03-23 14:30:00 msg123 <alice:111> hello");
}
```

- [ ] **Step 14: Implement format_message**

```rust
fn format_message(msg: &Message) -> String {
    format!(
        "{} {} {} <{}:{}> {}",
        direction_to_marker(msg.direction),
        msg.timestamp,
        msg.id,
        msg.author.name,
        msg.author.id,
        msg.content
    )
}
```

- [ ] **Step 15: Add test for full conversation roundtrip**

```rust
#[test]
fn test_conversation_roundtrip() {
    let mut conv = Conversation::default();
    conv.messages.push(Message {
        direction: MessageDirection::Unread,
        timestamp: "2026-03-23 14:30:00".into(),
        id: "msg123".into(),
        author: Author { name: "alice".into(), id: "111".into() },
        content: "hello".into(),
        reactions: vec![
            Reaction { emoji: "👍".into(), users: vec!["bob".into()], unknown_count: 0 },
        ],
    });
    conv.messages.push(Message::outgoing("msg124", Author { name: "river".into(), id: "999".into() }, "hi!"));

    let serialized = conv.to_string();
    let parsed = Conversation::from_str(&serialized).unwrap();

    assert_eq!(parsed.messages.len(), 2);
    assert_eq!(parsed.messages[0].reactions.len(), 1);
    assert_eq!(parsed.messages[1].direction, MessageDirection::Outgoing);
}
```

- [ ] **Step 16: Implement Conversation::to_string and from_str**

```rust
impl Conversation {
    pub fn to_string(&self) -> String {
        let mut lines = Vec::new();
        for msg in &self.messages {
            lines.push(format_message(msg));
            for r in &msg.reactions {
                lines.push(format_reaction(r));
            }
        }
        lines.join("\n")
    }

    pub fn from_str(s: &str) -> Result<Self, ParseError> {
        let mut messages = Vec::new();
        let mut current_msg: Option<Message> = None;

        for line in s.lines() {
            if line.starts_with("    ") {
                // Reaction line
                if let Some(ref mut msg) = current_msg {
                    if let Some(r) = parse_reaction_line(line) {
                        msg.reactions.push(r);
                    }
                }
            } else if !line.trim().is_empty() {
                // Message line
                if let Some(msg) = current_msg.take() {
                    messages.push(msg);
                }
                current_msg = parse_message_line(line);
            }
        }

        if let Some(msg) = current_msg {
            messages.push(msg);
        }

        Ok(Conversation { messages })
    }
}

#[derive(Debug)]
pub struct ParseError(pub String);
```

- [ ] **Step 17: Add pub mod format to mod.rs**

```rust
// crates/river-gateway/src/conversations/mod.rs
pub mod format;
```

- [ ] **Step 18: Run all tests and commit**

Run: `cargo test -p river-gateway conversations`
Expected: PASS

```bash
git add crates/river-gateway/src/conversations/
git commit -m "feat(conversations): custom serialization format

Human-readable file format with reactions as indented lines.
Supports direction markers, author parsing, reaction formats."
```

---

### Task 3: Path Helpers

**Files:**
- Create: `crates/river-gateway/src/conversations/path.rs`
- Modify: `crates/river-gateway/src/conversations/mod.rs`

- [ ] **Step 1: Write test for sanitize_name**

```rust
// crates/river-gateway/src/conversations/path.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("general"), "general");
        assert_eq!(sanitize_name("my/channel"), "my_channel");
        assert_eq!(sanitize_name("my\\channel"), "my_channel");
        assert_eq!(sanitize_name(""), "unknown");
    }
}
```

- [ ] **Step 2: Implement sanitize_name (copy from inbox/writer.rs)**

```rust
//! Path building for conversation files

use std::path::{Path, PathBuf};
use super::CONVERSATIONS_DIR;

pub fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            c => c,
        })
        .take(50)
        .collect();

    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}
```

- [ ] **Step 3: Add test for build_discord_path**

```rust
#[test]
fn test_build_discord_path_with_guild() {
    let workspace = Path::new("/workspace");
    let path = build_discord_path(
        workspace,
        Some("123456"),
        Some("myserver"),
        "789012",
        "general",
    );
    assert_eq!(
        path,
        PathBuf::from("/workspace/conversations/discord/123456-myserver/789012-general.txt")
    );
}

#[test]
fn test_build_discord_path_dm() {
    let workspace = Path::new("/workspace");
    let path = build_discord_path(workspace, None, None, "111222", "alice");
    assert_eq!(
        path,
        PathBuf::from("/workspace/conversations/discord/dm/111222-alice.txt")
    );
}
```

- [ ] **Step 4: Implement build_discord_path**

```rust
pub fn build_discord_path(
    workspace: &Path,
    guild_id: Option<&str>,
    guild_name: Option<&str>,
    channel_id: &str,
    channel_name: &str,
) -> PathBuf {
    let mut path = workspace.join(CONVERSATIONS_DIR).join("discord");

    match (guild_id, guild_name) {
        (Some(gid), Some(gname)) => {
            path = path.join(format!("{}-{}", gid, sanitize_name(gname)));
        }
        (Some(gid), None) => {
            path = path.join(format!("{}-unknown", gid));
        }
        (None, _) => {
            path = path.join("dm");
        }
    }

    path.join(format!("{}-{}.txt", channel_id, sanitize_name(channel_name)))
}
```

- [ ] **Step 5: Add pub mod path to mod.rs and run tests**

```rust
// crates/river-gateway/src/conversations/mod.rs
pub mod path;
```

Run: `cargo test -p river-gateway conversations::path`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/conversations/path.rs crates/river-gateway/src/conversations/mod.rs
git commit -m "feat(conversations): path building helpers

Reuses sanitize_name pattern from inbox, builds paths under conversations/"
```

---

### Task 4: ConversationWriter

**Files:**
- Create: `crates/river-gateway/src/conversations/writer.rs`
- Modify: `crates/river-gateway/src/conversations/mod.rs`

- [ ] **Step 1: Write test for Conversation::apply with Message**

```rust
// crates/river-gateway/src/conversations/mod.rs (in tests module)
#[test]
fn test_conversation_apply_message_new() {
    let mut conv = Conversation::default();
    let msg = Message {
        direction: MessageDirection::Unread,
        timestamp: "2026-03-23 14:30:00".into(),
        id: "msg1".into(),
        author: Author { name: "alice".into(), id: "111".into() },
        content: "hello".into(),
        reactions: vec![],
    };
    conv.apply_message(msg.clone());
    assert_eq!(conv.messages.len(), 1);
    assert_eq!(conv.messages[0].id, "msg1");
}

#[test]
fn test_conversation_apply_message_merge_duplicate() {
    let mut conv = Conversation::default();
    let msg1 = Message {
        direction: MessageDirection::Unread,
        timestamp: "2026-03-23 14:30:00".into(),
        id: "msg1".into(),
        author: Author { name: "alice".into(), id: "111".into() },
        content: "hello".into(),
        reactions: vec![],
    };
    conv.apply_message(msg1);

    // Same ID, different content (edit)
    let msg2 = Message {
        direction: MessageDirection::Unread,
        timestamp: "2026-03-23 14:30:00".into(),
        id: "msg1".into(),
        author: Author { name: "alice".into(), id: "111".into() },
        content: "hello edited".into(),
        reactions: vec![],
    };
    conv.apply_message(msg2);

    assert_eq!(conv.messages.len(), 1); // Still 1
    assert_eq!(conv.messages[0].content, "hello edited"); // Content updated
}
```

- [ ] **Step 2: Implement Conversation::apply_message and Message::merge**

```rust
impl Conversation {
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
}

impl Message {
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway conversations::tests`
Expected: PASS

- [ ] **Step 4: Create writer.rs with ConversationWriter**

```rust
// crates/river-gateway/src/conversations/writer.rs
//! Single-writer task for conversation files

use super::{Conversation, WriteOp};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub struct ConversationWriter {
    rx: mpsc::Receiver<WriteOp>,
    conversations: HashMap<PathBuf, Conversation>,
}

impl ConversationWriter {
    pub fn new(rx: mpsc::Receiver<WriteOp>) -> Self {
        Self {
            rx,
            conversations: HashMap::new(),
        }
    }

    pub async fn run(&mut self) {
        while let Some(op) = self.rx.recv().await {
            let path = op.path().clone();
            let conv = self.get_or_load(&path);
            self.apply(conv, op);
            if let Err(e) = conv.save(&path) {
                tracing::error!("Failed to save conversation {:?}: {}", path, e);
            }
        }
    }

    fn get_or_load(&mut self, path: &PathBuf) -> &mut Conversation {
        if !self.conversations.contains_key(path) {
            let conv = Conversation::load(path).unwrap_or_default();
            self.conversations.insert(path.clone(), conv);
        }
        self.conversations.get_mut(path).unwrap()
    }

    fn apply(&self, conv: &mut Conversation, op: WriteOp) {
        match op {
            WriteOp::Message { msg, .. } => {
                conv.apply_message(msg);
            }
            WriteOp::ReactionAdd { message_id, emoji, user, .. } => {
                if let Some(msg) = conv.get_mut(&message_id) {
                    msg.add_reaction(&emoji, &user);
                }
            }
            WriteOp::ReactionRemove { message_id, emoji, user, .. } => {
                if let Some(msg) = conv.get_mut(&message_id) {
                    msg.remove_reaction(&emoji, &user);
                }
            }
            WriteOp::ReactionCount { message_id, emoji, count, .. } => {
                if let Some(msg) = conv.get_mut(&message_id) {
                    msg.update_reaction_count(&emoji, count);
                }
            }
        }
    }
}
```

- [ ] **Step 5: Add Conversation::load and save methods**

```rust
// crates/river-gateway/src/conversations/mod.rs
impl Conversation {
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        Self::from_str(&content).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.0)
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_string())
    }
}
```

- [ ] **Step 6: Add pub mod writer and run tests**

```rust
// crates/river-gateway/src/conversations/mod.rs
pub mod writer;
```

Run: `cargo test -p river-gateway conversations`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/river-gateway/src/conversations/
git commit -m "feat(conversations): ConversationWriter single-writer task

Handles WriteOps, merges messages, immediate file writes."
```

---

### Task 5: Discord /read Endpoint

**Files:**
- Modify: `crates/river-discord/src/outbound.rs`
- Modify: `crates/river-discord/src/client.rs` (if needed)

- [ ] **Step 1: Add ReadRequest and ReadResponse types**

```rust
// crates/river-discord/src/outbound.rs

#[derive(Debug, Deserialize)]
pub struct ReadQuery {
    pub channel: String,
    #[serde(default = "default_limit")]
    pub limit: u64,
    pub before: Option<String>,
}

fn default_limit() -> u64 { 50 }

#[derive(Debug, Serialize)]
pub struct ReadMessage {
    pub id: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: i64,
    pub is_bot: bool,
    pub reactions: Vec<ReadReaction>,
}

#[derive(Debug, Serialize)]
pub struct ReadReaction {
    pub emoji: String,
    pub count: usize,
}
```

- [ ] **Step 2: Add /read route to router**

```rust
// In create_router()
.route("/read", get(handle_read))
```

- [ ] **Step 3: Implement handle_read**

```rust
async fn handle_read(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Vec<ReadMessage>>, (StatusCode, Json<SendResponse>)> {
    let discord_guard = state.discord.read().await;
    let Some(ref discord) = *discord_guard else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some("discord client not initialized".to_string()),
            }),
        ));
    };

    let channel_id: u64 = query.channel.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some("invalid channel id".to_string()),
            }),
        )
    })?;

    let before_id: Option<u64> = query.before
        .as_ref()
        .map(|s| s.parse())
        .transpose()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(SendResponse {
                    success: false,
                    message_id: None,
                    error: Some("invalid before message id".to_string()),
                }),
            )
        })?;

    let limit = query.limit.min(100) as u16;

    let messages = discord
        .read_messages(channel_id, limit, before_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(SendResponse {
                    success: false,
                    message_id: None,
                    error: Some(format!("discord api error: {}", e)),
                }),
            )
        })?;

    Ok(Json(messages))
}
```

- [ ] **Step 4: Add read_messages to DiscordSender**

```rust
// crates/river-discord/src/client.rs
impl DiscordSender {
    pub async fn read_messages(
        &self,
        channel_id: u64,
        limit: u16,
        before: Option<u64>,
    ) -> Result<Vec<ReadMessage>, serenity::Error> {
        use serenity::model::id::ChannelId;

        let channel = ChannelId::new(channel_id);
        let mut builder = channel.messages(&self.http).limit(limit);

        if let Some(before_id) = before {
            builder = builder.before(serenity::model::id::MessageId::new(before_id));
        }

        let messages = builder.await?;

        Ok(messages
            .into_iter()
            .map(|m| ReadMessage {
                id: m.id.to_string(),
                author_id: m.author.id.to_string(),
                author_name: m.author.name.clone(),
                content: m.content.clone(),
                timestamp: m.timestamp.unix_timestamp(),
                is_bot: m.author.bot,
                reactions: m.reactions.iter().map(|r| {
                    ReadReaction {
                        emoji: r.reaction_type.to_string(),
                        count: r.count as usize,
                    }
                }).collect(),
            })
            .collect())
    }
}
```

- [ ] **Step 5: Add imports and test**

Run: `cargo build -p river-discord`
Expected: PASS

- [ ] **Step 6: Write unit test for endpoint**

```rust
#[tokio::test]
async fn test_read_endpoint_requires_channel() {
    // Test that missing channel returns error
}
```

- [ ] **Step 7: Commit**

```bash
git add crates/river-discord/src/outbound.rs crates/river-discord/src/client.rs
git commit -m "feat(discord): add /read endpoint for message history

Returns messages with reactions, supports pagination via before param."
```

---

### Task 6: SyncConversationTool

**Files:**
- Create: `crates/river-gateway/src/tools/sync.rs`
- Modify: `crates/river-gateway/src/tools/mod.rs`

- [ ] **Step 1: Create SyncConversationTool struct**

```rust
// crates/river-gateway/src/tools/sync.rs
//! sync_conversation tool for fetching and merging message history

use crate::conversations::{Author, Message, MessageDirection, WriteOp};
use crate::tools::{AdapterRegistry, Tool, ToolResult};
use river_core::RiverError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

pub struct SyncConversationTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    workspace: PathBuf,
    writer_tx: mpsc::Sender<WriteOp>,
}

impl SyncConversationTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        workspace: PathBuf,
        writer_tx: mpsc::Sender<WriteOp>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            workspace,
            writer_tx,
        }
    }
}
```

- [ ] **Step 2: Implement Tool trait**

```rust
impl Tool for SyncConversationTool {
    fn name(&self) -> &str {
        "sync_conversation"
    }

    fn description(&self) -> &str {
        "Fetch message history from adapter and merge into conversation file"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "adapter": {
                    "type": "string",
                    "description": "Adapter name (e.g., 'discord')"
                },
                "channel": {
                    "type": "string",
                    "description": "Channel ID to sync"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max messages to fetch (default: 50)"
                },
                "before": {
                    "type": "string",
                    "description": "Fetch messages before this ID (pagination)"
                }
            },
            "required": ["adapter", "channel"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let adapter = args["adapter"].as_str()
            .ok_or_else(|| RiverError::tool("Missing 'adapter' parameter"))?;
        let channel = args["channel"].as_str()
            .ok_or_else(|| RiverError::tool("Missing 'channel' parameter"))?;
        let limit = args["limit"].as_u64().unwrap_or(50);
        let before = args["before"].as_str().map(String::from);

        // Block on async
        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let workspace = self.workspace.clone();
        let writer_tx = self.writer_tx.clone();
        let adapter = adapter.to_string();
        let channel = channel.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;
                let config = registry.get(&adapter)
                    .ok_or_else(|| RiverError::tool(format!("Unknown adapter: {}", adapter)))?;

                let read_url = config.read_url.as_ref()
                    .ok_or_else(|| RiverError::tool("Adapter doesn't support reading"))?;

                // Build URL with params
                let mut url = format!("{}?channel={}&limit={}", read_url, channel, limit);
                if let Some(ref before_id) = before {
                    url.push_str(&format!("&before={}", before_id));
                }

                // Fetch messages
                let response = http_client.get(&url).send().await
                    .map_err(|e| RiverError::tool(format!("HTTP error: {}", e)))?;

                if !response.status().is_success() {
                    let body = response.text().await.unwrap_or_default();
                    return Err(RiverError::tool(format!("Adapter error: {}", body)));
                }

                let messages: Vec<FetchedMessage> = response.json().await
                    .map_err(|e| RiverError::tool(format!("Parse error: {}", e)))?;

                // Build conversation path (simplified - would need channel metadata)
                let path = workspace.join("conversations").join(&adapter).join(format!("{}.txt", channel));

                let mut new_count = 0;
                for fetched in &messages {
                    let msg = Message {
                        direction: if fetched.is_bot {
                            MessageDirection::Outgoing
                        } else {
                            MessageDirection::Read // Assume read from history
                        },
                        timestamp: chrono::DateTime::from_timestamp(fetched.timestamp, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_default(),
                        id: fetched.id.clone(),
                        author: Author { name: fetched.author_name.clone(), id: fetched.author_id.clone() },
                        content: fetched.content.clone(),
                        reactions: vec![],
                    };

                    writer_tx.send(WriteOp::Message { path: path.clone(), msg }).await
                        .map_err(|_| RiverError::tool("Writer channel closed"))?;

                    // Send reaction counts
                    for r in &fetched.reactions {
                        writer_tx.send(WriteOp::ReactionCount {
                            path: path.clone(),
                            message_id: fetched.id.clone(),
                            emoji: r.emoji.clone(),
                            count: r.count,
                        }).await
                        .map_err(|_| RiverError::tool("Writer channel closed"))?;
                    }

                    new_count += 1;
                }

                Ok(ToolResult::success(serde_json::json!({
                    "fetched": messages.len(),
                    "processed": new_count,
                }).to_string()))
            })
        })
    }
}

#[derive(Debug, Deserialize)]
struct FetchedMessage {
    id: String,
    author_id: String,
    author_name: String,
    content: String,
    timestamp: i64,
    is_bot: bool,
    reactions: Vec<FetchedReaction>,
}

#[derive(Debug, Deserialize)]
struct FetchedReaction {
    emoji: String,
    count: usize,
}
```

- [ ] **Step 3: Add to tools/mod.rs**

```rust
mod sync;
pub use sync::SyncConversationTool;
```

- [ ] **Step 4: Build and verify**

Run: `cargo build -p river-gateway`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/tools/sync.rs crates/river-gateway/src/tools/mod.rs
git commit -m "feat(tools): add sync_conversation tool

Fetches message history from adapter, sends WriteOps to writer."
```

---

### Task 7: SendMessageTool Integration

**Files:**
- Modify: `crates/river-gateway/src/tools/communication.rs`

- [ ] **Step 1: Add workspace and agent fields to SendMessageTool**

```rust
pub struct SendMessageTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    workspace: PathBuf,
    agent_name: String,
    agent_id: String,
    writer_tx: mpsc::Sender<WriteOp>,
}
```

- [ ] **Step 2: Update constructor**

```rust
impl SendMessageTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        workspace: PathBuf,
        agent_name: String,
        agent_id: String,
        writer_tx: mpsc::Sender<WriteOp>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            workspace,
            agent_name,
            agent_id,
            writer_tx,
        }
    }

    fn agent_author(&self) -> Author {
        Author {
            name: self.agent_name.clone(),
            id: self.agent_id.clone(),
        }
    }
}
```

- [ ] **Step 3: Update execute to record outgoing messages**

In the success branch after sending:

```rust
// Record outgoing message
let conv_path = crate::conversations::path::build_discord_path(
    &self.workspace,
    None, // Would need guild info
    None,
    &channel,
    &channel, // Simplified
);

let msg = Message::outgoing(
    &message_id,
    self.agent_author(),
    &content,
);

let _ = self.writer_tx.send(WriteOp::Message {
    path: conv_path,
    msg,
}).await;
```

In the error branch:

```rust
// Record failed message
let msg = Message::failed(
    self.agent_author(),
    &error.to_string(),
    &content,
);

let _ = self.writer_tx.send(WriteOp::Message {
    path: conv_path,
    msg,
}).await;
```

- [ ] **Step 4: Add required imports**

```rust
use crate::conversations::{Author, Message, WriteOp};
use tokio::sync::mpsc;
```

- [ ] **Step 5: Build and verify**

Run: `cargo build -p river-gateway`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/tools/communication.rs
git commit -m "feat(send_message): record outgoing messages to conversation files

Sends WriteOp to ConversationWriter on success and failure."
```

---

### Task 8: Server Wiring and Migration

**Files:**
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Add inbox → conversations migration at startup**

```rust
// Near the start of run(), after workspace validation
let inbox_path = config.workspace.join("inbox");
let conversations_path = config.workspace.join("conversations");
if inbox_path.exists() && !conversations_path.exists() {
    std::fs::rename(&inbox_path, &conversations_path)?;
    tracing::info!("Migrated inbox/ to conversations/");
}
```

- [ ] **Step 2: Create and spawn ConversationWriter**

```rust
use crate::conversations::writer::ConversationWriter;

// Create writer channel
let (writer_tx, writer_rx) = mpsc::channel::<WriteOp>(256);

// Spawn writer task
let mut conversation_writer = ConversationWriter::new(writer_rx);
tokio::spawn(async move {
    conversation_writer.run().await;
});
```

- [ ] **Step 3: Update SendMessageTool registration**

```rust
registry.register(Box::new(SendMessageTool::new(
    adapter_registry.clone(),
    config.workspace.clone(),
    config.agent_name.clone(),
    snowflake_gen.node_id().to_string(), // Use snowflake node as agent ID
    writer_tx.clone(),
)));
```

- [ ] **Step 4: Register SyncConversationTool**

```rust
use crate::tools::SyncConversationTool;

registry.register(Box::new(SyncConversationTool::new(
    adapter_registry.clone(),
    config.workspace.clone(),
    writer_tx.clone(),
)));
tracing::info!("Registered sync_conversation tool");
```

- [ ] **Step 5: Add imports**

```rust
use crate::conversations::WriteOp;
```

- [ ] **Step 6: Build and test startup**

Run: `cargo build -p river-gateway`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/river-gateway/src/server.rs
git commit -m "feat(server): wire up ConversationWriter and migration

Spawns writer task, passes tx to tools, migrates inbox/ on startup."
```

---

### Task 9: Integration Testing

**Files:**
- All conversation-related files

- [ ] **Step 1: Run all unit tests**

Run: `cargo test -p river-gateway conversations`
Expected: PASS

- [ ] **Step 2: Run full test suite**

Run: `cargo test -p river-gateway`
Expected: PASS

- [ ] **Step 3: Run Discord adapter tests**

Run: `cargo test -p river-discord`
Expected: PASS

- [ ] **Step 4: Build entire workspace**

Run: `cargo build --workspace`
Expected: PASS

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "test(conversations): verify integration

All tests passing, workspace builds clean."
```

---

## Summary

This plan transforms the inbox system into bidirectional conversations in 9 tasks:

1. **Core Types** - MessageDirection, Author, Reaction, Message, Conversation, WriteOp
2. **Custom Format** - Human-readable serialization with reactions as indented lines
3. **Path Helpers** - Build conversation file paths
4. **ConversationWriter** - Single-writer task handling all updates
5. **Discord /read** - Endpoint for fetching message history
6. **SyncConversationTool** - Tool for merging history into files
7. **SendMessageTool** - Integration to record outgoing messages
8. **Server Wiring** - Migration, spawn writer, register tools
9. **Integration Testing** - Verify everything works together

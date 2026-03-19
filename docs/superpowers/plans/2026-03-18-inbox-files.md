# Inbox Files Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace direct message delivery with file-based inbox where messages are appended to text files and the agent marks them as read by editing.

**Architecture:** Gateway writes incoming messages to `inbox/{adapter}/{hierarchy}/{channel}.txt` files, sends lightweight `InboxUpdate` event to loop with affected file paths. Agent reads files, processes unread `[ ]` lines, marks `[x]` when done.

**Tech Stack:** Rust, std::fs, tokio, serde

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/river-gateway/src/inbox/mod.rs` | Module exports |
| `crates/river-gateway/src/inbox/format.rs` | Line formatting, parsing, escaping |
| `crates/river-gateway/src/inbox/writer.rs` | Path building, file creation, appending |
| `crates/river-gateway/src/inbox/reader.rs` | Reading and parsing inbox files |
| `crates/river-gateway/src/loop/state.rs` | Add `InboxUpdate` event and `Inbox` trigger |
| `crates/river-gateway/src/api/routes.rs` | Update `/incoming` to write inbox + notify |
| `crates/river-gateway/src/loop/mod.rs` | Handle `InboxUpdate`, read inbox in wake |
| `crates/river-discord/src/handler.rs` | Add guild/channel names to events |
| `crates/river-discord/src/gateway.rs` | Update `IncomingEvent` struct |

---

### Task 1: Add Inbox Message Format Module

**Files:**
- Create: `crates/river-gateway/src/inbox/mod.rs`
- Create: `crates/river-gateway/src/inbox/format.rs`

- [ ] **Step 1: Create inbox module structure**

Create `crates/river-gateway/src/inbox/mod.rs`:

```rust
//! File-based message inbox
//!
//! Messages are stored as human-readable text files with one message per line.
//! Format: `[status] timestamp messageId <authorName:authorId> content`

pub mod format;

pub use format::{escape_content, unescape_content, format_inbox_line, parse_inbox_line, InboxMessage};
```

- [ ] **Step 2: Create format.rs with escaping functions**

Create `crates/river-gateway/src/inbox/format.rs`:

```rust
//! Inbox line formatting and parsing

/// Escape content for single-line storage
/// Escapes: \ -> \\, \n -> \\n, \r -> \\r
pub fn escape_content(content: &str) -> String {
    content
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Unescape content from storage format
/// Unescapes: \\n -> \n, \\r -> \r, \\\\ -> \
pub fn unescape_content(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    result.push('\n');
                }
                Some('r') => {
                    chars.next();
                    result.push('\r');
                }
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                _ => result.push(c),
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Parsed inbox message
#[derive(Debug, Clone, PartialEq)]
pub struct InboxMessage {
    pub read: bool,
    pub timestamp: String,
    pub message_id: String,
    pub author_name: String,
    pub author_id: String,
    pub content: String,
    pub line_number: usize,
}

/// Format a message as an inbox line
pub fn format_inbox_line(
    timestamp: &str,
    message_id: &str,
    author_name: &str,
    author_id: &str,
    content: &str,
) -> String {
    format!(
        "[ ] {} {} <{}:{}> {}",
        timestamp,
        message_id,
        author_name,
        author_id,
        escape_content(content)
    )
}

/// Parse an inbox line into its components
/// Returns None if line is malformed
pub fn parse_inbox_line(line: &str, line_number: usize) -> Option<InboxMessage> {
    // Format: [status] timestamp messageId <name:id> content
    // Example: [ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there

    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Parse read status
    let (read, rest) = if line.starts_with("[x] ") {
        (true, &line[4..])
    } else if line.starts_with("[ ] ") {
        (false, &line[4..])
    } else {
        return None;
    };

    // Split into parts: timestamp (2 parts), message_id, <author>, content
    let mut parts = rest.splitn(4, ' ');

    let date = parts.next()?;
    let time = parts.next()?;
    let timestamp = format!("{} {}", date, time);

    let message_id = parts.next()?.to_string();

    let remainder = parts.next()?;

    // Parse <name:id> and content
    if !remainder.starts_with('<') {
        return None;
    }

    let author_end = remainder.find('>')?;
    let author_part = &remainder[1..author_end];
    let content_start = author_end + 2; // Skip "> "

    let (author_name, author_id) = author_part.rsplit_once(':')?;

    let content = if content_start < remainder.len() {
        unescape_content(&remainder[content_start..])
    } else {
        String::new()
    };

    Some(InboxMessage {
        read,
        timestamp,
        message_id,
        author_name: author_name.to_string(),
        author_id: author_id.to_string(),
        content,
        line_number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_content() {
        assert_eq!(escape_content("hello"), "hello");
        assert_eq!(escape_content("hello\nworld"), "hello\\nworld");
        assert_eq!(escape_content("a\\b"), "a\\\\b");
        assert_eq!(escape_content("a\r\nb"), "a\\r\\nb");
    }

    #[test]
    fn test_unescape_content() {
        assert_eq!(unescape_content("hello"), "hello");
        assert_eq!(unescape_content("hello\\nworld"), "hello\nworld");
        assert_eq!(unescape_content("a\\\\b"), "a\\b");
        assert_eq!(unescape_content("a\\r\\nb"), "a\r\nb");
    }

    #[test]
    fn test_escape_unescape_roundtrip() {
        let original = "hello\nworld\r\nwith\\backslash";
        let escaped = escape_content(original);
        let unescaped = unescape_content(&escaped);
        assert_eq!(unescaped, original);
    }

    #[test]
    fn test_format_inbox_line() {
        let line = format_inbox_line(
            "2026-03-18 22:15:32",
            "abc123",
            "alice",
            "123456789",
            "hello there",
        );
        assert_eq!(line, "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there");
    }

    #[test]
    fn test_format_inbox_line_with_newline() {
        let line = format_inbox_line(
            "2026-03-18 22:15:32",
            "abc123",
            "alice",
            "123456789",
            "hello\nthere",
        );
        assert_eq!(line, "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello\\nthere");
    }

    #[test]
    fn test_parse_inbox_line_unread() {
        let msg = parse_inbox_line(
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there",
            0,
        ).unwrap();

        assert!(!msg.read);
        assert_eq!(msg.timestamp, "2026-03-18 22:15:32");
        assert_eq!(msg.message_id, "abc123");
        assert_eq!(msg.author_name, "alice");
        assert_eq!(msg.author_id, "123456789");
        assert_eq!(msg.content, "hello there");
        assert_eq!(msg.line_number, 0);
    }

    #[test]
    fn test_parse_inbox_line_read() {
        let msg = parse_inbox_line(
            "[x] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there",
            5,
        ).unwrap();

        assert!(msg.read);
        assert_eq!(msg.line_number, 5);
    }

    #[test]
    fn test_parse_inbox_line_with_escaped_newline() {
        let msg = parse_inbox_line(
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello\\nthere",
            0,
        ).unwrap();

        assert_eq!(msg.content, "hello\nthere");
    }

    #[test]
    fn test_parse_inbox_line_empty_content() {
        let msg = parse_inbox_line(
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> ",
            0,
        ).unwrap();

        assert_eq!(msg.content, "");
    }

    #[test]
    fn test_parse_inbox_line_malformed() {
        assert!(parse_inbox_line("not a valid line", 0).is_none());
        assert!(parse_inbox_line("", 0).is_none());
        assert!(parse_inbox_line("[] 2026-03-18 abc", 0).is_none());
    }

    #[test]
    fn test_format_parse_roundtrip() {
        let line = format_inbox_line(
            "2026-03-18 22:15:32",
            "msg123",
            "bob",
            "987654321",
            "hello\nworld",
        );
        let msg = parse_inbox_line(&line, 0).unwrap();

        assert!(!msg.read);
        assert_eq!(msg.timestamp, "2026-03-18 22:15:32");
        assert_eq!(msg.message_id, "msg123");
        assert_eq!(msg.author_name, "bob");
        assert_eq!(msg.author_id, "987654321");
        assert_eq!(msg.content, "hello\nworld");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway inbox::format`
Expected: All 11 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/inbox/mod.rs \
        crates/river-gateway/src/inbox/format.rs
git commit -m "feat(gateway): add inbox message format module"
```

---

### Task 2: Add Inbox Writer Module

**Files:**
- Create: `crates/river-gateway/src/inbox/writer.rs`
- Modify: `crates/river-gateway/src/inbox/mod.rs`

- [ ] **Step 1: Create writer.rs with path sanitization and writing**

Create `crates/river-gateway/src/inbox/writer.rs`:

```rust
//! Inbox file writing operations

use river_core::{RiverError, RiverResult};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Sanitize a user-provided name for safe filesystem use
/// - Replaces path separators with _
/// - Replaces null bytes with _
/// - Limits to 50 characters
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

/// Build inbox file path for a Discord message
pub fn build_discord_path(
    workspace: &Path,
    guild_id: Option<&str>,
    guild_name: Option<&str>,
    channel_id: &str,
    channel_name: &str,
) -> PathBuf {
    let mut path = workspace.join("inbox").join("discord");

    match (guild_id, guild_name) {
        (Some(gid), Some(gname)) => {
            let dir_name = format!("{}-{}", gid, sanitize_name(gname));
            path = path.join(dir_name);
        }
        (Some(gid), None) => {
            let dir_name = format!("{}-unknown", gid);
            path = path.join(dir_name);
        }
        (None, _) => {
            // DM - no guild
            path = path.join("dm");
        }
    }

    let file_name = format!("{}-{}.txt", channel_id, sanitize_name(channel_name));
    path.join(file_name)
}

/// Ensure the parent directory exists
pub fn ensure_parent_dir(path: &Path) -> RiverResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Append a line to an inbox file
pub fn append_line(path: &Path, line: &str) -> RiverResult<()> {
    ensure_parent_dir(path)?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    writeln!(file, "{}", line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_sanitize_name_simple() {
        assert_eq!(sanitize_name("general"), "general");
        assert_eq!(sanitize_name("my-channel"), "my-channel");
    }

    #[test]
    fn test_sanitize_name_path_separators() {
        assert_eq!(sanitize_name("my/channel"), "my_channel");
        assert_eq!(sanitize_name("my\\channel"), "my_channel");
    }

    #[test]
    fn test_sanitize_name_null_byte() {
        assert_eq!(sanitize_name("my\0channel"), "my_channel");
    }

    #[test]
    fn test_sanitize_name_length_limit() {
        let long_name = "a".repeat(100);
        let sanitized = sanitize_name(&long_name);
        assert_eq!(sanitized.len(), 50);
    }

    #[test]
    fn test_sanitize_name_empty() {
        assert_eq!(sanitize_name(""), "unknown");
    }

    #[test]
    fn test_sanitize_name_unicode() {
        assert_eq!(sanitize_name("café"), "café");
        assert_eq!(sanitize_name("日本語"), "日本語");
    }

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
            PathBuf::from("/workspace/inbox/discord/123456-myserver/789012-general.txt")
        );
    }

    #[test]
    fn test_build_discord_path_dm() {
        let workspace = Path::new("/workspace");
        let path = build_discord_path(
            workspace,
            None,
            None,
            "111222",
            "alice",
        );
        assert_eq!(
            path,
            PathBuf::from("/workspace/inbox/discord/dm/111222-alice.txt")
        );
    }

    #[test]
    fn test_build_discord_path_sanitizes_names() {
        let workspace = Path::new("/workspace");
        let path = build_discord_path(
            workspace,
            Some("123"),
            Some("my/server"),
            "456",
            "gen/eral",
        );
        assert_eq!(
            path,
            PathBuf::from("/workspace/inbox/discord/123-my_server/456-gen_eral.txt")
        );
    }

    #[test]
    fn test_append_line_creates_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("inbox/discord/123-server/456-channel.txt");

        append_line(&path, "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello").unwrap();

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello\n");
    }

    #[test]
    fn test_append_line_appends() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        append_line(&path, "line 1").unwrap();
        append_line(&path, "line 2").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "line 1\nline 2\n");
    }
}
```

- [ ] **Step 2: Update mod.rs to export writer**

Update `crates/river-gateway/src/inbox/mod.rs`:

```rust
//! File-based message inbox
//!
//! Messages are stored as human-readable text files with one message per line.
//! Format: `[status] timestamp messageId <authorName:authorId> content`

pub mod format;
pub mod writer;

pub use format::{escape_content, unescape_content, format_inbox_line, parse_inbox_line, InboxMessage};
pub use writer::{sanitize_name, build_discord_path, append_line};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway inbox::writer`
Expected: All 10 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/inbox/writer.rs \
        crates/river-gateway/src/inbox/mod.rs
git commit -m "feat(gateway): add inbox writer module"
```

---

### Task 3: Add Inbox Reader Module

**Files:**
- Create: `crates/river-gateway/src/inbox/reader.rs`
- Modify: `crates/river-gateway/src/inbox/mod.rs`

- [ ] **Step 1: Create reader.rs**

Create `crates/river-gateway/src/inbox/reader.rs`:

```rust
//! Inbox file reading operations

use crate::inbox::format::{parse_inbox_line, InboxMessage};
use river_core::RiverResult;
use std::fs;
use std::path::Path;

/// Read all messages from an inbox file
pub fn read_all_messages(path: &Path) -> RiverResult<Vec<InboxMessage>> {
    let content = fs::read_to_string(path)?;

    Ok(content
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            match parse_inbox_line(line, i) {
                Some(msg) => Some(msg),
                None => {
                    if !line.trim().is_empty() {
                        tracing::warn!(
                            path = %path.display(),
                            line_number = i + 1,
                            "Malformed line in inbox file, skipping"
                        );
                    }
                    None
                }
            }
        })
        .collect())
}

/// Read only unread messages from an inbox file
pub fn read_unread_messages(path: &Path) -> RiverResult<Vec<InboxMessage>> {
    let messages = read_all_messages(path)?;
    Ok(messages.into_iter().filter(|m| !m.read).collect())
}

/// Check if an inbox file has any unread messages
pub fn has_unread_messages(path: &Path) -> RiverResult<bool> {
    let content = fs::read_to_string(path)?;
    Ok(content.lines().any(|line| line.starts_with("[ ] ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::format::format_inbox_line;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_test_file(path: &Path, lines: &[&str]) {
        let mut file = fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{}", line).unwrap();
        }
    }

    #[test]
    fn test_read_all_messages() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "[x] 2026-03-18 22:15:45 def456 <bob:456> world",
        ]);

        let messages = read_all_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(!messages[0].read);
        assert!(messages[1].read);
    }

    #[test]
    fn test_read_unread_messages() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "[x] 2026-03-18 22:15:45 def456 <bob:456> world",
            "[ ] 2026-03-18 22:16:00 ghi789 <alice:123> another",
        ]);

        let messages = read_unread_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message_id, "abc123");
        assert_eq!(messages[1].message_id, "ghi789");
    }

    #[test]
    fn test_has_unread_messages_true() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[x] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "[ ] 2026-03-18 22:15:45 def456 <bob:456> world",
        ]);

        assert!(has_unread_messages(&path).unwrap());
    }

    #[test]
    fn test_has_unread_messages_false() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[x] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "[x] 2026-03-18 22:15:45 def456 <bob:456> world",
        ]);

        assert!(!has_unread_messages(&path).unwrap());
    }

    #[test]
    fn test_read_empty_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[]);

        let messages = read_all_messages(&path).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_read_skips_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "this is not valid",
            "[ ] 2026-03-18 22:15:45 def456 <bob:456> world",
        ]);

        let messages = read_all_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_read_preserves_line_numbers() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123> first",
            "",
            "[ ] 2026-03-18 22:15:45 def456 <bob:456> third",
        ]);

        let messages = read_all_messages(&path).unwrap();
        assert_eq!(messages[0].line_number, 0);
        assert_eq!(messages[1].line_number, 2);
    }
}
```

- [ ] **Step 2: Update mod.rs to export reader**

Update `crates/river-gateway/src/inbox/mod.rs`:

```rust
//! File-based message inbox
//!
//! Messages are stored as human-readable text files with one message per line.
//! Format: `[status] timestamp messageId <authorName:authorId> content`

pub mod format;
pub mod reader;
pub mod writer;

pub use format::{escape_content, unescape_content, format_inbox_line, parse_inbox_line, InboxMessage};
pub use reader::{read_all_messages, read_unread_messages, has_unread_messages};
pub use writer::{sanitize_name, build_discord_path, append_line};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway inbox::reader`
Expected: All 7 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/inbox/reader.rs \
        crates/river-gateway/src/inbox/mod.rs
git commit -m "feat(gateway): add inbox reader module"
```

---

### Task 4: Add InboxUpdate Event and Inbox Trigger

**Files:**
- Modify: `crates/river-gateway/src/loop/state.rs`

- [ ] **Step 1: Add InboxUpdate to LoopEvent**

In `crates/river-gateway/src/loop/state.rs`, update the `LoopEvent` enum:

```rust
use std::path::PathBuf;

/// Events that can wake or signal the loop
#[derive(Debug, Clone)]
pub enum LoopEvent {
    /// Message from communication adapter (DEPRECATED - use InboxUpdate)
    Message(IncomingMessage),
    /// New messages written to inbox files
    InboxUpdate(Vec<PathBuf>),
    /// Heartbeat timer fired
    Heartbeat,
    /// Graceful shutdown requested
    Shutdown,
}
```

- [ ] **Step 2: Add Inbox to WakeTrigger**

Update the `WakeTrigger` enum:

```rust
/// What caused the agent to wake
#[derive(Debug, Clone)]
pub enum WakeTrigger {
    /// User or external message (DEPRECATED - use Inbox)
    Message(IncomingMessage),
    /// New messages in inbox files
    Inbox(Vec<PathBuf>),
    /// Scheduled heartbeat
    Heartbeat,
}
```

- [ ] **Step 3: Add tests for new variants**

Add tests at the end of the tests module:

```rust
#[test]
fn test_loop_event_inbox_update() {
    let paths = vec![PathBuf::from("/inbox/discord/123/456.txt")];
    let event = LoopEvent::InboxUpdate(paths.clone());
    match event {
        LoopEvent::InboxUpdate(p) => {
            assert_eq!(p.len(), 1);
            assert_eq!(p[0], PathBuf::from("/inbox/discord/123/456.txt"));
        }
        _ => panic!("Expected InboxUpdate event"),
    }
}

#[test]
fn test_wake_trigger_inbox() {
    let paths = vec![PathBuf::from("/inbox/test.txt")];
    let trigger = WakeTrigger::Inbox(paths);
    match trigger {
        WakeTrigger::Inbox(p) => {
            assert_eq!(p.len(), 1);
        }
        _ => panic!("Expected Inbox trigger"),
    }
}

#[test]
fn test_waking_with_inbox_trigger() {
    let paths = vec![PathBuf::from("/inbox/test.txt")];
    let state = LoopState::Waking {
        trigger: WakeTrigger::Inbox(paths),
    };
    assert!(!state.is_sleeping());
    assert!(!state.should_queue_messages());
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway loop::state`
Expected: All tests pass (existing + 3 new)

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/loop/state.rs
git commit -m "feat(gateway): add InboxUpdate event and Inbox trigger"
```

---

### Task 5: Register Inbox Module in Gateway

**Files:**
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Add inbox module to lib.rs**

Add to `crates/river-gateway/src/lib.rs`:

```rust
pub mod inbox;
```

- [ ] **Step 2: Run build to verify**

Run: `cargo build -p river-gateway`
Expected: Build succeeds

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/lib.rs
git commit -m "feat(gateway): register inbox module"
```

---

### Task 6: Update IncomingMessage with Channel/Guild Names

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Add name fields to IncomingMessage**

Update the `IncomingMessage` struct in `crates/river-gateway/src/api/routes.rs`:

```rust
/// Incoming message request
#[derive(Debug, Clone, Deserialize)]
pub struct IncomingMessage {
    pub adapter: String,
    pub event_type: String,
    pub channel: String,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub guild_name: Option<String>,
    pub author: Author,
    pub content: String,
    pub message_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    /// Priority level (defaults to Interactive for user messages)
    #[serde(default = "default_priority")]
    pub priority: river_core::Priority,
}
```

- [ ] **Step 2: Update test helper if it exists**

Check for test helpers that construct `IncomingMessage` and update them to include the new fields with default values.

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/api/routes.rs
git commit -m "feat(gateway): add channel/guild name fields to IncomingMessage"
```

---

### Task 7: Update /incoming Handler to Write Inbox

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Add inbox writing to handle_incoming**

Update the `handle_incoming` function:

```rust
use crate::inbox::{format_inbox_line, build_discord_path, append_line};
use chrono::Utc;

async fn handle_incoming(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(msg): Json<IncomingMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    tracing::info!(
        adapter = %msg.adapter,
        event_type = %msg.event_type,
        channel = %msg.channel,
        author_id = %msg.author.id,
        author_name = %msg.author.name,
        content_len = msg.content.len(),
        content_preview = %msg.content.chars().take(100).collect::<String>(),
        message_id = ?msg.message_id,
        priority = ?msg.priority,
        "Received incoming message"
    );

    // Validate authentication
    if let Err(status) = validate_auth(&headers, state.auth_token.as_deref()) {
        tracing::warn!(
            status = %status,
            has_auth_header = headers.get(AUTHORIZATION).is_some(),
            "Authentication failed for incoming message"
        );
        return Err(status);
    }
    tracing::debug!("Authentication passed");

    // Build inbox path and write message
    let inbox_path = if msg.adapter == "discord" {
        build_discord_path(
            &state.config.workspace,
            msg.guild_id.as_deref(),
            msg.guild_name.as_deref(),
            &msg.channel,
            msg.channel_name.as_deref().unwrap_or("unknown"),
        )
    } else {
        // Generic path for other adapters
        state.config.workspace
            .join("inbox")
            .join(&msg.adapter)
            .join(format!("{}.txt", msg.channel))
    };

    // Format and write inbox line
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let message_id = msg.message_id.as_deref().unwrap_or("unknown");
    let line = format_inbox_line(
        &timestamp,
        message_id,
        &msg.author.name,
        &msg.author.id,
        &msg.content,
    );

    if let Err(e) = append_line(&inbox_path, &line) {
        tracing::error!(error = %e, path = %inbox_path.display(), "Failed to write to inbox");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    tracing::debug!(path = %inbox_path.display(), "Message written to inbox");

    // Send inbox update to the loop
    if state.loop_tx.send(LoopEvent::InboxUpdate(vec![inbox_path.clone()])).await.is_err() {
        tracing::error!("Failed to send inbox update to loop - channel closed");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    tracing::info!(
        channel = %msg.channel,
        author = %msg.author.name,
        inbox_path = %inbox_path.display(),
        "Message delivered to inbox"
    );

    Ok(Json(serde_json::json!({
        "status": "delivered",
        "inbox_path": inbox_path.to_string_lossy()
    })))
}
```

- [ ] **Step 2: Add chrono dependency if needed**

Check `Cargo.toml` for chrono. If not present, add:

```toml
chrono = { version = "0.4", default-features = false, features = ["std", "clock"] }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway api`
Expected: All API tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/api/routes.rs \
        crates/river-gateway/Cargo.toml
git commit -m "feat(gateway): write incoming messages to inbox files"
```

---

### Task 8: Update Agent Loop to Handle InboxUpdate

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Handle InboxUpdate in sleep_phase**

In the `sleep_phase` method, add handling for `InboxUpdate`:

```rust
LoopEvent::InboxUpdate(paths) => {
    tracing::info!(file_count = paths.len(), "Inbox update received");
    self.state = LoopState::Waking {
        trigger: WakeTrigger::Inbox(paths),
    };
}
```

- [ ] **Step 2: Handle Inbox trigger in wake_phase**

In the `wake_phase` method, add handling for `WakeTrigger::Inbox`:

```rust
WakeTrigger::Inbox(paths) => {
    for path in &paths {
        match crate::inbox::read_unread_messages(path) {
            Ok(messages) => {
                for msg in messages {
                    let chat_msg = ChatMessage::user(format!(
                        "[{}] <{}:{}> {}",
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown"),
                        msg.author_name,
                        msg.author_id,
                        msg.content
                    ));
                    self.context.add_message(chat_msg.clone());

                    // Persist to context file
                    if let Some(ref file) = self.context_file {
                        if let Err(e) = file.append(&chat_msg) {
                            tracing::error!(error = %e, "Failed to append inbox message to context file");
                        }
                    }
                }
                tracing::info!(
                    path = %path.display(),
                    count = messages.len(),
                    "Processed inbox messages"
                );
            }
            Err(e) => {
                tracing::error!(error = %e, path = %path.display(), "Failed to read inbox file");
            }
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "feat(gateway): handle InboxUpdate events in agent loop"
```

---

### Task 9: Update Discord Adapter to Send Names

**Files:**
- Modify: `crates/river-discord/src/gateway.rs`
- Modify: `crates/river-discord/src/handler.rs`

- [ ] **Step 1: Update IncomingEvent struct**

In `crates/river-discord/src/gateway.rs`, update:

```rust
/// Incoming event sent to gateway
#[derive(Debug, Serialize)]
pub struct IncomingEvent {
    pub adapter: &'static str,
    pub event_type: String,
    pub channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_name: Option<String>,
    pub author: Author,
    pub content: String,
    pub message_id: String,
    pub metadata: EventMetadata,
}
```

- [ ] **Step 2: Update handler to populate names**

In `crates/river-discord/src/handler.rs`, update the message handling to include channel and guild names from the Discord cache or event context.

- [ ] **Step 3: Update test**

Update the serialization test in `gateway.rs`:

```rust
#[test]
fn test_incoming_event_serialization() {
    let event = IncomingEvent {
        adapter: "discord",
        event_type: "message".to_string(),
        channel: "123456".to_string(),
        channel_name: Some("general".to_string()),
        guild_id: Some("guild1".to_string()),
        guild_name: Some("My Server".to_string()),
        author: Author {
            id: "user123".to_string(),
            name: "TestUser".to_string(),
        },
        content: "Hello world".to_string(),
        message_id: "msg789".to_string(),
        metadata: EventMetadata {
            guild_id: Some("guild1".to_string()),
            thread_id: None,
            reply_to: None,
        },
    };

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"adapter\":\"discord\""));
    assert!(json.contains("\"channel_name\":\"general\""));
    assert!(json.contains("\"guild_name\":\"My Server\""));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-discord`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-discord/src/gateway.rs \
        crates/river-discord/src/handler.rs
git commit -m "feat(discord): add channel and guild names to incoming events"
```

---

### Task 10: Integration Testing

**Files:**
- Existing test infrastructure

- [ ] **Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: All tests pass

- [ ] **Step 2: Run release build**

Run: `cargo build --release`
Expected: Build succeeds

- [ ] **Step 3: Manual verification (optional)**

Start the gateway and send a test message to verify:
1. Inbox file is created at correct path
2. Message is formatted correctly
3. Agent receives InboxUpdate event

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "test: fix any integration issues"
```

---

## Summary

| Task | Description | New Files | Modified Files |
|------|-------------|-----------|----------------|
| 1 | Inbox format module | `inbox/mod.rs`, `inbox/format.rs` | - |
| 2 | Inbox writer module | `inbox/writer.rs` | `inbox/mod.rs` |
| 3 | Inbox reader module | `inbox/reader.rs` | `inbox/mod.rs` |
| 4 | InboxUpdate event | - | `loop/state.rs` |
| 5 | Register inbox module | - | `lib.rs` |
| 6 | IncomingMessage fields | - | `api/routes.rs` |
| 7 | /incoming handler | - | `api/routes.rs` |
| 8 | Loop InboxUpdate handling | - | `loop/mod.rs` |
| 9 | Discord adapter names | - | `gateway.rs`, `handler.rs` |
| 10 | Integration testing | - | - |

**Total:** ~800 lines of new code across 4 new files and 6 modified files.

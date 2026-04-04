# Backchannel Design Spec

> Authors: Cass, Claude
> Date: 2026-04-03

## Overview

The backchannel enables communication between workers in a dyad and debugging via river-tui. It uses a shared text file with the same conversation format as external channel conversations, eliminating the need for a separate HTTP server.

**Goals:**
- Workers communicate with each other through standard tooling (`speak` tool)
- River-tui can read and write to the backchannel for debugging
- Reuse conversation file format from archive (port to river-protocol)

**Non-goals:**
- Replace flash messages (those remain for time-sensitive peer-to-peer communication)
- External adapter registration for backchannel

## Part 1: Conversation Module in river-protocol

Port conversation file handling from `archive/river-gateway/src/conversations/` to `river-protocol`, adding read receipt support.

### File Structure

```
river-protocol/src/
  conversation/
    mod.rs        # re-exports, Conversation struct
    types.rs      # Line, Message, MessageDirection, Reaction
    format.rs     # parse/format functions
    meta.rs       # ConversationMeta (YAML frontmatter)
```

### Types

```rust
use serde::{Deserialize, Serialize};
use crate::identity::Author;

/// Message direction/status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageDirection {
    Unread,   // [ ] - incoming, not yet read
    Read,     // [x] - incoming, read
    Outgoing, // [>] - sent by this worker
    Failed,   // [!] - failed to send
}

/// A reaction on a message
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<String>,
    pub unknown_count: usize,
}

/// A message in the conversation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub direction: MessageDirection,
    pub timestamp: String,  // "2026-04-03 14:30:00"
    pub id: String,
    pub author: Author,
    pub content: String,
    pub reactions: Vec<Reaction>,
}

/// A line in a conversation file
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Line {
    Message(Message),
    ReadReceipt {
        timestamp: String,
        message_id: String,
    },
}

/// Conversation metadata (YAML frontmatter)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

/// A conversation file with metadata and lines
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Conversation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ConversationMeta>,
    pub lines: Vec<Line>,
}
```

### Line Format

```
[status] timestamp message_id <author_name:author_id> content
[r] timestamp message_id
```

**Line prefixes:**
| Prefix | Meaning |
|--------|---------|
| `[ ]` | Incoming, unread |
| `[x]` | Incoming, read |
| `[>]` | Outgoing, sent |
| `[!]` | Failed to send |
| `[r]` | Read receipt (no author/content) |

**Reactions** are indented 4 spaces under their message:
```
    👍 bob, charlie      # known users
    👍 3                 # count only
    👍 bob +2            # mixed: 1 known + 2 unknown
```

### File Format

```
---
adapter: discord
channel_id: "789012"
channel_name: general
---
[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey, can you help?
    👍 bob, charlie
[>] 2026-04-03 14:30:15 msg124 <river:999> Sure! What do you need?
[r] 2026-04-03 14:30:20 msg123
[x] 2026-04-03 14:30:30 msg125 <alice:111> I'm trying to deploy...
```

### API

```rust
impl Conversation {
    /// Load conversation from file
    pub fn load(path: &Path) -> Result<Self, std::io::Error>;

    /// Save conversation to file
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error>;

    /// Parse from string
    pub fn from_str(s: &str) -> Result<Self, ParseError>;

    /// Serialize to string
    pub fn to_string(&self) -> String;

    /// Append a line to the file (without loading full conversation)
    pub fn append_line(path: &Path, line: &Line) -> Result<(), std::io::Error>;

    /// Compact: apply read receipts to messages, sort by timestamp, remove receipts
    pub fn compact(&mut self);

    /// Check if compaction is needed (> 100 lines or has read receipts)
    pub fn needs_compaction(&self) -> bool;
}
```

### Compaction Logic

```rust
impl Conversation {
    pub fn compact(&mut self) {
        // 1. Collect all read receipt message IDs
        let read_ids: HashSet<String> = self.lines.iter()
            .filter_map(|line| match line {
                Line::ReadReceipt { message_id, .. } => Some(message_id.clone()),
                _ => None,
            })
            .collect();

        // 2. Filter to messages, apply read status
        let mut messages: Vec<Message> = self.lines.iter()
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
}
```

### Compaction Triggers

Compaction runs:
- On worker startup (before reading backchannel)
- When file exceeds 100 lines
- Explicitly via future `compact` tool (if needed)

### Porting from Archive

Source files to port from `archive/river-gateway/src/conversations/`:
- `mod.rs` → types and Conversation struct
- `format.rs` → parsing and formatting functions
- `meta.rs` → ConversationMeta

Changes from archive:
- Add `Line` enum with `ReadReceipt` variant
- Add `[r]` prefix parsing/formatting
- Add `compact()` method
- Reuse `Author` from `river_protocol::identity` (has extra `bot` field, unused)
- Add `append_line()` for efficient file appending

## Part 2: Wire Up Conversation Management in river-worker

Before implementing backchannel, wire up the conversation module for all conversation files as specified in the worker-design spec.

### Conversation File Paths

Per worker-design spec:
```
workspace/conversations/{adapter}/{guild_id}-{guild_name}/{channel_id}-{channel_name}.txt
workspace/conversations/{adapter}/dm/{user_id}-{user_name}.txt
```

### Integration Points

**1. Handle /notify events (http.rs):**

When worker receives `InboundEvent` from adapter:
```rust
async fn handle_notify(event: InboundEvent) {
    let path = conversation_path_for_channel(&event.channel);

    let msg = Message {
        direction: MessageDirection::Unread,
        timestamp: event.timestamp,
        id: event.message_id,
        author: event.author,
        content: event.content,
        reactions: vec![],
    };

    Conversation::append_line(&path, &Line::Message(msg))?;

    // Queue notification for status message
    state.pending_notifications.push(...);
}
```

**2. Speak tool writes outgoing (tools.rs):**

When `speak` sends a message:
```rust
// After successful send via adapter
let msg = Message {
    direction: MessageDirection::Outgoing,
    timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    id: response.message_id,
    author: Author {
        name: state.dyad.clone(),
        id: state.baton.to_string(),
        bot: true,
    },
    content,
    reactions: vec![],
};

let path = conversation_path_for_channel(&channel);
Conversation::append_line(&path, &Line::Message(msg))?;
```

**3. Mark messages read:**

When worker processes messages from a channel:
```rust
fn mark_messages_read(path: &Path, message_ids: &[String]) {
    for id in message_ids {
        Conversation::append_line(path, &Line::ReadReceipt {
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            message_id: id.clone(),
        })?;
    }
}
```

**4. Failed sends:**

When adapter returns error:
```rust
let msg = Message {
    direction: MessageDirection::Failed,
    timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    id: "-".to_string(),
    author: self_author,
    content: format!("(failed: {}) {}", error, original_content),
    reactions: vec![],
};
Conversation::append_line(&path, &Line::Message(msg))?;
```

**5. Compaction on startup:**

```rust
// In worker startup
fn compact_conversations(workspace: &Path) {
    for entry in walkdir::WalkDir::new(workspace.join("conversations")) {
        if entry.path().extension() == Some("txt") {
            if let Ok(mut convo) = Conversation::load(entry.path()) {
                if convo.needs_compaction() {  // > 100 lines or has read receipts
                    convo.compact();
                    convo.save(entry.path())?;
                }
            }
        }
    }
}
```

**6. Path helper:**

```rust
fn conversation_path_for_channel(workspace: &Path, channel: &Channel) -> PathBuf {
    if let Some(ref guild) = channel.guild {
        workspace
            .join("conversations")
            .join(&channel.adapter)
            .join(format!("{}-{}", guild.id, sanitize(&guild.name)))
            .join(format!("{}-{}.txt", channel.id, sanitize(&channel.name)))
    } else {
        // DM
        workspace
            .join("conversations")
            .join(&channel.adapter)
            .join("dm")
            .join(format!("{}-{}.txt", channel.id, sanitize(&channel.name)))
    }
}
```

## Part 3: Backchannel in river-worker

### File Location

```
workspace/conversations/backchannel.txt
```

### File Format

```
---
adapter: backchannel
channel_id: dyad
---
[>] 2026-04-03 14:30:00 msg-001 <river:actor> User seems frustrated
[r] 2026-04-03 14:30:05 msg-001
[>] 2026-04-03 14:30:10 msg-002 <river:spectator> Noted, I'll adjust tone
```

### Author Format

`<name:role>` where:
- `name` - worker/dyad identity (consistent across role switches)
- `role` - current baton: `actor` or `spectator`

After role switch, name stays same but role changes:
```
[>] 2026-04-03 14:30:00 msg-001 <river:actor> Before switch
[>] 2026-04-03 14:35:00 msg-002 <river:spectator> After switch (same worker, different role)
```

### Speak Tool Integration

When `speak` tool is called with channel "backchannel":

```rust
// In tools.rs speak handler
if channel == "backchannel" {
    // Generate message ID using snowflake
    let id = state.snowflake_generator.next_id()?.to_string();

    // Create message
    let msg = Message {
        direction: MessageDirection::Outgoing,
        timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        id,
        author: Author {
            name: state.dyad.clone(),
            id: state.baton.to_string(),  // "actor" or "spectator"
            bot: true,
        },
        content,
        reactions: vec![],
    };

    // Append to backchannel file
    let path = state.workspace.join("conversations/backchannel.txt");
    Conversation::append_line(&path, &Line::Message(msg))?;

    return Ok(SpeakResult { message_id: id, sent: true });
}
```

### Reading Backchannel

Worker reads backchannel to see messages from partner:

```rust
// Load conversation
let convo = Conversation::load(&backchannel_path)?;

// Find unread messages from partner (different role)
let my_role = state.baton.to_string();
let unread: Vec<&Message> = convo.lines.iter()
    .filter_map(|line| match line {
        Line::Message(msg) if msg.author.id != my_role
            && msg.direction == MessageDirection::Unread => Some(msg),
        _ => None,
    })
    .collect();

// Mark as read
for msg in &unread {
    Conversation::append_line(&path, &Line::ReadReceipt {
        timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        message_id: msg.id.clone(),
    })?;
}
```

### Discovery

1. Check registry for partner endpoint (for flash communication)
2. Backchannel file always available as fallback
3. Config can specify workspace path

## Part 4: River-TUI Integration

River-TUI reads and writes backchannel file directly, no adapter registration needed.

### Tailing

Same pattern as context file tailing:

```rust
async fn tail_backchannel(path: PathBuf, state: SharedState) {
    let mut last_line_count = 0;

    loop {
        if let Ok(convo) = Conversation::load(&path) {
            let new_lines = &convo.lines[last_line_count..];
            for line in new_lines {
                let mut s = state.write().await;
                s.add_backchannel_line(line);
            }
            last_line_count = convo.lines.len();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

### Writing

User can type messages to inject into backchannel:

```rust
// When user submits backchannel message
let msg = Message {
    direction: MessageDirection::Outgoing,
    timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    id: format!("tui-{}", Utc::now().timestamp_millis()),  // TUI-generated ID
    author: Author {
        name: "tui".to_string(),
        id: "debug".to_string(),
        bot: false,
    },
    content: user_input,
    reactions: vec![],
};

Conversation::append_line(&backchannel_path, &Line::Message(msg))?;
```

### Display

Backchannel messages displayed interleaved with context, distinguished by:
- Different color (e.g., cyan for backchannel vs white for context)
- Prefix indicator showing it's backchannel content

## Testing

### Conversation Module Tests

**Parse/format roundtrip:**
- All line types (`[ ]`, `[x]`, `[>]`, `[!]`, `[r]`)
- Messages with reactions
- YAML frontmatter with all fields
- YAML frontmatter with minimal fields
- No frontmatter
- Empty file
- Multiline content (escaped newlines)

**Compaction logic (extensive):**
- Single read receipt marks message as read
- Multiple read receipts for same message (idempotent)
- Read receipt for nonexistent message (ignored)
- Read receipt only affects Unread messages (not Outgoing/Failed)
- Messages sorted by timestamp after compaction
- Read receipts removed after compaction
- Reactions preserved through compaction
- Mixed line types (messages + receipts interleaved)
- Large file compaction (100+ lines)
- Already-compacted file (no read receipts) unchanged
- Compaction with duplicate message IDs (edge case)
- Empty conversation compacts to empty
- Conversation with only read receipts compacts to empty

**needs_compaction() detection:**
- Returns true when > 100 lines
- Returns true when any read receipts present
- Returns false for clean compacted file

### Backchannel Tests

- Speak tool routes to file for "backchannel" channel
- Messages written with correct author format
- Read receipts appended correctly
- Partner messages detected

### TUI Tests

- Backchannel file tailing works
- User messages written correctly

## Migration

1. Port conversation module to river-protocol
2. Wire up conversation management in river-worker (/notify, speak, read receipts, compaction)
3. Add backchannel handling to speak tool (special case for "backchannel" channel)
4. Update river-tui to tail backchannel file

## Dependencies

**river-protocol additions:**
- `serde_yaml` (for frontmatter)
- `chrono` (for timestamps)

## Related Documents

- `docs/superpowers/specs/2026-04-01-worker-design.md` - Conversation file format spec
- `archive/river-gateway/src/conversations/` - Original implementation

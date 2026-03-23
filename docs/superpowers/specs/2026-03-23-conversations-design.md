# Bidirectional Conversations

**Status:** Approved
**Date:** 2026-03-23

## Problem

The agent's message system is one-directional:

1. Incoming messages are written to inbox files
2. Outgoing messages (sent via `send_message`) go to Discord but are NOT recorded locally
3. The agent cannot see its own side of conversations
4. No way to fetch historical messages to rebuild context after rotation

## Solution

Extend the inbox system to capture both directions and add a sync tool for fetching history.

---

## 1. Data Model

### Core Types

```rust
// crates/river-gateway/src/conversations/mod.rs

pub const CONVERSATIONS_DIR: &str = "conversations";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDirection {
    Unread,   // [ ]
    Read,     // [x]
    Outgoing, // [>]
    Failed,   // [!]
}

#[derive(Debug, Clone)]
pub struct Author {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<String>,      // Known usernames (from events)
    pub unknown_count: usize,    // Additional count (from API sync)
}

impl Reaction {
    /// Merge with data from API or events
    pub fn merge(&mut self, other: &Reaction) {
        // Add any new usernames
        for user in &other.users {
            if !self.users.contains(user) {
                self.users.push(user.clone());
            }
        }
        // If API count > known users, track the difference
        let total_other = other.users.len() + other.unknown_count;
        if total_other > self.users.len() {
            self.unknown_count = total_other - self.users.len();
        }
    }

    /// Total reaction count
    pub fn count(&self) -> usize {
        self.users.len() + self.unknown_count
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub direction: MessageDirection,
    pub timestamp: String,
    pub id: String,
    pub author: Author,
    pub content: String,
    pub reactions: Vec<Reaction>,
}

#[derive(Debug, Clone, Default)]
pub struct Conversation {
    pub messages: Vec<Message>,
}
```

### Custom Serialization

```rust
impl Conversation {
    /// Serialize to human-readable format
    pub fn to_string(&self) -> String;

    /// Parse from file contents
    pub fn from_str(s: &str) -> Result<Self, ParseError>;

    /// Load from file path
    pub fn load(path: &Path) -> Result<Self, io::Error>;

    /// Save to file path
    pub fn save(&self, path: &Path) -> Result<(), io::Error>;
}
```

### File Format

Messages are lines starting with `[marker]`. Reactions are indented lines beneath their message.

```
[ ] 2026-03-23 14:30:00 msg123 <alice:111> hey, can you help?
    👍 bob, charlie
    ❤️ 3
[>] 2026-03-23 14:30:15 msg124 <river:999> Sure! What do you need?
[x] 2026-03-23 14:30:30 msg125 <alice:111> I'm trying to deploy...
    🎉 river +2
[!] 2026-03-23 14:31:00 - <river:999> (failed: Connection timeout) Original message
```

| Line Type | Pattern | Meaning |
|-----------|---------|---------|
| `[ ] ...` | Message | Incoming, unread |
| `[x] ...` | Message | Incoming, read |
| `[>] ...` | Message | Outgoing (sent by agent) |
| `[!] ...` | Message | Failed to send |
| `    emoji users` | Reaction | Known users from events |
| `    emoji N` | Reaction | Count only (from API) |
| `    emoji users +N` | Reaction | Known users + N unknown |

Reactions are indented with 4 spaces. Formats:
- `👍 bob, charlie` — usernames known
- `👍 3` — count only (no usernames)
- `👍 bob, charlie +1` — 2 known + 1 unknown = 3 total

### Folder Structure

```
workspace/conversations/
  discord/
    {guild_id}-{guild_name}/
      {channel_id}-{channel_name}.txt
    dm/
      {user_id}-{user_name}.txt
  {adapter}/
    {channel}.txt
```

---

## 2. Conversation Operations

### Append Message

```rust
impl Conversation {
    /// Append a message and save
    pub fn append(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    /// Append and save atomically
    pub fn append_and_save(&mut self, msg: Message, path: &Path) -> Result<(), io::Error> {
        self.append(msg);
        self.save(path)
    }
}
```

### Message Constructors

```rust
impl Message {
    pub fn outgoing(id: &str, author: Author, content: &str) -> Self {
        Self {
            direction: MessageDirection::Outgoing,
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            id: id.to_string(),
            author,
            content: content.to_string(),
            reactions: vec![],
        }
    }

    pub fn failed(author: Author, error: &str, content: &str) -> Self {
        Self {
            direction: MessageDirection::Failed,
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            id: "-".to_string(),
            author,
            content: format!("(failed: {}) {}", error, content),
            reactions: vec![],
        }
    }
}
```

---

## 3. Capture Outgoing Messages

When `send_message` succeeds:

```rust
// In SendMessageTool::execute()
let result = send_to_adapter(...).await?;

// Append to conversation
let mut conv = Conversation::load(&conv_path).unwrap_or_default();
conv.append_and_save(
    Message::outgoing(&result.message_id, self.agent_author(), &content),
    &conv_path,
)?;
```

On failure:

```rust
let mut conv = Conversation::load(&conv_path).unwrap_or_default();
conv.append_and_save(
    Message::failed(self.agent_author(), &error.to_string(), &content),
    &conv_path,
)?;
```

### Dependencies

`SendMessageTool` needs additional fields:
- `workspace: PathBuf`
- `agent_name: String`
- `agent_id: String`

Helper method:
```rust
fn agent_author(&self) -> Author {
    Author { name: self.agent_name.clone(), id: self.agent_id.clone() }
}
```

---

## 4. Discord `/read` Endpoint

Add to `crates/river-discord/src/outbound.rs`:

```
GET /read?channel={id}&limit={n}&before={msg_id}
```

| Param | Required | Default | Description |
|-------|----------|---------|-------------|
| `channel` | Yes | - | Channel ID |
| `limit` | No | 50 | Max messages (capped at 100) |
| `before` | No | - | Fetch messages before this ID (pagination) |

### Discord API Call

```rust
// Uses Discord REST API: GET /channels/{channel_id}/messages
let messages = state.http
    .channel_messages(channel_id)
    .limit(limit)?
    .before(before_id)?  // if provided
    .await?;
```

Requires `MESSAGE_HISTORY` intent (bot already has this).

### Response

```json
[
  {
    "id": "123456789",
    "author_id": "111",
    "author_name": "alice",
    "content": "hello",
    "timestamp": 1711200600,
    "is_bot": false,
    "reactions": [
      { "emoji": "👍", "count": 3 },
      { "emoji": "❤️", "count": 1 }
    ]
  }
]
```

Note: Discord API only provides counts, not usernames. Usernames come from real-time events.
```

`is_bot` comes from Discord's `message.author.bot` field.

### Rate Limiting

Discord allows 50 requests/second per channel. One request per tool call — agent controls pacing.

---

## 5. `sync_conversation` Tool

**Note:** This is separate from the existing `read_channel` tool. `read_channel` returns raw messages to the agent; `sync_conversation` merges them into the conversation file.

### Parameters

```json
{
  "name": "sync_conversation",
  "parameters": {
    "adapter": { "type": "string", "description": "Adapter name (e.g., 'discord')" },
    "channel": { "type": "string", "description": "Channel ID to sync" },
    "limit": { "type": "integer", "description": "Max messages to fetch (default: 50)" },
    "before": { "type": "string", "description": "Fetch messages before this ID (pagination)" }
  },
  "required": ["adapter", "channel"]
}
```

### Behavior

1. Call adapter's `/read` endpoint with params
2. Load existing conversation file (if any)
3. Merge messages (deduplicate by message ID)
4. Sort by message ID (Discord IDs are chronological snowflakes)
5. Write back to file

### Response

```json
{
  "fetched": 50,
  "new": 23,
  "total": 156
}
```

### Pagination

Agent reads oldest message ID from file, passes as `before=` to fetch earlier messages.

---

## 6. File Migration

Code module stays `crates/river-gateway/src/inbox/` — no rename.

File paths change:
- `workspace/inbox/discord/...` → `workspace/conversations/discord/...`

### Startup Migration

```rust
// In crates/river-gateway/src/server.rs, during startup
if workspace.join("inbox").exists() && !workspace.join("conversations").exists() {
    fs::rename(workspace.join("inbox"), workspace.join("conversations"))?;
    tracing::info!("Migrated inbox/ to conversations/");
}
```

---

## 7. Concurrency

Single-writer assumption: Only one agent process writes to a conversation file at a time. If multiple processes need to write:
- Use file locking (`flock`)
- Or accept interleaved writes (messages may be out of order but deduplication handles it)

For now, we assume single-writer. Sync tool reads, merges, writes atomically per file.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/river-gateway/src/tools/communication.rs` | Add workspace/agent fields, capture outgoing messages |
| `crates/river-gateway/src/server.rs` | Pass workspace/agent to SendMessageTool, add migration |
| `crates/river-gateway/src/lib.rs` | Add `pub mod conversations;` |
| `crates/river-discord/src/outbound.rs` | Add `/read` endpoint with reactions |

New files:

| File | Purpose |
|------|---------|
| `crates/river-gateway/src/conversations/mod.rs` | `Conversation`, `Message`, `MessageDirection`, `Reaction` types |
| `crates/river-gateway/src/conversations/format.rs` | `Conversation::to_string()`, `Conversation::from_str()` |
| `crates/river-gateway/src/conversations/path.rs` | Path building helpers (replaces inbox/writer.rs path logic) |
| `crates/river-gateway/src/tools/sync.rs` | `SyncConversationTool` implementation |

Deprecate (keep for migration):

| File | Status |
|------|--------|
| `crates/river-gateway/src/inbox/` | Keep for now, migrate callers to `conversations/` |

---

## Testing

```rust
#[test]
fn test_conversation_roundtrip() {
    let mut conv = Conversation::default();
    conv.append(Message {
        direction: MessageDirection::Unread,
        timestamp: "2026-03-23 14:30:00".into(),
        id: "msg123".into(),
        author: Author { name: "alice".into(), id: "111".into() },
        content: "hello".into(),
        reactions: vec![
            Reaction { emoji: "👍".into(), users: vec!["bob".into()], unknown_count: 0 },
        ],
    });
    conv.append(Message::outgoing("msg124", Author { name: "river".into(), id: "999".into() }, "hi!"));

    let serialized = conv.to_string();
    let parsed = Conversation::from_str(&serialized).unwrap();

    assert_eq!(parsed.messages.len(), 2);
    assert_eq!(parsed.messages[0].reactions.len(), 1);
    assert_eq!(parsed.messages[1].direction, MessageDirection::Outgoing);
}

#[test]
fn test_parse_with_reactions() {
    let input = r#"[ ] 2026-03-23 14:30:00 msg123 <alice:111> hello
    👍 bob, charlie
    ❤️ 3
    🎉 river +2
[>] 2026-03-23 14:30:15 msg124 <river:999> hi there
"#;
    let conv = Conversation::from_str(input).unwrap();

    assert_eq!(conv.messages.len(), 2);
    assert_eq!(conv.messages[0].reactions.len(), 3);

    // Known users
    assert_eq!(conv.messages[0].reactions[0].emoji, "👍");
    assert_eq!(conv.messages[0].reactions[0].users, vec!["bob", "charlie"]);
    assert_eq!(conv.messages[0].reactions[0].count(), 2);

    // Count only
    assert_eq!(conv.messages[0].reactions[1].emoji, "❤️");
    assert_eq!(conv.messages[0].reactions[1].users.len(), 0);
    assert_eq!(conv.messages[0].reactions[1].count(), 3);

    // Mixed
    assert_eq!(conv.messages[0].reactions[2].emoji, "🎉");
    assert_eq!(conv.messages[0].reactions[2].users, vec!["river"]);
    assert_eq!(conv.messages[0].reactions[2].unknown_count, 2);
    assert_eq!(conv.messages[0].reactions[2].count(), 3);
}

#[test]
fn test_message_outgoing_constructor() {
    let msg = Message::outgoing("123", Author { name: "river".into(), id: "999".into() }, "Hello!");
    assert_eq!(msg.direction, MessageDirection::Outgoing);
    assert_eq!(msg.id, "123");
    assert!(msg.reactions.is_empty());
}

#[test]
fn test_sync_merges_and_deduplicates() {
    let mut conv = Conversation::default();
    conv.append(Message::outgoing("msg1", author(), "first"));

    // Simulate sync returning msg1 (duplicate) and msg2 (new)
    let fetched = vec![
        Message { id: "msg1".into(), ..default_msg() },
        Message { id: "msg2".into(), ..default_msg() },
    ];

    conv.merge(fetched);
    assert_eq!(conv.messages.len(), 2); // Not 3
}

#[tokio::test]
async fn test_send_message_appends_to_file() {
    // Send message via tool
    // Verify conversation file contains outgoing message
}
```

---

## Summary

Transform one-directional inbox into bidirectional conversations:

```
Before:                          After:
inbox/discord/ch.txt             conversations/discord/ch.txt
  [ ] alice: hi                    [ ] alice: hi
  [ ] alice: you there?            [>] river: Hello!
                                   [x] alice: you there?
                                   [>] river: Yes, how can I help?
```

The agent can see both sides of conversations, sync history when needed, and maintain continuity across context rotations.

# Chats: Bidirectional Message History

**Status:** Draft
**Author:** Cassie
**Date:** 2026-03-21

## Problem

The agent's inbox is one-directional:

1. **Incoming messages** are written to inbox files
2. **Outgoing messages** (sent via `send_message`) go to Discord but are NOT recorded locally
3. The agent cannot see its own side of conversations
4. No way to fetch historical messages from Discord to rebuild context

**Result:** The agent has amnesia about what it said. After context rotation, it loses both sides of the conversation.

## Goals

1. **Bidirectional chat files** — Both incoming and outgoing messages in one file
2. **History fetching** — Ability to pull message history from Discord API
3. **Sync on demand** — Rebuild chat files when needed (gaps, startup, explicit request)
4. **Multi-adapter support** — Pattern works for Discord, Slack, IRC, etc.

---

## Solution: Inbox → Chats

### Folder Structure

```
workspace/chats/
  discord/
    {guild_id}-{guild_name}/
      {channel_id}-{channel_name}.txt
    dm/
      {user_id}-{user_name}.txt
  {adapter}/
    {channel}.txt
```

### Line Format

Extend current format to indicate direction:

```
[status] timestamp messageId <authorName:authorId> content
```

**Status markers:**
- `[ ]` — Incoming, unread
- `[x]` — Incoming, read
- `[>]` — Outgoing (sent by agent)
- `[!]` — Failed to send (with error in content)

**Examples:**
```
[ ] 2026-03-21 14:30:00 1234567890 <alice:111> hey, can you help me?
[>] 2026-03-21 14:30:15 1234567891 <river:999> Sure! What do you need?
[x] 2026-03-21 14:30:30 1234567892 <alice:111> I'm trying to deploy...
[>] 2026-03-21 14:31:00 1234567893 <river:999> Let me check the config.
```

---

## Implementation

### 1. Capture Outgoing Messages

When `send_message` tool succeeds, append to the chat file.

```rust
// src/tools/communication.rs

impl SendMessageTool {
    async fn execute(&self, params: SendMessageParams) -> Result<Value> {
        // Send to adapter (existing)
        let response = self.client.post(&url).json(&payload).send().await?;
        let result: SendResult = response.json().await?;

        // NEW: Append to chat file
        let chat_path = build_chat_path(&self.workspace, &params.adapter, &params.channel);
        let line = format_outgoing_line(
            result.message_id,
            &self.agent_name,
            &self.agent_id,
            &params.content,
        );
        append_line(&chat_path, &line).await?;

        Ok(json!({ "message_id": result.message_id }))
    }
}
```

**Files to modify:**
- `src/tools/communication.rs` — Add chat file append after send
- `src/inbox/writer.rs` → `src/chats/writer.rs` — Rename and add `append_outgoing()`
- `src/inbox/format.rs` → `src/chats/format.rs` — Add `[>]` format support

### 2. Discord History Endpoint

Add `/read` endpoint to Discord adapter.

```rust
// crates/river-discord/src/outbound.rs

async fn read_messages(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ReadParams>,
) -> Result<Json<Vec<HistoryMessage>>, StatusCode> {
    let channel_id = params.channel.parse::<Id<ChannelMarker>>()?;
    let limit = params.limit.unwrap_or(50).min(100);

    let messages = state.http
        .channel_messages(channel_id)
        .limit(limit)?
        .await?
        .models()
        .await?;

    let history: Vec<HistoryMessage> = messages
        .into_iter()
        .map(|m| HistoryMessage {
            id: m.id.to_string(),
            author_id: m.author.id.to_string(),
            author_name: m.author.name.clone(),
            content: m.content.clone(),
            timestamp: m.timestamp.as_secs(),
            is_bot: m.author.bot,
        })
        .collect();

    Ok(Json(history))
}

// Add route
.route("/read", get(read_messages))
```

**Files to modify:**
- `crates/river-discord/src/outbound.rs` — Add `/read` endpoint
- `crates/river-discord/src/client.rs` — Add `fetch_messages()` wrapper

### 3. Sync Tool

New tool for agent to request history sync.

```rust
// src/tools/communication.rs

pub struct SyncChatTool { /* ... */ }

impl Tool for SyncChatTool {
    fn name(&self) -> &str { "sync_chat" }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "adapter": { "type": "string", "description": "Adapter name (e.g., 'discord')" },
                "channel": { "type": "string", "description": "Channel ID to sync" },
                "limit": { "type": "integer", "description": "Max messages to fetch (default: 50)" }
            },
            "required": ["adapter", "channel"]
        })
    }
}

impl SyncChatTool {
    async fn execute(&self, params: SyncChatParams) -> Result<Value> {
        let adapter = self.adapters.get(&params.adapter)?;
        let read_url = adapter.read_url.as_ref()
            .ok_or("Adapter doesn't support history fetch")?;

        // Fetch from adapter
        let url = format!("{}?channel={}&limit={}", read_url, params.channel, params.limit);
        let messages: Vec<HistoryMessage> = self.client.get(&url).send().await?.json().await?;

        // Load existing chat file
        let chat_path = build_chat_path(&self.workspace, &params.adapter, &params.channel);
        let existing = load_chat_file(&chat_path).await.unwrap_or_default();
        let existing_ids: HashSet<_> = existing.iter().map(|m| &m.id).collect();

        // Merge new messages (deduplicate)
        let mut merged = existing;
        for msg in messages {
            if !existing_ids.contains(&msg.id) {
                merged.push(msg);
            }
        }

        // Sort by timestamp/ID and write
        merged.sort_by_key(|m| m.id.clone());
        write_chat_file(&chat_path, &merged).await?;

        Ok(json!({
            "synced": messages.len(),
            "total": merged.len()
        }))
    }
}
```

### 4. Adapter Configuration

Update adapter config to include read URL.

```toml
# workspace config or CLI
[adapters.discord]
outbound_url = "http://localhost:8081/send"
read_url = "http://localhost:8081/read"      # NEW
```

Or via CLI flag:
```bash
--adapter "discord:http://localhost:8081/send:http://localhost:8081/read"
```

### 5. Rename Inbox → Chats

```rust
// src/lib.rs
pub mod chats;  // was: pub mod inbox

// src/chats/mod.rs
pub mod writer;
pub mod reader;
pub mod format;
pub mod sync;   // NEW
```

**Migration:** On startup, if `inbox/` exists and `chats/` doesn't, rename it.

---

## Open Design Questions

### When to sync?

| Option | Pros | Cons |
|--------|------|------|
| **On demand** (agent calls `sync_chat`) | Explicit, no surprise costs | Agent must remember to sync |
| **On wake** (always) | Always fresh | Rate limits, slow startup |
| **On wake if stale** (>N minutes since last sync) | Balance | Complexity |
| **Background service** | Always current | Extra process, complexity |

**Recommendation:** Start with on-demand (`sync_chat` tool). Agent can sync when it needs context.

### How much history?

| Option | Pros | Cons |
|--------|------|------|
| **Last N messages** (e.g., 50) | Bounded, fast | May miss context |
| **Since last sync** | Efficient | Need to track last sync time |
| **Since agent joined channel** | Complete | Could be huge |
| **All time** | Complete | Definitely huge |

**Recommendation:** Last 50 messages by default, configurable via tool param.

### File size management?

Options:
1. **No limit** — Let files grow, agent manages
2. **Rotate** — Archive old messages to `{channel}.1.txt`, `{channel}.2.txt`
3. **Truncate** — Keep only last N messages in file
4. **Summarize** — Replace old messages with summary (like context rotation)

**Recommendation:** Start with no limit. Add rotation later if needed.

### Database integration?

The `messages` table stores conversation history. Options:

1. **Files are view, DB is truth** — Sync from DB, files are cache
2. **Files are truth, DB is backup** — Write to files, periodically backup to DB
3. **Both are independent** — Files for external chats, DB for agent's internal context

**Recommendation:** Files are source of truth for external chats. Database stores agent's thinking/context history separately.

---

## Testing

```rust
#[tokio::test]
async fn test_outgoing_message_appended_to_chat() {
    let workspace = TempDir::new().unwrap();
    let tool = SendMessageTool::new(&workspace, mock_adapter());

    tool.execute(json!({
        "adapter": "discord",
        "channel": "123",
        "content": "Hello!"
    })).await.unwrap();

    let chat = read_to_string(workspace.join("chats/discord/123.txt")).unwrap();
    assert!(chat.contains("[>]"));
    assert!(chat.contains("Hello!"));
}

#[tokio::test]
async fn test_sync_deduplicates() {
    let workspace = TempDir::new().unwrap();
    // Pre-populate with one message
    write(&workspace.join("chats/discord/123.txt"), "[x] ... msg1 ...").unwrap();

    let tool = SyncChatTool::new(&workspace, mock_adapter_returns(vec![msg1, msg2]));
    let result = tool.execute(json!({
        "adapter": "discord",
        "channel": "123"
    })).await.unwrap();

    assert_eq!(result["synced"], 2);  // Fetched 2
    assert_eq!(result["total"], 2);   // But only 2 total (msg1 deduplicated)
}

#[tokio::test]
async fn test_format_outgoing_line() {
    let line = format_outgoing_line("123", "river", "999", "Hello world");
    assert!(line.starts_with("[>]"));
    assert!(line.contains("<river:999>"));
    assert!(line.contains("Hello world"));
}
```

---

## Files to Create/Modify

| File | Change |
|------|--------|
| `src/inbox/` → `src/chats/` | Rename module |
| `src/chats/format.rs` | Add `[>]` outgoing format |
| `src/chats/writer.rs` | Add `append_outgoing()` |
| `src/chats/sync.rs` | **New** — Sync logic |
| `src/tools/communication.rs` | Append to chat on send, add `sync_chat` tool |
| `src/server.rs` | Register `sync_chat` tool |
| `crates/river-discord/src/outbound.rs` | Add `/read` endpoint |
| `crates/river-discord/src/client.rs` | Add `fetch_messages()` |

---

## Migration

1. **Rename folder:** `inbox/` → `chats/` (on startup if needed)
2. **Existing files:** Keep as-is, all `[ ]` and `[x]` markers still valid
3. **New outgoing:** Appended with `[>]` marker
4. **Backfill:** Agent can call `sync_chat` to pull history and fill gaps

---

## Future Enhancements

| Feature | Description |
|---------|-------------|
| **Reactions** | Track emoji reactions in chat file |
| **Edits** | Handle message edits (update line or append edit marker) |
| **Deletions** | Handle message deletions (mark as deleted or remove) |
| **Threads** | Sub-files for thread conversations |
| **Attachments** | Store attachment URLs/metadata |
| **Auto-sync** | Background sync service |
| **Compression** | Gzip old chat files |

---

## Summary

Transform the one-directional inbox into bidirectional chat files:

```
Before:                          After:
inbox/discord/ch.txt             chats/discord/ch.txt
  [ ] alice: hi                    [ ] alice: hi
  [ ] alice: you there?            [>] river: Hello!
                                   [x] alice: you there?
                                   [>] river: Yes, how can I help?
```

The agent can see both sides of the conversation, sync history from Discord when needed, and maintain continuity across context rotations.

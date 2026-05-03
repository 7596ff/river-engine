# Read History Design Spec

> Authors: Cass, Claude
> Date: 2026-04-04

## Overview

Add a `read_history` tool that lets the LLM fetch message history from the platform (Discord/Slack). Fetched messages are written to conversation files using existing logic, with compaction handling deduplication.

**Goals:**
- LLM can fetch past messages from channels
- Messages persist to conversation files for future reference
- Pagination support for large histories
- Feature-gated (only available if adapter supports it)

**Non-goals:**
- Startup/recovery auto-fetch (push via `/notify` handles ongoing messages)
- Local-only history lookup (this is platform fetch)

## Part 1: Adapter Protocol Changes

### Add `after` Parameter

Update `OutboundRequest::ReadHistory` in `river-adapter/src/feature.rs`:

```rust
ReadHistory {
    channel: String,
    limit: Option<u32>,
    before: Option<String>,  // messages before this ID (existing)
    after: Option<String>,   // messages after this ID (new)
}
```

Both `before` and `after` are optional and mutually exclusive (provide at most one). If neither provided, fetch most recent messages. If both provided, adapter returns error.

### Response Format

Adapter returns messages in standard format:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadHistoryResponse {
    pub messages: Vec<HistoryMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub channel: String,
    pub author: Author,
    pub content: String,
    pub message_id: String,
    pub timestamp: String,
    pub reply_to: Option<String>,
    pub attachments: Vec<Attachment>,
}
```

Same fields as `EventMetadata::MessageCreate` for consistency.

## Part 2: Worker Tool

### Tool Definition

New tool `read_history`, gated by `FeatureId::ReadHistory`:

```json
{
  "name": "read_history",
  "description": "Fetch message history from a channel. Messages are saved to the conversation file.",
  "parameters": {
    "type": "object",
    "properties": {
      "channel": {
        "type": "string",
        "description": "Channel ID to fetch from"
      },
      "adapter": {
        "type": "string",
        "description": "Adapter name (e.g., 'discord')"
      },
      "limit": {
        "type": "integer",
        "description": "Max messages to fetch (1-100, default 50)"
      },
      "before": {
        "type": "string",
        "description": "Fetch messages before this message ID"
      },
      "after": {
        "type": "string",
        "description": "Fetch messages after this message ID"
      }
    },
    "required": ["channel", "adapter"]
  }
}
```

### Tool Result

```rust
#[derive(Debug, Serialize)]
pub struct ReadHistoryResult {
    pub success: bool,
    pub messages_fetched: usize,
    pub oldest_id: Option<String>,    // for pagination: "before" this for older
    pub newest_id: Option<String>,    // for pagination: "after" this for newer
    pub error: Option<String>,
    pub retry_after_ms: Option<u64>,  // rate limit backoff hint
}
```

Example success:
```json
{
  "success": true,
  "messages_fetched": 50,
  "oldest_id": "1234567890",
  "newest_id": "1234567940"
}
```

Example failure:
```json
{
  "success": false,
  "messages_fetched": 0,
  "oldest_id": null,
  "newest_id": null,
  "error": "Adapter does not support ReadHistory"
}
```

Example rate limited:
```json
{
  "success": false,
  "messages_fetched": 0,
  "oldest_id": null,
  "newest_id": null,
  "error": "Rate limited, retry after 2.5 seconds"
}
```

### Rate Limit Handling

When rate limited:
- `success: false`
- `retry_after_ms` populated with milliseconds to wait
- LLM can decide to wait and retry, or give up

Discord returns `Retry-After` header or `retry_after` field in 429 responses. Adapter should parse and forward this.

### Feature Gating

Tool only appears in tool list if:
1. Worker has adapter registered for the specified adapter type
2. That adapter reported `FeatureId::ReadHistory` in its features

Check in tool execution:
```rust
if !adapter_supports_feature(&registry, adapter, FeatureId::ReadHistory) {
    return ReadHistoryResult {
        success: false,
        error: Some("Adapter does not support ReadHistory".into()),
        ..Default::default()
    };
}
```

## Part 3: Message Processing

### Write to Conversation File

For each message returned by adapter:

```rust
let msg = Message {
    direction: MessageDirection::Unread,
    timestamp: history_msg.timestamp.clone(),
    id: history_msg.message_id.clone(),
    author: ProtocolAuthor {
        name: history_msg.author.name.clone(),
        id: history_msg.author.id.clone(),
        bot: history_msg.author.bot,
    },
    content: history_msg.content.clone(),
    reactions: vec![],
};

let path = conversation_path_for_channel(&workspace, &channel);
Conversation::append_line(&path, &Line::Message(msg))?;
```

Uses existing conversation file logic - same as `/notify` handler.

### Compaction Dedupe

Update `Conversation::compact()` to dedupe by message ID:

```rust
pub fn compact(&mut self) {
    // 1. Collect read receipt message IDs
    let read_ids: HashSet<String> = ...;

    // 2. Filter to messages, apply read status, DEDUPE BY ID
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut messages: Vec<Message> = self
        .lines
        .iter()
        .filter_map(|line| match line {
            Line::Message(msg) => {
                // Skip if we've seen this ID
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

    // 4. Replace lines
    self.lines = messages.into_iter().map(Line::Message).collect();
}
```

First occurrence of each ID wins (keeps original message, drops duplicates from re-fetch).

## Part 4: Data Flow

```
LLM calls read_history(channel: "general", adapter: "discord", limit: 50)
    │
    ▼
Worker checks registry for discord adapter with ReadHistory feature
    │
    ▼
Worker sends POST {adapter}/execute
    body: { "read_history": { channel, limit, before, after } }
    │
    ▼
Adapter calls Discord API: GET /channels/{id}/messages?limit=50
    │
    ▼
Adapter returns ReadHistoryResponse { messages: [...] }
    │
    ▼
Worker appends each message to conversation file
    │
    ▼
Worker returns ReadHistoryResult { success: true, messages_fetched: 50, oldest_id, newest_id }
    │
    ▼
LLM can chain: read_history(before: oldest_id) to get older messages
```

## Part 5: Adapter Implementation (Discord)

### Endpoint Handler

In `river-discord`, handle `ReadHistory` in execute endpoint:

```rust
OutboundRequest::ReadHistory { channel, limit, before, after } => {
    let limit = limit.unwrap_or(50).min(100);

    let mut request = client.channel_messages(channel_id);

    if let Some(before_id) = before {
        request = request.before(before_id.parse()?);
    }
    if let Some(after_id) = after {
        request = request.after(after_id.parse()?);
    }
    request = request.limit(limit as u16);

    let messages = request.await?;

    let history_messages: Vec<HistoryMessage> = messages
        .iter()
        .map(|m| HistoryMessage {
            channel: channel.clone(),
            author: Author {
                id: m.author.id.to_string(),
                name: m.author.name.clone(),
                bot: m.author.bot,
            },
            content: m.content.clone(),
            message_id: m.id.to_string(),
            timestamp: m.timestamp.to_string(),
            reply_to: m.reference.as_ref().map(|r| r.message_id.map(|id| id.to_string())).flatten(),
            attachments: vec![], // TODO: map attachments
        })
        .collect();

    Ok(ReadHistoryResponse { messages: history_messages })
}
```

## Testing

### Unit Tests

**Compaction dedupe:**
- Duplicate message IDs → only first kept
- Duplicate IDs with different content → first wins
- Mixed duplicates and unique → correct filtering

**Tool gating:**
- Adapter without ReadHistory feature → tool returns error
- Adapter with feature → tool executes

### Integration Tests

- Fetch 50 messages → 50 written to file
- Fetch with `before` → older messages returned
- Fetch with `after` → newer messages returned
- Re-fetch same messages → dedupe on compact
- Pagination chain → can walk full history

## File Changes

| File | Change |
|------|--------|
| `river-adapter/src/feature.rs` | Add `after` to `ReadHistory`, add response types |
| `river-protocol/src/conversation/mod.rs` | Add dedupe to `compact()` |
| `river-worker/src/tools.rs` | Add `read_history` tool |
| `river-discord/src/execute.rs` | Handle `ReadHistory` request |

## Cleanup

Remove unused `since_id` field from `Notification` struct in `river-worker/src/state.rs`. It was intended for startup catchup which we're not implementing - this on-demand tool replaces that use case.

## Migration

None required. New feature, additive changes only.

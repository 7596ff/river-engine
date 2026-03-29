# Utterances Design Spec

> Channel-aware speech for agents
>
> Date: 2026-03-28
> Authors: Cass, Claude

---

## 1. Summary

Add a `speak` tool that sends messages to the agent's current channel, and a `switch_channel` tool for explicit channel navigation. This makes speech a deliberate act while simplifying the common case of responding in context.

**Philosophy:** "The agent thinks, then utters."

---

## 2. Tools

### `speak`

Send a message to the current channel.

```json
{
  "type": "object",
  "properties": {
    "content": {
      "type": "string",
      "description": "Message content to send"
    },
    "reply_to": {
      "type": "string",
      "description": "Optional message ID to reply to"
    }
  },
  "required": ["content"]
}
```

**Behavior:**
1. Check `channel_context` is set (error if not)
2. Look up adapter in registry by name
3. POST to adapter's outbound URL with `channel_id` and `content`
4. Record outgoing message to conversation file
5. Return success/failure

**Errors:**
- `"No channel selected. Use switch_channel first."` - no context set
- `"Adapter '{name}' not registered"` - adapter missing
- `"Failed to send: {error}"` - outbound failed

### `switch_channel`

Set the agent's current channel.

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Path to conversation file (e.g., 'conversations/discord/myserver/general.txt')"
    }
  },
  "required": ["path"]
}
```

**Behavior:**
1. Resolve path relative to workspace
2. Read conversation file, parse frontmatter
3. Validate adapter exists in registry
4. Update `channel_context` in AgentTask
5. Emit `ChannelSwitched` event
6. Return success with channel name

**Errors:**
- `"Conversation file not found: {path}"` - file missing
- `"Conversation file missing routing metadata"` - no frontmatter
- `"Conversation file missing adapter or channel_id"` - required fields absent

### Relationship to `send_message`

| Tool | Use Case |
|------|----------|
| `speak` | Contextual - uses current channel |
| `send_message` | Explicit - requires adapter + channel params |

Both share underlying send logic to avoid duplication.

---

## 3. Conversation File Format

Add YAML frontmatter to conversation files with routing metadata:

```
---
adapter: discord
channel_id: "789012345678901234"
channel_name: general
guild_id: "123456789012345678"
guild_name: myserver
thread_id: null
---
[ ] 2026-03-23 14:30:00 | msg123 | alice (111) | hey, can you help?
[>] 2026-03-23 14:30:15 | msg124 | river (999) | Sure!
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `adapter` | Yes | Adapter name for registry lookup |
| `channel_id` | Yes | Platform-specific channel identifier |
| `channel_name` | No | Human-readable name |
| `guild_id` | No | Server/workspace ID if applicable |
| `guild_name` | No | Human-readable server name |
| `thread_id` | No | Thread ID if this is a thread conversation |

### ConversationMeta

New struct for parsed frontmatter:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub adapter: String,
    pub channel_id: String,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub guild_name: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
}
```

### Changes to Conversation

```rust
pub struct Conversation {
    pub meta: Option<ConversationMeta>,  // NEW
    pub messages: Vec<Message>,
}
```

- `from_str` parses frontmatter if present
- `to_string` emits frontmatter if `meta` is `Some`
- Conversation writer populates meta on first message from incoming

---

## 4. ChannelContext

Cached routing context stored in AgentTask:

```rust
/// Cached routing context for the current channel
#[derive(Debug, Clone)]
pub struct ChannelContext {
    /// Path to conversation file (relative to workspace)
    pub path: PathBuf,
    /// Adapter name (for registry lookup)
    pub adapter: String,
    /// Platform channel ID (for outbound messages)
    pub channel_id: String,
    /// Human-readable channel name (for logging/display)
    pub channel_name: Option<String>,
    /// Guild/server ID if applicable
    pub guild_id: Option<String>,
}

impl ChannelContext {
    /// Parse from conversation file
    pub fn from_conversation(path: PathBuf, meta: &ConversationMeta) -> Self;
}
```

### Integration with AgentTask

```rust
pub struct AgentTask {
    // ... existing fields ...
    channel_context: Option<ChannelContext>,  // replaces current_channel: String
}
```

**Behavior:**
- Starts as `None`
- Set by `switch_channel` tool
- Read by `speak` tool
- Only mutated by `switch_channel` (no cache invalidation needed)

---

## 5. Shared Send Logic

Extract common send logic used by both `speak` and `send_message`:

```rust
/// Send a message through an adapter
async fn send_to_adapter(
    http_client: &reqwest::Client,
    registry: &AdapterRegistry,
    adapter: &str,
    channel_id: &str,
    content: &str,
    reply_to: Option<&str>,
    writer_tx: &mpsc::Sender<WriteOp>,
    conversation_path: &Path,
    agent_author: Author,
) -> Result<ToolResult, RiverError>
```

- `send_message`: extracts adapter/channel from tool args, builds conversation path
- `speak`: gets adapter/channel from `channel_context`, uses cached path

---

## 6. Event Integration

The existing `ChannelSwitched` event works as-is:

```rust
AgentEvent::ChannelSwitched {
    from: String,
    to: String,
    timestamp: DateTime<Utc>,
}
```

`switch_channel` emits this event. Spectator can observe channel focus changes.

Future consideration: `AgentEvent::Utterance` for spectator to track all speech. Not needed for v1.

---

## 7. File Structure

| File | Changes |
|------|---------|
| `crates/river-gateway/src/conversations/mod.rs` | Add `ConversationMeta`, update `Conversation` |
| `crates/river-gateway/src/conversations/format.rs` | Parse/emit frontmatter |
| `crates/river-gateway/src/agent/channel.rs` | New: `ChannelContext` |
| `crates/river-gateway/src/agent/task.rs` | Replace `current_channel` with `channel_context` |
| `crates/river-gateway/src/tools/communication.rs` | Add `speak`, `switch_channel`, extract shared send logic |

---

## 8. Testing

### Unit Tests

- `ConversationMeta` parsing from frontmatter
- `ChannelContext::from_conversation`
- `speak` tool schema validation
- `switch_channel` tool schema validation
- Conversation roundtrip with frontmatter

### Integration Tests

- Switch to channel → speak → verify message in conversation file
- Speak without switching → error
- Switch to nonexistent file → error
- Switch to file without frontmatter → error

---

## 9. Migration

Not applicable. This is a greenfield system - all conversation files will have frontmatter from creation. No legacy format support needed.

---

## 10. Summary

| Component | Description |
|-----------|-------------|
| `speak` tool | Send to current channel |
| `switch_channel` tool | Set current channel by path |
| `ConversationMeta` | Frontmatter struct with routing info |
| `ChannelContext` | Cached routing context in AgentTask |
| Shared send logic | Common implementation for both tools |

The agent thinks, then utters. Speech is deliberate.

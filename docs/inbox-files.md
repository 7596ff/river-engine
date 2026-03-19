# Inbox Files

File-based message delivery system for river-engine. Messages are appended to text files in the workspace, and the agent is notified of files with new messages.

## Philosophy

**Everything is a file.** The workspace is the agent's world. Rather than receiving messages through transient channels, messages persist as files the agent can read, edit, and reference. This aligns with Unix philosophy and creates a consistent file-first interaction model.

## Goals

- **Persistence**: Messages survive gateway restarts before being processed
- **Observability**: Inspect pending messages with `cat`, `tail -f`, `grep`
- **Agent agency**: Agent decides when/if to process messages by reading/editing files
- **Human readable**: Plain text format, not JSON

---

## Directory Structure

```
workspace/
└── inbox/
    └── {adapter}/
        └── {hierarchy}/
            └── {channel}.txt
```

### Discord Layout

```
inbox/
└── discord/
    ├── 123456-myserver/
    │   ├── 789012-general.txt
    │   └── 789013-random.txt
    └── dm/
        └── 111222-alice.txt
```

**Guild messages:** `inbox/discord/{guildId}-{guildName}/{channelId}-{channelName}.txt`

**DMs:** `inbox/discord/dm/{channelId}-{userName}.txt`

### Other Adapters

Each adapter defines its own hierarchy:

```
inbox/irc/liberachat/river-engine.txt
inbox/matrix/!abc123-myroom.txt
inbox/slack/workspace-name/channel-name.txt
```

---

## Message Format

### Line Structure

```
[status] timestamp messageId <authorName:authorId> content
```

### Fields

| Field | Format | Description |
|-------|--------|-------------|
| status | `[ ]` or `[x]` | Unread or read |
| timestamp | `YYYY-MM-DD HH:MM:SS` | UTC time |
| messageId | string | Platform message ID |
| author | `<name:id>` | Author name and platform ID |
| content | text | Message content (escaped) |

### Examples

```
[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there
[ ] 2026-03-18 22:15:45 def456 <bob:987654321> hey alice\nhow are you?
[x] 2026-03-18 22:16:01 ghi789 <alice:123456789> just working on river-engine
```

### Content Escaping

Content is escaped to ensure one message per line:

| Character | Escaped As |
|-----------|------------|
| Backslash (`\`) | `\\` |
| Newline (`\n`) | `\n` |
| Carriage return (`\r`) | `\r` |

When parsing, unescape in reverse order: `\\` → `\`, then `\n` → newline, `\r` → CR.

### Marking as Read

The agent marks messages as read by editing `[ ]` to `[x]`:

```diff
-[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there
+[x] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there
```

---

## Message Flow

```
Adapter → POST /incoming → Gateway writes to inbox file
                        → LoopEvent::InboxUpdate(paths) → Agent notified
                                                        → Agent reads files
                                                        → Agent marks [x]
```

### Sequence

1. Adapter receives message from platform
2. Adapter POSTs to gateway `/incoming` endpoint
3. Gateway determines inbox file path from message metadata
4. Gateway appends formatted line to inbox file (creates dirs/file if needed)
5. Gateway sends `LoopEvent::InboxUpdate(vec![path])` to agent loop
6. Agent wakes, receives system message with list of files
7. Agent reads file(s), finds `[ ]` lines, processes them
8. Agent edits file to mark `[x]` on processed messages

---

## Path Sanitization

User-controlled names (guild names, channel names, usernames) are sanitized:

1. Replace path separators (`/`, `\`) with `_`
2. Replace null bytes with `_`
3. Limit to 50 characters (truncate)
4. Preserve Unicode (NFC normalized)

Example: `my/guild` → `my_guild`, `café` → `café`

---

## Code Structure

### New Files

| File | Responsibility |
|------|----------------|
| `crates/river-gateway/src/inbox/mod.rs` | Module exports |
| `crates/river-gateway/src/inbox/format.rs` | Line formatting, parsing, escaping |
| `crates/river-gateway/src/inbox/writer.rs` | Path building, file creation, appending |
| `crates/river-gateway/src/inbox/reader.rs` | Reading and parsing inbox files |

### Modified Files

| File | Changes |
|------|---------|
| `crates/river-gateway/src/loop/state.rs` | Added `LoopEvent::InboxUpdate`, `WakeTrigger::Inbox` |
| `crates/river-gateway/src/api/routes.rs` | `/incoming` writes to inbox, sends `InboxUpdate` |
| `crates/river-gateway/src/loop/mod.rs` | Handle `InboxUpdate` in sleep, `Inbox` in wake |
| `crates/river-discord/src/gateway.rs` | Added `channel_name`, `guild_id`, `guild_name` fields |
| `crates/river-discord/src/handler.rs` | Populate new fields (None for now, caching TODO) |

---

## API

### Inbox Format Functions

```rust
// Escape content for single-line storage
pub fn escape_content(content: &str) -> String;

// Unescape content from storage format
pub fn unescape_content(content: &str) -> String;

// Format a message as an inbox line (always unread)
pub fn format_inbox_line(
    timestamp: &str,
    message_id: &str,
    author_name: &str,
    author_id: &str,
    content: &str,
) -> String;

// Parse an inbox line into its components
pub fn parse_inbox_line(line: &str, line_number: usize) -> Option<InboxMessage>;
```

### Inbox Writer Functions

```rust
// Sanitize user-provided name for filesystem safety
pub fn sanitize_name(name: &str) -> String;

// Build inbox path for Discord message
pub fn build_discord_path(
    workspace: &Path,
    guild_id: Option<&str>,
    guild_name: Option<&str>,
    channel_id: &str,
    channel_name: &str,
) -> PathBuf;

// Append a line to an inbox file (creates dirs if needed)
pub fn append_line(path: &Path, line: &str) -> RiverResult<()>;
```

### Inbox Reader Functions

```rust
// Read all messages from an inbox file
pub fn read_all_messages(path: &Path) -> RiverResult<Vec<InboxMessage>>;

// Read only unread messages
pub fn read_unread_messages(path: &Path) -> RiverResult<Vec<InboxMessage>>;

// Check if file has any unread messages (efficient)
pub fn has_unread_messages(path: &Path) -> RiverResult<bool>;
```

### InboxMessage Struct

```rust
pub struct InboxMessage {
    pub read: bool,
    pub timestamp: String,
    pub message_id: String,
    pub author_name: String,
    pub author_id: String,
    pub content: String,
    pub line_number: usize,
}
```

---

## Event Types

### LoopEvent

```rust
pub enum LoopEvent {
    /// Message from adapter (DEPRECATED - use InboxUpdate)
    Message(IncomingMessage),
    /// New messages written to inbox files
    InboxUpdate(Vec<PathBuf>),
    /// Heartbeat timer fired
    Heartbeat,
    /// Graceful shutdown requested
    Shutdown,
}
```

### WakeTrigger

```rust
pub enum WakeTrigger {
    /// Direct message (DEPRECATED - use Inbox)
    Message(IncomingMessage),
    /// Inbox files with new messages
    Inbox(Vec<PathBuf>),
    /// Scheduled heartbeat
    Heartbeat,
}
```

---

## Concurrency

### Gateway Writes, Agent Reads/Edits

Safe because:

1. **Append-only writes**: Gateway only appends, never modifies existing lines
2. **POSIX O_APPEND**: Atomic append on POSIX systems
3. **Agent edits in-place**: Agent changes `[ ]` → `[x]` at fixed positions
4. **Read-before-edit**: Agent reads file, identifies unread lines, then edits

### Potential Race

If gateway appends while agent is editing:
- Agent's edit completes on existing content
- New line appears at end
- Agent sees new line on next wake

This is acceptable: new messages aren't lost, just processed on next cycle.

### Single Gateway Assumption

Only one gateway process writes to inbox files. Multiple gateways would require file locking or separate inbox directories.

---

## Error Handling

| Scenario | Resolution |
|----------|------------|
| Inbox dir creation fails | Log error, return 500 to adapter |
| File append fails | Log error, return 500 to adapter |
| Malformed line in inbox | Skip line, log warning |
| File read fails | Log error, continue with other files |

---

## Migration

The current implementation uses a dual-path approach:

1. **Phase 1 (current)**: Gateway writes to inbox AND sends event
   - `InboxUpdate` event notifies agent of files
   - Agent can use tools to read inbox files
   - Legacy `Message` event deprecated but still in types

2. **Phase 2 (future)**: Remove legacy path
   - Remove `LoopEvent::Message` handling
   - Remove `WakeTrigger::Message`
   - Adapters only need to POST, gateway handles filing

---

## Testing

### Unit Tests

| Module | Test Count | Coverage |
|--------|------------|----------|
| `inbox::format` | 11 | Escaping, parsing, roundtrip |
| `inbox::writer` | 11 | Sanitization, paths, appending |
| `inbox::reader` | 7 | Reading, filtering, malformed handling |
| `loop::state` | 14 | Event/trigger variants, state behavior |

### Integration

All 250+ workspace tests pass. Release build succeeds with no warnings.

---

## Future Considerations

- **Archival**: Move old messages to `inbox/archive/` periodically
- **Adapter library**: Abstract inbox format for adapter implementers
- **Watch mode**: Optional inotify/fswatch for faster notification
- **Outbox**: Similar pattern for outgoing messages (`outbox/discord/...`)
- **Channel name caching**: Discord adapter caches names from gateway events

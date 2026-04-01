# Inbox Files Design

## Overview

Replace direct message delivery with file-based inbox. Messages are appended to text files in the workspace, and the agent is notified of new files to check. The agent marks messages as read by editing the file.

## Philosophy

**Everything is a file.** The workspace is the agent's world. Rather than receiving messages through transient channels, messages persist as files the agent can read, edit, and reference. This aligns with Unix philosophy and creates a consistent file-first interaction model.

## Goals

- **Persistence**: Messages survive gateway restarts before being processed
- **Observability**: Inspect pending messages with `cat`, `tail -f`, `grep`
- **Agent agency**: Agent decides when/if to process messages by editing files
- **Human readable**: Plain text format, not JSON

## Inbox Structure

### Directory Layout

```
workspace/
└── inbox/
    └── {adapter}/
        └── {hierarchy}/
            └── {channel}.txt
```

### Discord Adapter

```
inbox/
└── discord/
    ├── 123456-myserver/
    │   ├── 789012-general.txt
    │   └── 789013-random.txt
    └── dm/
        └── 111222-alice.txt
```

**Path format**: `inbox/discord/{guildId}-{guildName}/{channelId}-{channelName}.txt`

**DMs**: `inbox/discord/dm/{userId}-{userName}.txt`

**Name resolution**: Guild and channel names come from the Discord event context. The adapter caches guild/channel info from Discord's gateway events. If a name is unavailable, use `unknown`.

### Path Sanitization

User-controlled names (guild names, channel names, usernames) are sanitized:

1. Replace path separators (`/`, `\`) with `_`
2. Replace null bytes with `_`
3. Limit to 50 characters (truncate with `...` suffix)
4. Preserve Unicode but normalize to NFC

Example: `my/guild` → `my_guild`, `café` → `café` (NFC normalized)

### Other Adapters

Each adapter defines its own hierarchy. Examples:

```
inbox/irc/liberachat/river-engine.txt
inbox/matrix/!abc123-myroom.txt
inbox/slack/workspace-name/channel-name.txt
```

Adapter library abstraction will formalize this later.

## Message Format

### Structure

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
| content | text | Message content with escaping (see below) |

### Content Escaping

Content is escaped to ensure one message per line:

| Character | Escaped As |
|-----------|------------|
| Newline (`\n`) | `\\n` |
| Carriage return (`\r`) | `\\r` |
| Backslash (`\`) | `\\` |

Example: `hello\nworld` in source becomes `hello\\nworld` in file.

When parsing, unescape in reverse order: `\\` → `\`, then `\\n` → newline, `\\r` → CR.

### Examples

```
[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there
[ ] 2026-03-18 22:15:45 def456 <bob:987654321> hey alice\nhow are you?
[x] 2026-03-18 22:16:01 ghi789 <alice:123456789> just working on river-engine
```

### Marking as Read

The agent marks messages as read by editing `[ ]` to `[x]`:

```diff
-[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there
+[x] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there
```

This creates a clear audit trail and the agent can re-read old messages if needed.

## Message Flow

### Current Flow (to be replaced)

```
Adapter → POST /incoming → LoopEvent::Message(msg) → Agent processes
```

### New Flow

```
Adapter → POST /incoming → Gateway writes to inbox file
                        → LoopEvent::InboxUpdate(paths) → Agent reads files
                                                        → Agent marks [x]
```

### Sequence

1. Adapter receives message from platform
2. Adapter POSTs to gateway `/incoming` endpoint
3. Gateway determines inbox file path from message metadata
4. Gateway appends formatted line to inbox file (creates dirs/file if needed)
5. Gateway sends `LoopEvent::InboxUpdate(vec![path])` to agent loop
6. Agent wakes, receives list of files with new messages
7. Agent reads file(s), finds `[ ]` lines, processes them
8. Agent edits file to mark `[x]` on processed messages

### Batching

If multiple messages arrive rapidly, gateway can batch:
- Append all to respective files
- Send single `InboxUpdate` with list of all affected files

## Code Changes

### New Types

```rust
// In loop/state.rs
pub enum LoopEvent {
    Message(IncomingMessage),  // DEPRECATED - remove later
    InboxUpdate(Vec<PathBuf>), // NEW - files with new messages
    Heartbeat,
    Shutdown,
}

pub enum WakeTrigger {
    Message(IncomingMessage),  // DEPRECATED
    Inbox(Vec<PathBuf>),       // NEW - inbox files to process
    Heartbeat,
}
```

### Gateway Changes

**File**: `crates/river-gateway/src/api/routes.rs`

```rust
async fn handle_incoming(
    State(state): State<Arc<AppState>>,
    Json(msg): Json<IncomingMessage>,
) -> impl IntoResponse {
    // Determine inbox path
    let inbox_path = build_inbox_path(&state.config.workspace, &msg);

    // Format and append message
    let line = format_inbox_line(&msg);
    append_to_inbox(&inbox_path, &line)?;

    // Notify loop
    state.loop_tx.send(LoopEvent::InboxUpdate(vec![inbox_path])).await?;

    // ...
}
```

### New Module

**File**: `crates/river-gateway/src/inbox/mod.rs`

```rust
pub struct InboxWriter {
    workspace: PathBuf,
}

impl InboxWriter {
    pub fn write_message(&self, msg: &IncomingMessage) -> RiverResult<PathBuf>;
    pub fn format_line(msg: &IncomingMessage) -> String;
    pub fn build_path(&self, msg: &IncomingMessage) -> PathBuf;
}
```

### Loop Changes

**File**: `crates/river-gateway/src/loop/mod.rs`

Update `sleep_phase` to handle `InboxUpdate`:

```rust
LoopEvent::InboxUpdate(paths) => {
    tracing::info!(file_count = paths.len(), "Inbox update received");
    self.state = LoopState::Waking {
        trigger: WakeTrigger::Inbox(paths),
    };
}
```

Update `wake_phase` to read inbox files:

```rust
WakeTrigger::Inbox(paths) => {
    for path in paths {
        let unread = self.read_unread_messages(&path)?;
        for msg in unread {
            let chat_msg = ChatMessage::user(format!(
                "[{}] {} {}",
                path.display(), msg.author, msg.content
            ));
            self.context.add_message(chat_msg);
        }
    }
}
```

### Discord Adapter Changes

**File**: `crates/river-discord/src/gateway.rs`

Update `IncomingEvent` to include guild/channel names for path building:

```rust
pub struct IncomingEvent {
    pub adapter: String,
    pub channel_id: String,
    pub channel_name: String,      // NEW
    pub guild_id: Option<String>,
    pub guild_name: Option<String>, // NEW
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub message_id: String,
    // ...
}
```

## File Operations

### Creating Inbox Directories

Gateway creates directories on first message:

```rust
fn ensure_inbox_dir(path: &Path) -> RiverResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
```

### Appending Messages

Use append mode with file locking for concurrent safety:

```rust
fn append_to_inbox(path: &Path, line: &str) -> RiverResult<()> {
    ensure_inbox_dir(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}
```

### Reading Unread Messages

```rust
struct InboxMessage {
    read: bool,
    timestamp: String,
    message_id: String,
    author_name: String,
    author_id: String,
    content: String,
    line_number: usize,
}

fn read_unread_messages(path: &Path) -> RiverResult<Vec<InboxMessage>> {
    let content = std::fs::read_to_string(path)?;
    content.lines()
        .enumerate()
        .filter_map(|(i, line)| parse_inbox_line(line, i))
        .filter(|msg| !msg.read)
        .collect()
}
```

### Marking as Read

Agent uses the `edit` tool to change `[ ]` to `[x]`:

```rust
// Agent calls edit tool with:
// old_string: "[ ] 2026-03-18 22:15:32 abc123"
// new_string: "[x] 2026-03-18 22:15:32 abc123"
```

## Concurrency

### Gateway writes, Agent reads/edits

The gateway appends new messages while the agent reads and edits the same file. This is safe because:

1. **Append-only writes**: Gateway only appends, never modifies existing lines
2. **POSIX O_APPEND**: Atomic append on POSIX systems
3. **Agent edits in-place**: Agent changes `[ ]` → `[x]` at fixed positions, doesn't move lines
4. **Read-before-edit**: Agent reads file, identifies unread lines, then edits specific positions

### Potential race

If gateway appends while agent is editing:
- Agent's edit completes on existing content
- New line appears at end
- Agent sees new line on next wake

This is acceptable: new messages aren't lost, just processed on next cycle.

### Single gateway assumption

Only one gateway process writes to inbox files. Multiple gateways would require file locking or separate inbox directories per gateway.

## Error Handling

| Scenario | Resolution |
|----------|------------|
| Inbox dir creation fails | Log error, return 500 to adapter |
| File append fails | Log error, return 500 to adapter |
| Malformed line in inbox | Skip line, log warning |
| File read fails | Log error, continue with other files |

## Migration Path

1. **Phase 1**: Implement inbox writing alongside existing flow
   - Gateway writes to inbox AND sends LoopEvent::Message
   - Agent continues using Message events

2. **Phase 2**: Switch agent to read from inbox
   - Agent processes InboxUpdate events
   - LoopEvent::Message deprecated but still functional

3. **Phase 3**: Remove legacy path
   - Remove LoopEvent::Message handling
   - Adapters only need to POST, gateway handles filing

## Testing Strategy

### Unit Tests

| Module | Tests |
|--------|-------|
| `inbox/format.rs` | Line formatting, parsing, escaping |
| `inbox/writer.rs` | Path building, file creation, appending |
| `inbox/reader.rs` | Reading unread, parsing lines |

### Integration Tests

| Scenario | Verification |
|----------|--------------|
| Message arrives | File created, line appended |
| Multiple messages | Lines in correct order |
| Agent marks read | `[x]` persisted |
| Restart after write | Messages still in file |
| Concurrent writes | No corruption |

## Future Considerations

- **Archival**: Move old messages to `inbox/archive/` periodically
- **Adapter library**: Abstract inbox format for adapter implementers
- **Watch mode**: Optional inotify/fswatch for faster notification
- **Outbox**: Similar pattern for outgoing messages (`outbox/discord/...`)

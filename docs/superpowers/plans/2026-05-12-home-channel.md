# Home Channel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the agent's invisible internal context with a visible, append-only home channel log that becomes the single source of truth for all agent activity.

**Architecture:** The home channel is an append-only JSONL log at `channels/home/{agent_name}.jsonl`. All agent output, tool calls, incoming messages, and heartbeats are written to it through a serialized log writer actor. The model context is built ephemerally by reading the log tail plus spectator moves (stored as files, not SQL). Per-adapter channel logs remain as secondary projections. SQL message storage is eliminated.

**Tech Stack:** Rust, tokio (async), serde/serde_json (serialization), JSONL (storage)

**Serde strategy:** The home channel uses `#[serde(tag = "type")]` for entry discrimination, NOT `#[serde(untagged)]`. This is a new file format at a new path (`channels/home/`), so no migration of existing JSONL files is needed.

---

### Task 1: New Entry Types

**Files:**
- Modify: `crates/river-gateway/src/channels/entry.rs`

- [ ] **Step 1: Add ToolEntry and HeartbeatEntry structs**

Add after the existing `CursorEntry`:

```rust
/// A tool call or tool result entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEntry {
    /// Snowflake ID
    pub id: String,
    /// "tool_call" or "tool_result"
    pub kind: String,
    /// Tool name
    pub tool_name: String,
    /// Tool call arguments (JSON value)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
    /// Tool result content, or file path if large
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// File path if result was persisted to disk
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_file: Option<String>,
    /// Model's tool call ID for threading
    pub tool_call_id: String,
}

/// A heartbeat entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatEntry {
    /// Snowflake ID
    pub id: String,
    /// Always "heartbeat"
    pub kind: String,
    /// ISO timestamp
    pub timestamp: String,
}
```

- [ ] **Step 2: Create HomeChannelEntry enum with tagged serde**

The home channel uses a *separate* enum from `ChannelEntry` (which stays `untagged` for backward compatibility with existing adapter logs):

```rust
/// Entry in a home channel log — uses tagged serde for unambiguous deserialization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HomeChannelEntry {
    #[serde(rename = "message")]
    Message(MessageEntry),
    #[serde(rename = "cursor")]
    Cursor(CursorEntry),
    #[serde(rename = "tool")]
    Tool(ToolEntry),
    #[serde(rename = "heartbeat")]
    Heartbeat(HeartbeatEntry),
}

impl HomeChannelEntry {
    pub fn id(&self) -> &str {
        match self {
            HomeChannelEntry::Message(m) => &m.id,
            HomeChannelEntry::Cursor(c) => &c.id,
            HomeChannelEntry::Tool(t) => &t.id,
            HomeChannelEntry::Heartbeat(h) => &h.id,
        }
    }
}
```

- [ ] **Step 3: Add source fields to MessageEntry**

Add optional source fields for tracking where user messages came from. The content field stays clean — the context builder formats the tag:

```rust
// Add to MessageEntry struct:
/// Source adapter (for user messages routed through home channel)
#[serde(skip_serializing_if = "Option::is_none")]
pub source_adapter: Option<String>,
/// Source channel ID
#[serde(skip_serializing_if = "Option::is_none")]
pub source_channel_id: Option<String>,
/// Source channel name
#[serde(skip_serializing_if = "Option::is_none")]
pub source_channel_name: Option<String>,
```

- [ ] **Step 4: Add constructors for new roles and types**

```rust
impl MessageEntry {
    /// Create a user message with source tracking (for home channel)
    pub fn user_home(
        id: String,
        author: String,
        author_id: String,
        content: String,
        source_adapter: String,
        source_channel_id: String,
        source_channel_name: Option<String>,
        msg_id: Option<String>,
    ) -> Self {
        Self {
            id,
            role: "user".to_string(),
            author: Some(author),
            author_id: Some(author_id),
            content,
            adapter: "home".to_string(),
            msg_id,
            source_adapter: Some(source_adapter),
            source_channel_id: Some(source_channel_id),
            source_channel_name,
        }
    }

    /// Create a bystander message (anonymous)
    pub fn bystander(id: String, content: String) -> Self {
        Self {
            id,
            role: "bystander".to_string(),
            author: None,
            author_id: None,
            content,
            adapter: "home".to_string(),
            msg_id: None,
            source_adapter: None,
            source_channel_id: None,
            source_channel_name: None,
        }
    }

    /// Create a system message
    pub fn system_msg(id: String, content: String) -> Self {
        Self {
            id,
            role: "system".to_string(),
            author: None,
            author_id: None,
            content,
            adapter: "home".to_string(),
            msg_id: None,
            source_adapter: None,
            source_channel_id: None,
            source_channel_name: None,
        }
    }
}

impl ToolEntry {
    pub fn call(id: String, tool_name: String, arguments: serde_json::Value, tool_call_id: String) -> Self {
        Self {
            id, kind: "tool_call".to_string(), tool_name,
            arguments: Some(arguments), result: None, result_file: None, tool_call_id,
        }
    }

    pub fn result(id: String, tool_name: String, content: String, tool_call_id: String) -> Self {
        Self {
            id, kind: "tool_result".to_string(), tool_name,
            arguments: None, result: Some(content), result_file: None, tool_call_id,
        }
    }

    pub fn result_file(id: String, tool_name: String, file_path: String, tool_call_id: String) -> Self {
        Self {
            id, kind: "tool_result".to_string(), tool_name,
            arguments: None, result: None, result_file: Some(file_path), tool_call_id,
        }
    }
}

impl HeartbeatEntry {
    pub fn new(id: String, timestamp: String) -> Self {
        Self { id, kind: "heartbeat".to_string(), timestamp }
    }
}
```

- [ ] **Step 5: Update existing MessageEntry constructors**

Add default `None` values for the new source fields to `incoming()` and `agent()` constructors so existing code compiles.

- [ ] **Step 6: Write tests**

Test serialization/deserialization of all new types. Verify `#[serde(tag = "type")]` round-trips correctly. Verify `HomeChannelEntry::id()`.

- [ ] **Step 7: Run tests and commit**

Run: `cargo test -p river-gateway -- channels::entry`

```bash
git add -A && git commit -m "feat(channels): add HomeChannelEntry with tagged serde, ToolEntry, HeartbeatEntry, source fields"
```

---

### Task 2: Log Writer Actor

**Files:**
- Create: `crates/river-gateway/src/channels/writer.rs`
- Modify: `crates/river-gateway/src/channels/mod.rs`

- [ ] **Step 1: Create the log writer actor**

```rust
//! Home channel log writer — serialized writes for ordering guarantees

use super::entry::HomeChannelEntry;
use super::log::ChannelLog;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{error, info};

pub enum LogWriteRequest {
    Append(HomeChannelEntry),
    Shutdown,
}

#[derive(Clone)]
pub struct HomeChannelWriter {
    tx: mpsc::Sender<LogWriteRequest>,
}

impl HomeChannelWriter {
    pub fn spawn(home_channel_path: PathBuf) -> Self {
        let (tx, mut rx) = mpsc::channel::<LogWriteRequest>(1024);

        tokio::spawn(async move {
            let log = ChannelLog::from_path(home_channel_path);
            info!("Home channel writer started");

            while let Some(req) = rx.recv().await {
                match req {
                    LogWriteRequest::Append(entry) => {
                        if let Err(e) = log.append_entry(&entry).await {
                            error!(error = %e, "Failed to write to home channel");
                        }
                    }
                    LogWriteRequest::Shutdown => {
                        info!("Home channel writer shutting down");
                        break;
                    }
                }
            }
        });

        Self { tx }
    }

    pub async fn write(&self, entry: HomeChannelEntry) {
        if let Err(e) = self.tx.send(LogWriteRequest::Append(entry)).await {
            error!(error = %e, "Failed to send to home channel writer");
        }
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(LogWriteRequest::Shutdown).await;
    }
}
```

- [ ] **Step 2: Add to channels module**

Add `pub mod writer;` to `crates/river-gateway/src/channels/mod.rs`.

- [ ] **Step 3: Write tests and commit**

Run: `cargo test -p river-gateway -- channels::writer`

```bash
git add -A && git commit -m "feat(channels): add HomeChannelWriter actor for serialized writes"
```

---

### Task 3: Home Channel Context Builder

**Files:**
- Create: `crates/river-gateway/src/agent/home_context.rs`
- Modify: `crates/river-gateway/src/agent/mod.rs`

- [ ] **Step 1: Create the context builder**

The context builder reads the home channel log + move files and produces model messages. It must correctly reconstruct tool call assistant messages:

```rust
//! Home channel context builder — derives model context from home channel + moves

use crate::channels::entry::{HomeChannelEntry, MessageEntry, ToolEntry};
use crate::channels::log::ChannelLog;
use crate::model::{ChatMessage, ToolCallRequest, FunctionCall};
use std::path::{Path, PathBuf};

pub struct HomeContextConfig {
    pub limit: usize,
    pub max_tail_entries: usize,
}

impl Default for HomeContextConfig {
    fn default() -> Self {
        Self { limit: 128_000, max_tail_entries: 200 }
    }
}

/// Build model context from home channel + moves
pub async fn build_context(
    home_channel_path: &Path,
    moves: &[String],
    config: &HomeContextConfig,
) -> std::io::Result<Vec<ChatMessage>> {
    let log = ChannelLog::from_path(home_channel_path.to_path_buf());
    let all_entries = log.read_all_home().await?;

    let mut messages = Vec::new();

    // Moves as compressed history
    for mov in moves {
        messages.push(ChatMessage::system(mov.clone()));
    }

    // Tail entries
    let tail_start = all_entries.len().saturating_sub(config.max_tail_entries);
    let tail = &all_entries[tail_start..];

    // Process tail — need to group tool calls with their assistant message
    let mut i = 0;
    while i < tail.len() {
        match &tail[i] {
            HomeChannelEntry::Message(m) => {
                match m.role.as_str() {
                    "agent" => messages.push(ChatMessage::assistant(Some(m.content.clone()), None)),
                    "user" => {
                        // Format tag from source fields
                        let tagged = format_user_tag(m);
                        messages.push(ChatMessage::user(tagged));
                    }
                    "bystander" => {
                        let tagged = format!("[bystander] {}", m.content);
                        messages.push(ChatMessage::user(tagged));
                    }
                    "system" => messages.push(ChatMessage::system(m.content.clone())),
                    _ => {}
                }
            }
            HomeChannelEntry::Tool(t) => {
                match t.kind.as_str() {
                    "tool_call" => {
                        // Collect consecutive tool calls into one assistant message
                        let mut tool_calls = Vec::new();
                        while i < tail.len() {
                            if let HomeChannelEntry::Tool(tc) = &tail[i] {
                                if tc.kind == "tool_call" {
                                    tool_calls.push(ToolCallRequest {
                                        id: tc.tool_call_id.clone(),
                                        function: FunctionCall {
                                            name: tc.tool_name.clone(),
                                            arguments: tc.arguments
                                                .as_ref()
                                                .map(|a| serde_json::to_string(a).unwrap_or_default())
                                                .unwrap_or_default(),
                                        },
                                    });
                                    i += 1;
                                    continue;
                                }
                            }
                            break;
                        }
                        messages.push(ChatMessage::assistant(None, Some(tool_calls)));
                        continue; // don't increment i again
                    }
                    "tool_result" => {
                        let content = if let Some(ref result) = t.result {
                            result.clone()
                        } else if let Some(ref file) = t.result_file {
                            tokio::fs::read_to_string(file).await
                                .unwrap_or_else(|_| format!("[tool result file missing: {}]", file))
                        } else {
                            "[empty tool result]".to_string()
                        };
                        messages.push(ChatMessage::tool(&t.tool_call_id, &content));
                    }
                    _ => {}
                }
            }
            HomeChannelEntry::Heartbeat(_) => {
                messages.push(ChatMessage::system("[heartbeat]".to_string()));
            }
            HomeChannelEntry::Cursor(_) => {}
        }
        i += 1;
    }

    Ok(messages)
}

/// Format a user message with source adapter/channel tag
fn format_user_tag(m: &MessageEntry) -> String {
    let author = m.author.as_deref().unwrap_or("unknown");
    match (&m.source_adapter, &m.source_channel_id, &m.source_channel_name) {
        (Some(adapter), Some(ch_id), Some(ch_name)) => {
            format!("[user:{}:{}/{}] {}: {}", adapter, ch_id, ch_name, author, m.content)
        }
        (Some(adapter), Some(ch_id), None) => {
            format!("[user:{}:{}] {}: {}", adapter, ch_id, author, m.content)
        }
        _ => format!("{}: {}", author, m.content),
    }
}
```

- [ ] **Step 2: Add `read_all_home` to ChannelLog**

The home channel reader deserializes `HomeChannelEntry` (tagged) instead of `ChannelEntry` (untagged):

```rust
// In channels/log.rs:
pub async fn read_all_home(&self) -> std::io::Result<Vec<HomeChannelEntry>> {
    if !self.path.exists() {
        return Ok(Vec::new());
    }
    let file = tokio::fs::File::open(&self.path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut entries = Vec::new();
    let mut line_num = 0usize;

    while let Some(line) = lines.next_line().await? {
        line_num += 1;
        if line.trim().is_empty() { continue; }
        match serde_json::from_str::<HomeChannelEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!(line_num, error = %e, path = %self.path.display(), "Skipping malformed home channel line");
            }
        }
    }
    Ok(entries)
}
```

- [ ] **Step 3: Add to agent module**

Add `pub mod home_context;` to `crates/river-gateway/src/agent/mod.rs`.

- [ ] **Step 4: Write tests**

Test context building with a home channel containing messages, tool calls (consecutive), tool results, bystander messages, heartbeats. Verify tool calls are grouped into single assistant messages. Verify user messages are tagged correctly from source fields.

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p river-gateway -- agent::home_context`

```bash
git add -A && git commit -m "feat(agent): add home channel context builder with full tool call mapping"
```

---

### Task 4: Bystander Endpoint

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`
- Modify: `crates/river-gateway/src/state.rs`

- [ ] **Step 1: Add `home_channel_writer` to AppState**

Add to `AppState`:

```rust
pub home_channel_writer: crate::channels::writer::HomeChannelWriter,
```

Update `AppState::new` to accept and store it.

- [ ] **Step 2: Add bystander handler and route**

```rust
#[derive(Deserialize)]
struct BystanderMessage {
    content: String,
}

async fn handle_bystander(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(msg): Json<BystanderMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if let Err(status) = validate_auth(&headers, state.auth_token.as_deref()) {
        return Err(status);
    }

    let snowflake = state.snowflake_gen.next_id(river_core::SnowflakeType::Message);
    let entry = crate::channels::entry::MessageEntry::bystander(
        snowflake.to_string(), msg.content,
    );

    state.home_channel_writer.write(
        crate::channels::entry::HomeChannelEntry::Message(entry)
    ).await;

    state.message_queue.push(crate::queue::ChannelNotification {
        channel: "home".to_string(),
        snowflake_id: snowflake.to_string(),
    });

    Ok(Json(serde_json::json!({ "ok": true, "id": snowflake.to_string() })))
}
```

Register: `.route("/home/:agent_name/message", post(handle_bystander))`

- [ ] **Step 3: Write tests and commit**

```bash
git add -A && git commit -m "feat(api): add bystander endpoint POST /home/:agent_name/message"
```

---

### Task 5: Refactor Tool Execution to Preserve Tool Names

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`

Before wiring the home channel into the turn cycle, we need tool execution to return the tool name alongside the result.

- [ ] **Step 1: Create ToolResult struct**

Add at the top of `task.rs` (or in a shared types module):

```rust
/// Result of a tool execution, preserving tool name for logging
pub struct ToolExecResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub result: String,
}
```

- [ ] **Step 2: Update `execute_tool_calls` return type**

Change from `Vec<(String, String)>` to `Vec<ToolExecResult>`. Thread the tool name through from `ToolCallRequest::function::name`.

- [ ] **Step 3: Update all call sites**

Update the loop that adds tool results to the conversation to use `ToolExecResult` fields.

- [ ] **Step 4: Run tests and commit**

```bash
git add -A && git commit -m "refactor(agent): ToolExecResult preserves tool name through execution"
```

---

### Task 6: Wire Home Channel into Turn Cycle

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`

- [ ] **Step 1: Add HomeChannelWriter to AgentTask**

Add field, pass from server setup.

- [ ] **Step 2: Write assistant responses to home channel**

After assistant response is added to context:

```rust
if let Some(ref content) = response.content {
    let entry = MessageEntry::agent(
        self.snowflake_gen.next_id(SnowflakeType::Message).to_string(),
        content.clone(), "home".to_string(), None,
    );
    self.home_channel_writer.write(HomeChannelEntry::Message(entry)).await;
}
```

- [ ] **Step 3: Write tool calls to home channel**

Before execution, using the now-available tool name:

```rust
for tc in &response.tool_calls {
    let entry = ToolEntry::call(
        self.snowflake_gen.next_id(SnowflakeType::Message).to_string(),
        tc.function.name.clone(),
        serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null),
        tc.id.clone(),
    );
    self.home_channel_writer.write(HomeChannelEntry::Tool(entry)).await;
}
```

- [ ] **Step 4: Write tool results to home channel**

Using `ToolExecResult` for the tool name. Large results (>4096 bytes) written to files:

```rust
for result in &tool_results {
    let snowflake = self.snowflake_gen.next_id(SnowflakeType::Message).to_string();
    let entry = if result.result.len() > 4096 {
        let results_dir = self.config.workspace.join("channels/home").join(&self.agent_name).join("tool-results");
        tokio::fs::create_dir_all(&results_dir).await.ok();
        let file_path = results_dir.join(format!("{}.txt", snowflake));
        tokio::fs::write(&file_path, &result.result).await.ok();
        ToolEntry::result_file(snowflake, result.tool_name.clone(), file_path.to_string_lossy().to_string(), result.tool_call_id.clone())
    } else {
        ToolEntry::result(snowflake, result.tool_name.clone(), result.result.clone(), result.tool_call_id.clone())
    };
    self.home_channel_writer.write(HomeChannelEntry::Tool(entry)).await;
}
```

- [ ] **Step 5: Add final batch check**

After the turn loop (no tool calls), check for messages that arrived during the model call:

```rust
let final_batch = self.message_queue.drain();
if !final_batch.is_empty() {
    // Inject as system message, loop back to model completion
}
```

- [ ] **Step 6: Run tests and commit**

```bash
git add -A && git commit -m "feat(agent): write all output to home channel during turn cycle"
```

---

### Task 7: Switch Context Source from PersistentContext to Home Channel

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`

This is the actual switchover — the moment the agent starts reading its context from the home channel instead of `PersistentContext`.

- [ ] **Step 1: Replace context building**

In the turn cycle, replace:
```rust
let messages = self.context.build_messages(&system_prompt);
```
with:
```rust
let moves = self.load_moves().await;
let messages_from_home = home_context::build_context(
    &self.home_channel_path, &moves, &self.home_context_config,
).await?;
let mut messages = vec![ChatMessage::system(system_prompt)];
messages.extend(messages_from_home);
```

- [ ] **Step 2: Remove PersistentContext from AgentTask**

Remove the `context: PersistentContext` field. Remove all `self.context.append(...)` calls — these are replaced by the home channel writes from Task 6. Remove `persist_turn_messages` calls (SQL write).

- [ ] **Step 3: Remove context.rs**

Delete `crates/river-gateway/src/agent/context.rs` and remove `pub mod context;` from `agent/mod.rs`. Fix all compilation errors.

- [ ] **Step 4: Run tests and commit**

```bash
git add -A && git commit -m "feat(agent): switch context source to home channel, remove PersistentContext"
```

---

### Task 8: Wire Incoming Messages to Home Channel

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Write incoming messages to home channel (write-ahead)**

In `handle_incoming`, before the adapter channel log write:

```rust
let home_entry = MessageEntry::user_home(
    snowflake_str.clone(),
    msg.author.name.clone(),
    msg.author.id.clone(),
    msg.content.clone(),
    msg.adapter.clone(),
    msg.channel.clone(),
    msg.channel_name.clone(),
    msg.message_id.clone(),
);
state.home_channel_writer.write(HomeChannelEntry::Message(home_entry)).await;
```

- [ ] **Step 2: Update notification channel key**

```rust
state.message_queue.push(crate::queue::ChannelNotification {
    channel: "home".to_string(),
    snowflake_id: snowflake_str,
});
```

- [ ] **Step 3: Run tests and commit**

```bash
git add -A && git commit -m "feat(api): write incoming messages to home channel (write-ahead)"
```

---

### Task 9: Remove ChannelContext and ChannelSwitched

**Files:**
- Delete: `crates/river-gateway/src/agent/channel.rs`
- Modify: `crates/river-gateway/src/agent/mod.rs`
- Modify: `crates/river-gateway/src/coordinator/events.rs`

- [ ] **Step 1: Remove channel.rs**

Delete the file. Remove `pub mod channel;` from `agent/mod.rs`.

- [ ] **Step 2: Remove ChannelSwitched from AgentEvent**

Remove the variant from `coordinator/events.rs`.

- [ ] **Step 3: Fix all compilation errors**

Specific files that will break:
- `agent/task.rs` — remove `channel_context`, `pending_channel_switch` fields and all switching logic
- `server.rs` — remove any ChannelContext construction
- Any spectator handler that references channel switches

- [ ] **Step 4: Run full test suite and commit**

Run: `cargo test -p river-gateway`

```bash
git add -A && git commit -m "refactor: remove ChannelContext, ChannelSwitched, channel switching"
```

---

### Task 10: Update Spectator — File-Based Moves with Snowflake Ranges

**Files:**
- Modify: `crates/river-gateway/src/spectator/handlers.rs`
- Modify: `crates/river-gateway/src/spectator/format.rs`
- Create: moves directory at `channels/home/{agent_name}/moves/`

- [ ] **Step 1: Move storage from SQL to files**

Moves are written to `channels/home/{agent_name}/moves/{snowflake_start}-{snowflake_end}.md`. Each move file contains the summary text. The filename encodes the snowflake range it covers.

- [ ] **Step 2: Update spectator transcript to read home channel**

The spectator's `on_turn_complete` reads the home channel entries for the current turn (by snowflake range) and formats them into a transcript.

- [ ] **Step 3: Add `load_moves` method to AgentTask**

Read all move files from the moves directory, sorted by snowflake range, return as `Vec<String>`.

- [ ] **Step 4: Run tests and commit**

```bash
git add -A && git commit -m "feat(spectator): file-based moves with snowflake ranges, read home channel"
```

---

### Task 11: Tool Result File Cleanup

**Files:**
- Modify: `crates/river-gateway/src/channels/writer.rs`

- [ ] **Step 1: Add cleanup to log writer**

When notified of a new move, the writer reads entries in the covered snowflake range, finds `ToolEntry` entries with `result_file`, and deletes the files:

```rust
impl HomeChannelWriter {
    pub async fn cleanup_tool_results(&self, move_start: &str, move_end: &str, home_channel_path: &Path) {
        // Read entries in range, find tool result files, delete them
    }
}
```

- [ ] **Step 2: Wire into spectator move completion**

After the spectator writes a move file, send cleanup request to writer.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(channels): tool result file cleanup on move supersession"
```

---

### Task 12: Server Wiring

**Files:**
- Modify: `crates/river-gateway/src/server.rs`
- Modify: `crates/river-gateway/src/state.rs`

- [ ] **Step 1: Create HomeChannelWriter on startup**

```rust
let home_channel_path = config.workspace
    .join("channels/home")
    .join(format!("{}.jsonl", config.agent_name));
let home_channel_writer = HomeChannelWriter::spawn(home_channel_path);
```

- [ ] **Step 2: Pass to AppState and AgentTask**

- [ ] **Step 3: Create home channel directory on birth**

Create `channels/home/{agent_name}/` and `channels/home/{agent_name}/moves/` and `channels/home/{agent_name}/tool-results/`.

- [ ] **Step 4: Remove SQL message persistence**

Remove `persist_turn_messages` and related DB calls for message storage. The `Database` may still be needed for other purposes (config, etc.) — only remove message-specific tables.

- [ ] **Step 5: Run full test suite**

Run: `cargo test -p river-gateway`

- [ ] **Step 6: Integration smoke test**

Start the gateway. Verify:
- Home channel JSONL created with tagged serde entries
- Agent responses, tool calls (with names), tool results appear
- Incoming messages appear with source tags
- Bystander endpoint works
- Context is built from home channel + moves
- No SQL message writes

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat(server): wire HomeChannelWriter, remove SQL message persistence"
```

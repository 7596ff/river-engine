# Home Channel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the agent's invisible internal context with a visible, append-only home channel log that becomes the single source of truth for all agent activity.

**Architecture:** The home channel is an append-only JSONL log at `channels/home/{agent_name}.jsonl`. All agent output, tool calls, incoming messages, and heartbeats are written to it through a serialized log writer actor. The model context is built ephemerally by reading the log tail plus spectator moves. Per-adapter channel logs remain as secondary projections. SQL message storage is eliminated.

**Tech Stack:** Rust, tokio (async), serde/serde_json (serialization), JSONL (storage)

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

- [ ] **Step 2: Extend ChannelEntry enum**

```rust
pub enum ChannelEntry {
    Message(MessageEntry),
    Cursor(CursorEntry),
    Tool(ToolEntry),
    Heartbeat(HeartbeatEntry),
}
```

Update `is_agent()` and `id()` methods on `ChannelEntry` to handle the new variants:

```rust
impl ChannelEntry {
    pub fn is_agent(&self) -> bool {
        match self {
            ChannelEntry::Message(m) => m.is_agent(),
            ChannelEntry::Cursor(_) => true,
            ChannelEntry::Tool(_) => true,
            ChannelEntry::Heartbeat(_) => false,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            ChannelEntry::Message(m) => &m.id,
            ChannelEntry::Cursor(c) => &c.id,
            ChannelEntry::Tool(t) => &t.id,
            ChannelEntry::Heartbeat(h) => &h.id,
        }
    }
}
```

- [ ] **Step 3: Extend MessageEntry with new roles**

Add `bystander` and `system` as valid role values. Add convenience constructors:

```rust
impl MessageEntry {
    /// Create a user message tagged with adapter source
    pub fn user(
        id: String,
        adapter: String,
        channel_id: String,
        channel_name: Option<String>,
        author: String,
        author_id: String,
        content: String,
        msg_id: Option<String>,
    ) -> Self {
        let tag = match channel_name {
            Some(ref name) => format!("[user:{}:{}/{}] {}", adapter, channel_id, name, content),
            None => format!("[user:{}:{}] {}", adapter, channel_id, content),
        };
        Self {
            id,
            role: "user".to_string(),
            author: Some(author),
            author_id: Some(author_id),
            content: tag,
            adapter,
            msg_id,
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
        }
    }

    /// Create a system message
    pub fn system(id: String, content: String) -> Self {
        Self {
            id,
            role: "system".to_string(),
            author: None,
            author_id: None,
            content,
            adapter: "home".to_string(),
            msg_id: None,
        }
    }
}
```

- [ ] **Step 4: Add ToolEntry constructors**

```rust
impl ToolEntry {
    pub fn call(id: String, tool_name: String, arguments: serde_json::Value, tool_call_id: String) -> Self {
        Self {
            id,
            kind: "tool_call".to_string(),
            tool_name,
            arguments: Some(arguments),
            result: None,
            result_file: None,
            tool_call_id,
        }
    }

    pub fn result(id: String, tool_name: String, result: String, tool_call_id: String) -> Self {
        Self {
            id,
            kind: "tool_result".to_string(),
            tool_name,
            arguments: None,
            result: Some(result),
            result_file: None,
            tool_call_id,
        }
    }

    pub fn result_file(id: String, tool_name: String, file_path: String, tool_call_id: String) -> Self {
        Self {
            id,
            kind: "tool_result".to_string(),
            tool_name,
            arguments: None,
            result: None,
            result_file: Some(file_path),
            tool_call_id,
        }
    }
}

impl HeartbeatEntry {
    pub fn new(id: String, timestamp: String) -> Self {
        Self {
            id,
            kind: "heartbeat".to_string(),
            timestamp,
        }
    }
}
```

- [ ] **Step 5: Write tests for new entry types**

Add tests for serialization/deserialization of `ToolEntry`, `HeartbeatEntry`, `MessageEntry::user`, `MessageEntry::bystander`, `MessageEntry::system`, and updated `ChannelEntry::is_agent`/`id` methods.

- [ ] **Step 6: Run tests**

Run: `cargo test -p river-gateway -- channels::entry`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat(channels): add ToolEntry, HeartbeatEntry, user/bystander/system message roles"
```

---

### Task 2: Log Writer Actor

**Files:**
- Create: `crates/river-gateway/src/channels/writer.rs`
- Modify: `crates/river-gateway/src/channels/mod.rs`

- [ ] **Step 1: Create the log writer actor**

```rust
//! Home channel log writer — serialized writes for ordering guarantees
//!
//! All writes to the home channel go through this actor. It owns the file
//! handle and ensures entries are ordered and never interleaved.

use super::entry::ChannelEntry;
use super::log::ChannelLog;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{error, info};

/// Message to the log writer
pub enum LogWriteRequest {
    /// Append an entry to the home channel
    Append(ChannelEntry),
    /// Shutdown the writer
    Shutdown,
}

/// Handle for sending writes to the log writer actor
#[derive(Clone)]
pub struct HomeChannelWriter {
    tx: mpsc::Sender<LogWriteRequest>,
}

impl HomeChannelWriter {
    /// Spawn the log writer actor, returns a handle for sending writes
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

    /// Write an entry to the home channel
    pub async fn write(&self, entry: ChannelEntry) {
        if let Err(e) = self.tx.send(LogWriteRequest::Append(entry)).await {
            error!(error = %e, "Failed to send to home channel writer");
        }
    }

    /// Shutdown the writer
    pub async fn shutdown(&self) {
        let _ = self.tx.send(LogWriteRequest::Shutdown).await;
    }
}
```

- [ ] **Step 2: Add to channels module**

Add `pub mod writer;` to `crates/river-gateway/src/channels/mod.rs`.

- [ ] **Step 3: Write tests**

Test that the writer serializes writes correctly and entries appear in order.

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway -- channels::writer`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(channels): add HomeChannelWriter actor for serialized writes"
```

---

### Task 3: Home Channel Context Builder

**Files:**
- Create: `crates/river-gateway/src/agent/home_context.rs`
- Modify: `crates/river-gateway/src/agent/mod.rs`

- [ ] **Step 1: Create the context builder**

```rust
//! Home channel context builder — derives model context from home channel + moves
//!
//! Reads the home channel log, finds the most recent move's coverage,
//! and builds model messages from moves (compressed past) + log tail (live present).

use crate::channels::entry::{ChannelEntry, MessageEntry, ToolEntry, HeartbeatEntry};
use crate::channels::log::ChannelLog;
use crate::model::ChatMessage;
use std::path::{Path, PathBuf};

/// Configuration for context building
#[derive(Debug, Clone)]
pub struct HomeContextConfig {
    /// Maximum tokens for the context window
    pub limit: usize,
    /// Maximum entries to read from the log tail
    pub max_tail_entries: usize,
}

impl Default for HomeContextConfig {
    fn default() -> Self {
        Self {
            limit: 128_000,
            max_tail_entries: 200,
        }
    }
}

/// Build model context from home channel + moves
pub async fn build_context(
    home_channel_path: &Path,
    moves: &[String],
    config: &HomeContextConfig,
) -> std::io::Result<Vec<ChatMessage>> {
    let log = ChannelLog::from_path(home_channel_path.to_path_buf());
    let entries = log.read_all().await?;

    let mut messages = Vec::new();

    // Add moves as compressed history (system messages)
    for mov in moves {
        messages.push(ChatMessage::system(mov.clone()));
    }

    // Read tail entries after the most recent move
    // For now, take the last max_tail_entries entries
    let tail_start = entries.len().saturating_sub(config.max_tail_entries);
    let tail = &entries[tail_start..];

    // Map entries to model messages
    for entry in tail {
        match entry {
            ChannelEntry::Message(m) => {
                match m.role.as_str() {
                    "agent" => messages.push(ChatMessage::assistant(
                        Some(m.content.clone()), None,
                    )),
                    "user" | "bystander" => messages.push(ChatMessage::user(
                        m.content.clone(),
                    )),
                    "system" => messages.push(ChatMessage::system(
                        m.content.clone(),
                    )),
                    _ => {} // skip unknown roles
                }
            }
            ChannelEntry::Tool(t) => {
                match t.kind.as_str() {
                    "tool_call" => {
                        // Reconstruct as assistant message with tool_use
                        // (simplified — full implementation maps to ToolCallRequest)
                    }
                    "tool_result" => {
                        let content = if let Some(ref result) = t.result {
                            result.clone()
                        } else if let Some(ref file) = t.result_file {
                            // Read from file
                            match tokio::fs::read_to_string(file).await {
                                Ok(c) => c,
                                Err(_) => format!("[tool result file missing: {}]", file),
                            }
                        } else {
                            "[empty tool result]".to_string()
                        };
                        messages.push(ChatMessage::tool(&t.tool_call_id, &content));
                    }
                    _ => {}
                }
            }
            ChannelEntry::Heartbeat(_) => {
                messages.push(ChatMessage::system("[heartbeat]".to_string()));
            }
            ChannelEntry::Cursor(_) => {} // skip cursors
        }
    }

    Ok(messages)
}
```

- [ ] **Step 2: Add to agent module**

Add `pub mod home_context;` to `crates/river-gateway/src/agent/mod.rs`.

- [ ] **Step 3: Write tests**

Test context building from a home channel with various entry types. Verify moves appear as system messages before the tail entries.

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway -- agent::home_context`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(agent): add home channel context builder"
```

---

### Task 4: Bystander Endpoint

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Add bystander message handler**

Add a new route and handler:

```rust
/// Bystander message request
#[derive(Deserialize)]
struct BystanderMessage {
    content: String,
}

async fn handle_bystander(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(msg): Json<BystanderMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Validate authentication
    if let Err(status) = validate_auth(&headers, state.auth_token.as_deref()) {
        return Err(status);
    }

    let snowflake = state.snowflake_gen.next_id(river_core::SnowflakeType::Message);
    let entry = crate::channels::entry::MessageEntry::bystander(
        snowflake.to_string(),
        msg.content,
    );

    // Write to home channel via the writer actor
    state.home_channel_writer.write(
        crate::channels::entry::ChannelEntry::Message(entry)
    ).await;

    // Notify the message queue
    state.message_queue.push(crate::queue::ChannelNotification {
        channel: "home".to_string(),
        snowflake_id: snowflake.to_string(),
    });

    Ok(Json(serde_json::json!({ "ok": true, "id": snowflake.to_string() })))
}
```

- [ ] **Step 2: Register the route**

In the router builder, add:

```rust
.route("/home/:agent_name/message", post(handle_bystander))
```

- [ ] **Step 3: Add `home_channel_writer` to AppState**

Modify `crates/river-gateway/src/state.rs` to add:

```rust
pub home_channel_writer: crate::channels::writer::HomeChannelWriter,
```

And update `AppState::new` to accept and store it.

- [ ] **Step 4: Write tests**

Test the bystander endpoint: valid request writes to home channel, auth required.

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-gateway -- api::routes`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(api): add bystander endpoint POST /home/:agent_name/message"
```

---

### Task 5: Wire Home Channel into Turn Cycle

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`

This is the largest task. It modifies the turn cycle to write all output to the home channel and build context from it.

- [ ] **Step 1: Add HomeChannelWriter to AgentTask**

Add a `home_channel_writer: HomeChannelWriter` field to `AgentTask`. Pass it in from server setup.

- [ ] **Step 2: Write assistant responses to home channel**

After line 391 (where assistant response is added to context), also write to home channel:

```rust
// Write to home channel
if let Some(ref content) = response.content {
    let entry = MessageEntry::agent(
        self.snowflake_gen.next_id(SnowflakeType::Message).to_string(),
        content.clone(),
        "home".to_string(),
        None,
    );
    self.home_channel_writer.write(ChannelEntry::Message(entry)).await;
}
```

- [ ] **Step 3: Write tool calls to home channel**

Before tool execution, write the tool call entry:

```rust
for tc in &response.tool_calls {
    let entry = ToolEntry::call(
        self.snowflake_gen.next_id(SnowflakeType::Message).to_string(),
        tc.function.name.clone(),
        serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null),
        tc.id.clone(),
    );
    self.home_channel_writer.write(ChannelEntry::Tool(entry)).await;
}
```

- [ ] **Step 4: Write tool results to home channel**

After tool execution, write each result. For large results (>4096 bytes), write to a file:

```rust
for (tool_call_id, result_text) in &tool_results {
    let snowflake = self.snowflake_gen.next_id(SnowflakeType::Message).to_string();

    let entry = if result_text.len() > 4096 {
        // Write to file
        let results_dir = self.config.workspace
            .join("channels/home")
            .join(&self.agent_name)
            .join("tool-results");
        tokio::fs::create_dir_all(&results_dir).await.ok();
        let file_path = results_dir.join(format!("{}.txt", snowflake));
        tokio::fs::write(&file_path, result_text).await.ok();
        ToolEntry::result_file(
            snowflake,
            "unknown".to_string(), // tool name not available here — needs threading
            file_path.to_string_lossy().to_string(),
            tool_call_id.clone(),
        )
    } else {
        ToolEntry::result(
            snowflake,
            "unknown".to_string(),
            result_text.clone(),
            tool_call_id.clone(),
        )
    };
    self.home_channel_writer.write(ChannelEntry::Tool(entry)).await;
}
```

- [ ] **Step 5: Add final batch check**

After the turn loop ends (no tool calls), check for batched messages:

```rust
// Step 6: Final batch check — even if no tool calls
let final_batch = self.message_queue.drain();
if !final_batch.is_empty() {
    // Inject as system message and continue the turn
    // ... (append to messages, go to step 3)
}
```

- [ ] **Step 6: Remove channel switching**

Remove `channel_context`, `pending_channel_switch`, `ChannelContext` imports, and all channel switching logic from `AgentTask`.

- [ ] **Step 7: Run tests**

Run: `cargo test -p river-gateway -- agent::task`
Expected: All existing tests pass (some may need updating for removed channel context).

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat(agent): wire home channel into turn cycle, remove channel switching"
```

---

### Task 6: Wire Incoming Messages to Home Channel

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Write incoming messages to home channel (write-ahead)**

In `handle_incoming`, before writing to the adapter channel log, write a tagged user entry to the home channel:

```rust
// Write-ahead: home channel first
let home_entry = MessageEntry::user(
    snowflake_str.clone(),
    msg.adapter.clone(),
    msg.channel.clone(),
    msg.channel_name.clone(),
    msg.author.name.clone(),
    msg.author.id.clone(),
    msg.content.clone(),
    msg.message_id.clone(),
);
state.home_channel_writer.write(ChannelEntry::Message(home_entry)).await;

// Then adapter channel log (secondary)
let adapter_entry = MessageEntry::incoming(/* ... existing code ... */);
let log = ChannelLog::open(&channels_dir, &msg.adapter, &msg.channel);
if let Err(e) = log.append_entry(&adapter_entry).await {
    tracing::warn!(error = %e, "Failed to write adapter channel log (secondary)");
}
```

- [ ] **Step 2: Update notification to use "home" channel key**

Change the message queue notification:

```rust
state.message_queue.push(crate::queue::ChannelNotification {
    channel: "home".to_string(),
    snowflake_id: snowflake_str,
});
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway -- api::routes`
Expected: Tests pass (update test fixtures for new notification channel key).

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(api): write incoming messages to home channel (write-ahead)"
```

---

### Task 7: Remove ChannelContext and ChannelSwitched Event

**Files:**
- Delete: `crates/river-gateway/src/agent/channel.rs`
- Modify: `crates/river-gateway/src/agent/mod.rs`
- Modify: `crates/river-gateway/src/coordinator/events.rs`

- [ ] **Step 1: Remove channel.rs**

Delete the file and remove `pub mod channel;` from `agent/mod.rs`.

- [ ] **Step 2: Remove ChannelSwitched from events**

Remove the `ChannelSwitched` variant from `AgentEvent` in `coordinator/events.rs`.

- [ ] **Step 3: Fix all compilation errors**

Remove all references to `ChannelContext`, `ChannelSwitched`, `pending_channel_switch` throughout the codebase.

- [ ] **Step 4: Run full test suite**

Run: `cargo test -p river-gateway`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "refactor: remove ChannelContext, ChannelSwitched, channel switching logic"
```

---

### Task 8: Update Spectator to Read Home Channel

**Files:**
- Modify: `crates/river-gateway/src/spectator/handlers.rs`
- Modify: `crates/river-gateway/src/spectator/format.rs`

- [ ] **Step 1: Update spectator to read home channel for transcript**

The spectator's `on_turn_complete` handler builds a transcript from the turn. Update it to read from the home channel log instead of `PersistentContext`. The spectator reads the entries for the current turn (by snowflake range) and formats them into a transcript.

- [ ] **Step 2: Update move format to include snowflake range**

Moves should include the snowflake range they cover so the context builder knows where the move's coverage ends.

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway -- spectator`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(spectator): read home channel for transcript, snowflake ranges in moves"
```

---

### Task 9: Tool Result File Cleanup

**Files:**
- Modify: `crates/river-gateway/src/channels/writer.rs`

- [ ] **Step 1: Add cleanup method to HomeChannelWriter**

When notified that a move has been written covering a snowflake range, the writer deletes any tool result files referenced by entries in that range:

```rust
impl HomeChannelWriter {
    /// Clean up tool result files for entries covered by a move
    pub async fn cleanup_tool_results(&self, covered_snowflake_ids: &[String]) {
        // Send a cleanup request through the channel
        // The writer task reads entries, finds tool result files, deletes them
    }
}
```

- [ ] **Step 2: Wire into spectator move completion**

After the spectator writes a move, notify the writer to clean up.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(channels): tool result file cleanup on move supersession"
```

---

### Task 10: Server Wiring

**Files:**
- Modify: `crates/river-gateway/src/server.rs`
- Modify: `crates/river-gateway/src/state.rs`

- [ ] **Step 1: Create HomeChannelWriter on server startup**

```rust
let home_channel_path = config.workspace
    .join("channels/home")
    .join(format!("{}.jsonl", config.agent_name));
let home_channel_writer = HomeChannelWriter::spawn(home_channel_path);
```

- [ ] **Step 2: Pass writer to AppState and AgentTask**

Update `AppState` to hold the writer. Pass it to `AgentTask` during construction.

- [ ] **Step 3: Create home channel directory on birth**

During agent birth, create `channels/home/{agent_name}/` and the initial JSONL file.

- [ ] **Step 4: Run full test suite**

Run: `cargo test -p river-gateway`
Expected: All tests pass.

- [ ] **Step 5: Integration smoke test**

Start the gateway with an agent. Verify:
- Home channel JSONL file is created
- Agent responses appear in the log
- Tool calls/results appear in the log
- Incoming messages appear tagged
- Bystander endpoint works

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(server): wire HomeChannelWriter into server startup and state"
```

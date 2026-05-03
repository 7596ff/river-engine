# Channel Messages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the broken inbox system with JSONL channel logs, connecting the HTTP handler to the agent via a lightweight notification queue.

**Architecture:** New `channels` module handles JSONL read/write and cursor scanning. The `MessageQueue` is simplified to carry notifications (channel + snowflake) instead of full messages. The handler appends to the channel log and pushes a notification. The agent reads the log from its cursor on wake.

**Tech Stack:** Rust, serde_json, tokio, river-core Snowflake IDs

**Compilation note:** Tasks 3-5 form a refactor batch. After Task 3 changes the queue payload, the codebase will not compile until Tasks 4 and 5 update the consumers. Move through these three tasks rapidly in a single session. Do not expect `cargo check` to pass between Tasks 3 and 5.

---

### Task 1: Channel Log Entry Types

Define the JSONL entry types and serialization.

**Files:**
- Create: `crates/river-gateway/src/channels/entry.rs`
- Create: `crates/river-gateway/src/channels/mod.rs`

- [ ] **Step 1: Write the failing test for entry serialization**

Create `crates/river-gateway/src/channels/entry.rs`:

```rust
//! Channel log entry types
//!
//! Each line in a channel JSONL log is one of these entries.

use river_core::Snowflake;
use serde::{Deserialize, Serialize};

/// A single entry in a channel log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChannelEntry {
    Message(MessageEntry),
    Cursor(CursorEntry),
}

/// A message from either the agent or another speaker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEntry {
    /// Snowflake ID — unique, sortable, encodes timestamp
    pub id: String,
    /// "agent" or "other"
    pub role: String,
    /// Display name of the speaker (for role: "other")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Adapter-specific unique ID of the speaker (for role: "other")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    /// The message text
    pub content: String,
    /// Which adapter the message came through
    pub adapter: String,
    /// Adapter-specific message ID (for replies, edits, deletes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
}

/// A cursor entry — agent read up to this point without speaking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorEntry {
    /// Snowflake ID
    pub id: String,
    /// Always "agent"
    pub role: String,
    /// Always true
    pub cursor: bool,
}

impl MessageEntry {
    /// Create an incoming message entry (role: "other")
    pub fn incoming(
        id: String,
        author: String,
        author_id: String,
        content: String,
        adapter: String,
        msg_id: Option<String>,
    ) -> Self {
        Self {
            id,
            role: "other".to_string(),
            author: Some(author),
            author_id: Some(author_id),
            content,
            adapter,
            msg_id,
        }
    }

    /// Create an outbound agent message entry (role: "agent")
    pub fn agent(
        id: String,
        content: String,
        adapter: String,
        msg_id: Option<String>,
    ) -> Self {
        Self {
            id,
            role: "agent".to_string(),
            author: None,
            author_id: None,
            content,
            adapter,
            msg_id,
        }
    }

    /// Returns true if this is an agent message
    pub fn is_agent(&self) -> bool {
        self.role == "agent"
    }
}

impl CursorEntry {
    pub fn new(id: String) -> Self {
        Self {
            id,
            role: "agent".to_string(),
            cursor: true,
        }
    }
}

impl ChannelEntry {
    /// Returns true if this entry is from the agent (message or cursor)
    pub fn is_agent(&self) -> bool {
        match self {
            ChannelEntry::Message(m) => m.is_agent(),
            ChannelEntry::Cursor(_) => true,
        }
    }

    /// Get the snowflake ID string
    pub fn id(&self) -> &str {
        match self {
            ChannelEntry::Message(m) => &m.id,
            ChannelEntry::Cursor(c) => &c.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message_serialization() {
        let entry = MessageEntry::incoming(
            "ABC123".to_string(),
            "cassie".to_string(),
            "12345".to_string(),
            "hello".to_string(),
            "discord".to_string(),
            Some("msg_001".to_string()),
        );
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"role\":\"other\""));
        assert!(json.contains("\"author\":\"cassie\""));
        assert!(json.contains("\"msg_id\":\"msg_001\""));
    }

    #[test]
    fn test_agent_message_serialization() {
        let entry = MessageEntry::agent(
            "ABC124".to_string(),
            "good morning".to_string(),
            "discord".to_string(),
            Some("msg_002".to_string()),
        );
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"role\":\"agent\""));
        assert!(!json.contains("\"author\""));
        assert!(!json.contains("\"author_id\""));
    }

    #[test]
    fn test_cursor_serialization() {
        let entry = CursorEntry::new("ABC125".to_string());
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"cursor\":true"));
        assert!(json.contains("\"role\":\"agent\""));
        assert!(!json.contains("\"content\""));
    }

    #[test]
    fn test_channel_entry_is_agent() {
        let msg = ChannelEntry::Message(MessageEntry::agent(
            "1".to_string(), "hi".to_string(), "discord".to_string(), None,
        ));
        assert!(msg.is_agent());

        let other = ChannelEntry::Message(MessageEntry::incoming(
            "2".to_string(), "user".to_string(), "u1".to_string(),
            "hello".to_string(), "discord".to_string(), None,
        ));
        assert!(!other.is_agent());

        let cursor = ChannelEntry::Cursor(CursorEntry::new("3".to_string()));
        assert!(cursor.is_agent());
    }

    #[test]
    fn test_roundtrip_message() {
        let entry = MessageEntry::incoming(
            "ABC123".to_string(),
            "cassie".to_string(),
            "12345".to_string(),
            "hello world".to_string(),
            "discord".to_string(),
            Some("msg_001".to_string()),
        );
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: MessageEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "ABC123");
        assert_eq!(parsed.role, "other");
        assert_eq!(parsed.author.unwrap(), "cassie");
        assert_eq!(parsed.content, "hello world");
    }

    #[test]
    fn test_roundtrip_cursor() {
        let entry = CursorEntry::new("ABC125".to_string());
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: CursorEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "ABC125");
        assert!(parsed.cursor);
    }
}
```

Create `crates/river-gateway/src/channels/mod.rs`:

```rust
//! Channel log management — JSONL read/write and cursor scanning

pub mod entry;
pub mod log;

pub use entry::{ChannelEntry, MessageEntry, CursorEntry};
pub use log::ChannelLog;
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cd ~/river-engine && cargo test -p river-gateway channels::entry`
Expected: All tests pass (these are self-contained serialization tests)

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/channels/
git commit -m "feat(channels): add JSONL entry types with serialization"
```

---

### Task 2: Channel Log Read/Write

JSONL append, read-from-cursor, and malformed line handling.

**Files:**
- Create: `crates/river-gateway/src/channels/log.rs`
- Modify: `crates/river-gateway/src/channels/mod.rs`

- [ ] **Step 1: Write channel log with tests**

Create `crates/river-gateway/src/channels/log.rs`:

```rust
//! Channel log — JSONL file operations
//!
//! One JSONL file per channel at channels/{adapter}_{channel_id}.jsonl
//! Handles append, read-from-cursor, and malformed line skipping.
//!
//! Uses tokio::fs for async I/O — this module is called from async contexts
//! (agent task, HTTP handler) and must not block the executor.

use super::entry::{ChannelEntry, MessageEntry, CursorEntry};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Manages a single channel's JSONL log file
pub struct ChannelLog {
    path: PathBuf,
}

/// Sanitize a string for use in a filename — alphanumeric, dash, underscore only
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

impl ChannelLog {
    /// Open a channel log at the standard path: {channels_dir}/{adapter}_{channel_id}.jsonl
    pub fn open(channels_dir: &Path, adapter: &str, channel_id: &str) -> Self {
        let filename = format!("{}_{}.jsonl", sanitize(adapter), sanitize(channel_id));
        Self {
            path: channels_dir.join(filename),
        }
    }

    /// Open a channel log at an explicit path (for testing)
    pub fn from_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// Append a serialized entry as a single JSONL line
    pub async fn append_entry(&self, entry: &impl serde::Serialize) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }

        let json = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;

        file.write_all(json.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
        Ok(())
    }

    /// Read all entries from the log, skipping malformed lines
    pub async fn read_all(&self) -> std::io::Result<Vec<ChannelEntry>> {
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
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ChannelEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!(
                        line_num = line_num,
                        error = %e,
                        path = %self.path.display(),
                        "Skipping malformed JSONL line"
                    );
                }
            }
        }

        Ok(entries)
    }

    /// Read new entries since the agent's last cursor position.
    ///
    /// Scans backward for the last role:agent entry, returns everything after it.
    /// If no agent entry exists, returns the last `default_window` entries.
    pub async fn read_since_cursor(&self, default_window: usize) -> std::io::Result<Vec<ChannelEntry>> {
        let all = self.read_all().await?;

        // Find the last agent entry (message or cursor)
        let last_agent_idx = all.iter().rposition(|e| e.is_agent());

        match last_agent_idx {
            Some(idx) => {
                // Return everything after the cursor
                Ok(all[idx + 1..].to_vec())
            }
            None => {
                // No cursor — return last N entries
                let start = all.len().saturating_sub(default_window);
                Ok(all[start..].to_vec())
            }
        }
    }

    /// Get the path to this channel log
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_log(dir: &TempDir) -> ChannelLog {
        ChannelLog::open(dir.path(), "discord", "general")
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("discord"), "discord");
        assert_eq!(sanitize("my-channel"), "my-channel");
        assert_eq!(sanitize("guild/channel"), "guild_channel");
        assert_eq!(sanitize("a:b:c"), "a_b_c");
    }

    #[test]
    fn test_channel_log_path() {
        let dir = TempDir::new().unwrap();
        let log = ChannelLog::open(dir.path(), "discord", "general");
        assert!(log.path().ends_with("discord_general.jsonl"));
    }

    #[tokio::test]
    async fn test_append_and_read() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        let entry = MessageEntry::incoming(
            "001".to_string(),
            "cassie".to_string(),
            "u1".to_string(),
            "hello".to_string(),
            "discord".to_string(),
            None,
        );
        log.append_entry(&entry).await.unwrap();

        let entries = log.read_all().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].is_agent());
    }

    #[tokio::test]
    async fn test_read_empty_log() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);
        let entries = log.read_all().await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_read_since_cursor_with_agent_message() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        // 3 messages from others, then agent speaks, then 2 more from others
        log.append_entry(&MessageEntry::incoming(
            "001".into(), "alice".into(), "a1".into(), "hi".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "002".into(), "bob".into(), "b1".into(), "hey".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "003".into(), "carol".into(), "c1".into(), "sup".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::agent(
            "004".into(), "hello everyone".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "005".into(), "alice".into(), "a1".into(), "nice".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "006".into(), "bob".into(), "b1".into(), "cool".into(), "discord".into(), None,
        )).await.unwrap();

        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].id(), "005");
        assert_eq!(new[1].id(), "006");
    }

    #[tokio::test]
    async fn test_read_since_cursor_with_cursor_entry() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        log.append_entry(&MessageEntry::incoming(
            "001".into(), "alice".into(), "a1".into(), "hi".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&CursorEntry::new("002".into())).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "003".into(), "bob".into(), "b1".into(), "hey".into(), "discord".into(), None,
        )).await.unwrap();

        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id(), "003");
    }

    #[tokio::test]
    async fn test_read_since_cursor_no_agent_entry() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        // 100 messages, no agent entry
        for i in 0..100 {
            log.append_entry(&MessageEntry::incoming(
                format!("{:03}", i), "user".into(), "u1".into(),
                format!("msg {}", i), "discord".into(), None,
            )).await.unwrap();
        }

        // Default window of 50
        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 50);
        assert_eq!(new[0].id(), "050");
        assert_eq!(new[49].id(), "099");
    }

    #[tokio::test]
    async fn test_malformed_line_skipped() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        // Write a valid entry
        log.append_entry(&MessageEntry::incoming(
            "001".into(), "alice".into(), "a1".into(), "hi".into(), "discord".into(), None,
        )).await.unwrap();

        // Write a malformed line directly using std::fs (sync, for test setup)
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(log.path()).unwrap();
        writeln!(file, "{{this is not valid json").unwrap();

        // Write another valid entry
        log.append_entry(&MessageEntry::incoming(
            "003".into(), "bob".into(), "b1".into(), "hey".into(), "discord".into(), None,
        )).await.unwrap();

        let entries = log.read_all().await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id(), "001");
        assert_eq!(entries[1].id(), "003");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd ~/river-engine && cargo test -p river-gateway channels::log`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/channels/
git commit -m "feat(channels): add JSONL log read/write with cursor scanning"
```

---

### Task 3: Notification Queue

Replace the `MessageQueue` payload from `IncomingMessage` to a lightweight notification.

**Files:**
- Modify: `crates/river-gateway/src/queue.rs`

- [ ] **Step 1: Rewrite queue.rs with notification payload**

Replace the contents of `crates/river-gateway/src/queue.rs`:

```rust
//! Thread-safe notification queue
//!
//! Carries lightweight notifications (channel + snowflake ID) to wake the agent.
//! The agent reads the actual message content from the channel log.

use std::collections::VecDeque;
use std::sync::Mutex;

/// A lightweight notification that a channel has a new message
#[derive(Debug, Clone)]
pub struct ChannelNotification {
    /// Channel identifier (e.g., "discord_general")
    pub channel: String,
    /// Snowflake ID of the new message
    pub snowflake_id: String,
}

/// Thread-safe notification queue
///
/// Notifications are processed in FIFO order.
pub struct MessageQueue {
    inner: Mutex<VecDeque<ChannelNotification>>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Push a notification
    pub fn push(&self, notification: ChannelNotification) {
        let mut queue = self.inner.lock().unwrap();
        queue.push_back(notification);
    }

    /// Drain all notifications
    pub fn drain(&self) -> Vec<ChannelNotification> {
        let mut queue = self.inner.lock().unwrap();
        queue.drain(..).collect()
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        let queue = self.inner.lock().unwrap();
        queue.is_empty()
    }

    /// Get queue length
    pub fn len(&self) -> usize {
        let queue = self.inner.lock().unwrap();
        queue.len()
    }
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_queue_is_empty() {
        let queue = MessageQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_push_and_drain() {
        let queue = MessageQueue::new();

        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            snowflake_id: "001".to_string(),
        });
        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            snowflake_id: "002".to_string(),
        });

        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 2);

        let notifications = queue.drain();
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].snowflake_id, "001");
        assert_eq!(notifications[1].snowflake_id, "002");

        assert!(queue.is_empty());
    }

    #[test]
    fn test_drain_empty_queue() {
        let queue = MessageQueue::new();
        let notifications = queue.drain();
        assert!(notifications.is_empty());
    }

    #[test]
    fn test_fifo_order() {
        let queue = MessageQueue::new();

        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            snowflake_id: "first".to_string(),
        });
        queue.push(ChannelNotification {
            channel: "discord_dm".to_string(),
            snowflake_id: "second".to_string(),
        });
        queue.push(ChannelNotification {
            channel: "discord_general".to_string(),
            snowflake_id: "third".to_string(),
        });

        let notifications = queue.drain();
        assert_eq!(notifications[0].snowflake_id, "first");
        assert_eq!(notifications[1].snowflake_id, "second");
        assert_eq!(notifications[2].snowflake_id, "third");
    }

    #[test]
    fn test_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let queue = Arc::new(MessageQueue::new());
        let mut handles = vec![];

        for i in 0..10 {
            let q = queue.clone();
            handles.push(thread::spawn(move || {
                q.push(ChannelNotification {
                    channel: "test".to_string(),
                    snowflake_id: format!("{}", i),
                });
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(queue.len(), 10);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd ~/river-engine && cargo test -p river-gateway queue`
Expected: All tests pass. Note: other code that imports from `queue.rs` will fail to compile until updated in later tasks.

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/queue.rs
git commit -m "refactor(queue): replace IncomingMessage payload with ChannelNotification"
```

---

### Task 4: Wire handle_incoming to Channel Log + Queue

Replace the inbox file write with JSONL append and notification push.

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`
- Modify: `crates/river-gateway/src/state.rs`
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Add `channels` module to lib.rs**

Add to `crates/river-gateway/src/lib.rs`:

```rust
pub mod channels;
```

- [ ] **Step 2: Update handle_incoming in routes.rs**

Replace the `handle_incoming` function body in `crates/river-gateway/src/api/routes.rs`. Keep the `IncomingMessage` and `Author` structs (adapters still send this format). Replace the inbox write + missing queue push with channel log append + notification push:

```rust
async fn handle_incoming(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(msg): Json<IncomingMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    tracing::info!(
        adapter = %msg.adapter,
        channel = %msg.channel,
        author_name = %msg.author.name,
        content_len = msg.content.len(),
        "Received incoming message"
    );

    // Validate authentication
    if let Err(status) = validate_auth(&headers, state.auth_token.as_deref()) {
        return Err(status);
    }

    // Generate snowflake ID
    let snowflake = state.snowflake_gen.next_id(river_core::SnowflakeType::Message);
    let snowflake_str = snowflake.to_string();

    // Build channel log entry
    let entry = crate::channels::MessageEntry::incoming(
        snowflake_str.clone(),
        msg.author.name.clone(),
        msg.author.id.clone(),
        msg.content.clone(),
        msg.adapter.clone(),
        msg.message_id.clone(),
    );

    // Append to channel log
    let channels_dir = state.config.workspace.join("channels");
    let log = crate::channels::ChannelLog::open(&channels_dir, &msg.adapter, &msg.channel);

    if let Err(e) = log.append_entry(&entry).await {
        tracing::error!(error = %e, "Failed to write to channel log");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Only push notification after successful write
    let channel_key = format!("{}_{}", msg.adapter, msg.channel);
    state.message_queue.push(crate::queue::ChannelNotification {
        channel: channel_key.clone(),
        snowflake_id: snowflake_str,
    });

    tracing::info!(channel = %channel_key, "Message delivered to channel log");

    Ok(Json(serde_json::json!({
        "status": "delivered",
        "channel": channel_key,
    })))
}
```

- [ ] **Step 3: Remove inbox imports from routes.rs**

Remove the inbox import at the top of `routes.rs`:

```rust
// DELETE this line:
use crate::inbox::{format_inbox_line, build_discord_path, append_line, sanitize_name};
```

- [ ] **Step 4: Run compilation check**

Run: `cd ~/river-engine && cargo check -p river-gateway 2>&1 | head -30`
Expected: May have errors from other files still importing old queue types. The routes.rs changes should compile. Fix any compilation errors in this file first.

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/api/routes.rs crates/river-gateway/src/lib.rs
git commit -m "feat(incoming): wire handle_incoming to channel log + notification queue"
```

---

### Task 5: Update Agent Turn Cycle

Replace the agent's message drain with channel log reading.

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`

- [ ] **Step 1: Update the WAKE section of turn_cycle**

In `AgentTask::turn_cycle()`, replace the incoming message drain (lines ~223-238) with channel log reading:

```rust
        // ========== WAKE ==========
        self.flash_queue.tick_turn().await;
        let channel_name = self.channel_context
            .as_ref()
            .map(|c| c.display_name().to_string())
            .unwrap_or_else(|| "default".to_string());

        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: channel_name.clone(),
            turn_number: self.turn_count,
            timestamp: Utc::now(),
        }));

        tracing::info!(
            turn = self.turn_count,
            channel = %channel_name,
            is_heartbeat = is_heartbeat,
            "Turn started"
        );

        // Drain notifications and read channel logs
        let notifications = self.message_queue.drain();
        let channels_dir = self.config.workspace.join("channels");

        // Deduplicate channels (multiple notifications for same channel)
        let mut seen_channels = std::collections::HashSet::new();
        for notification in &notifications {
            seen_channels.insert(notification.channel.clone());
        }

        // Read new messages from each channel
        let mut all_new_messages = Vec::new();
        for channel_key in &seen_channels {
            // Parse adapter and channel_id from the key
            let parts: Vec<&str> = channel_key.splitn(2, '_').collect();
            if parts.len() != 2 {
                tracing::warn!(channel = %channel_key, "Invalid channel key format");
                continue;
            }
            let log = crate::channels::ChannelLog::open(&channels_dir, parts[0], parts[1]);
            match log.read_since_cursor(50).await {
                Ok(entries) => {
                    for entry in entries {
                        if let crate::channels::ChannelEntry::Message(msg) = entry {
                            if !msg.is_agent() {
                                all_new_messages.push((channel_key.clone(), msg));
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(channel = %channel_key, error = %e, "Failed to read channel log");
                }
            }
        }

        // Add messages to context
        for (channel, msg) in &all_new_messages {
            let author = msg.author.as_deref().unwrap_or("unknown");
            let chat_msg = ChatMessage::user(format!(
                "[{}] {}: {}",
                channel, author, msg.content
            ));
            self.context.append(ContextMessage::new(chat_msg, self.turn_count));
        }

        // Add heartbeat trigger if applicable
        if is_heartbeat && all_new_messages.is_empty() {
            self.context.append(ContextMessage::new(
                ChatMessage::user(":heartbeat:"),
                self.turn_count,
            ));
        }
```

- [ ] **Step 2: Add cursor write after turn completes (in SETTLE section)**

After the think/act loop, before `persist_turn_messages()`, write cursor entries for channels the agent read but didn't speak in. Add a field to track which channels the agent spoke in during the turn:

This is a larger change — for now, write a cursor for every channel that had notifications. If the agent spoke in a channel (via send_message tool), the agent message entry is the cursor and the explicit cursor is redundant but harmless.

```rust
        // ========== SETTLE ==========
        // Write cursor entries for channels we read
        for channel_key in &seen_channels {
            let parts: Vec<&str> = channel_key.splitn(2, '_').collect();
            if parts.len() == 2 {
                let log = crate::channels::ChannelLog::open(&channels_dir, parts[0], parts[1]);
                let cursor_id = self.snowflake_gen.next_id(river_core::SnowflakeType::Message).to_string();
                let cursor = crate::channels::CursorEntry::new(cursor_id);
                if let Err(e) = log.append_entry(&cursor).await {
                    tracing::warn!(channel = %channel_key, error = %e, "Failed to write cursor");
                }
            }
        }

        self.persist_turn_messages();
```

- [ ] **Step 3: Run compilation check**

Run: `cd ~/river-engine && cargo check -p river-gateway 2>&1 | head -30`
Expected: Should compile. Fix any type mismatches.

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/agent/task.rs
git commit -m "feat(agent): read channel logs on wake, write cursors on settle"
```

---

### Task 6: Update SendMessageTool to Log Agent Messages

When the agent sends a message, append a `role: agent` entry to the channel log.

**Files:**
- Modify: `crates/river-gateway/src/tools/communication.rs`

- [ ] **Step 1: Update SendMessageTool::execute**

In the `SendMessageTool`'s `execute` method, after a successful send to the adapter (where it receives `message_id` back), append the agent's message to the channel log:

```rust
// After successful send_to_adapter, before returning the tool result:
let channels_dir = self.workspace.join("channels");
let log = crate::channels::ChannelLog::open(&channels_dir, &adapter_name, &channel);
let snowflake = self.snowflake_gen.next_id(river_core::SnowflakeType::Message);
let agent_entry = crate::channels::MessageEntry::agent(
    snowflake.to_string(),
    content.clone(),
    adapter_name.clone(),
    Some(returned_message_id.clone()),
);
if let Err(e) = log.append_entry(&agent_entry).await {
    tracing::warn!(error = %e, "Failed to log agent message to channel");
}
```

**Constructor changes required:** `SendMessageTool` and `SpeakTool` need access to `SnowflakeGenerator`. In `server.rs`, the tools are constructed with various fields. Add `snowflake_gen: Arc<SnowflakeGenerator>` to both tool structs and pass `snowflake_gen.clone()` in their constructors in `server.rs::run()` (where `snowflake_gen` is already available as a local variable).

- [ ] **Step 2: Apply same change to SpeakTool**

The `SpeakTool` is similar — it sends to the current channel. Apply the same channel log append after successful send.

- [ ] **Step 3: Run compilation check**

Run: `cd ~/river-engine && cargo check -p river-gateway 2>&1 | head -30`

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/tools/communication.rs
git commit -m "feat(tools): log agent messages to channel JSONL on send"
```

---

### Task 7: Remove Old Inbox/Conversations Code

Clean up the code that the new channel system replaces.

**Files:**
- Modify: `crates/river-gateway/src/lib.rs` — remove `pub mod inbox;` and `pub mod conversations;`
- Modify: `crates/river-gateway/src/server.rs` — remove inbox migration logic and ConversationWriter spawn
- Delete: `crates/river-gateway/src/inbox/` (entire directory)
- Delete: `crates/river-gateway/src/conversations/` (entire directory)

- [ ] **Step 1: Remove module declarations from lib.rs**

In `crates/river-gateway/src/lib.rs`, remove:

```rust
// DELETE these lines:
pub mod inbox;
pub mod conversations;
```

- [ ] **Step 2: Remove inbox migration and ConversationWriter from server.rs**

In `crates/river-gateway/src/server.rs`, remove:
- The `inbox_path`/`conversations_path` migration block (lines ~65-71)
- The `ConversationWriter` spawn (lines ~243-251)
- The `conv_writer_tx` references in tool registrations

- [ ] **Step 3: Remove inbox and conversations directories**

```bash
rm -rf crates/river-gateway/src/inbox/
rm -rf crates/river-gateway/src/conversations/
```

- [ ] **Step 4: Refactor SyncConversationTool**

`crates/river-gateway/src/tools/sync.rs` — `SyncConversationTool` depends heavily on the old `conversations` module (`WriteOp`, `build_discord_path`, etc.). Refactor it to use `ChannelLog::append_entry` instead of `mpsc::Sender<WriteOp>`. The tool fetches message history from the adapter and writes it to the channel log. Replace:
- `conv_writer_tx: mpsc::Sender<WriteOp>` → `workspace: PathBuf` + `snowflake_gen: Arc<SnowflakeGenerator>`
- `WriteOp` sends → `ChannelLog::append_entry` calls
- Discord-specific path building → `ChannelLog::open` with adapter + channel_id

Update the constructor call in `server.rs` to pass `workspace` and `snowflake_gen` instead of `conv_writer_tx`.

- [ ] **Step 5: Fix all remaining compilation errors**

Run: `cd ~/river-engine && cargo check -p river-gateway 2>&1`

Fix any remaining references to the old inbox/conversations modules. Common locations:
- `server.rs` — `conv_writer_tx` in SendMessageTool constructor (should already be removed if Task 6 was done correctly)
- `tools/communication.rs` — any remaining ConversationWriter references

- [ ] **Step 6: Run full test suite**

Run: `cd ~/river-engine && cargo test -p river-gateway`
Expected: All tests pass. Some old tests referencing inbox format will need to be removed — these are replaced by the channel log tests in Tasks 1, 2, and 8.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "cleanup: remove old inbox and conversations modules, refactor SyncConversationTool"
```

---

### Task 8: Integration Test

End-to-end test: incoming message → channel log → notification → agent reads log.

**Files:**
- Create: `crates/river-gateway/src/channels/integration_test.rs` (or add to existing test module)

- [ ] **Step 1: Write integration test**

Add to `crates/river-gateway/src/channels/log.rs` tests module:

```rust
    #[tokio::test]
    async fn test_full_flow_incoming_cursor_read() {
        let dir = TempDir::new().unwrap();
        let log = ChannelLog::open(dir.path(), "discord", "general");

        // Simulate: 3 messages arrive, agent reads (cursor), 2 more arrive
        log.append_entry(&MessageEntry::incoming(
            "001".into(), "alice".into(), "a1".into(),
            "hello".into(), "discord".into(), Some("d001".into()),
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "002".into(), "bob".into(), "b1".into(),
            "hi there".into(), "discord".into(), Some("d002".into()),
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "003".into(), "carol".into(), "c1".into(),
            "hey all".into(), "discord".into(), Some("d003".into()),
        )).await.unwrap();

        // Agent reads — should get all 3 (no prior cursor)
        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 3);

        // Agent speaks — implicit cursor
        log.append_entry(&MessageEntry::agent(
            "004".into(), "hello everyone!".into(),
            "discord".into(), Some("d004".into()),
        )).await.unwrap();

        // Two more messages arrive
        log.append_entry(&MessageEntry::incoming(
            "005".into(), "alice".into(), "a1".into(),
            "how are you?".into(), "discord".into(), Some("d005".into()),
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "006".into(), "bob".into(), "b1".into(),
            "doing great".into(), "discord".into(), Some("d006".into()),
        )).await.unwrap();

        // Agent reads again — should only get the 2 new messages
        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].id(), "005");
        assert_eq!(new[1].id(), "006");

        // Agent reads but doesn't speak — writes cursor
        log.append_entry(&CursorEntry::new("007".into())).await.unwrap();

        // One more message
        log.append_entry(&MessageEntry::incoming(
            "008".into(), "carol".into(), "c1".into(),
            "late message".into(), "discord".into(), Some("d008".into()),
        )).await.unwrap();

        // Agent reads — should get 1 new message (after cursor)
        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id(), "008");
    }
```

- [ ] **Step 2: Run tests**

Run: `cd ~/river-engine && cargo test -p river-gateway channels`
Expected: All tests pass

- [ ] **Step 3: Run full test suite**

Run: `cd ~/river-engine && cargo test`
Expected: All tests across all crates pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "test(channels): add integration test for full message flow"
```

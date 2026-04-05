# Context Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the context architecture redesign where context.jsonl stores only LLM outputs, tool results go to workspace/inbox/, and context is assembled fresh on channel switch.

**Architecture:** Split persistence into two paths: stream of consciousness (context.jsonl with assistant messages + system warnings only) and tool results (workspace/inbox/ with timestamped JSON files). Modify worker_loop to persist selectively and rebuild context on channel switch.

**Tech Stack:** Rust, tokio, serde_json, river-context, river-worker

---

## File Structure

**Modified files:**
- `crates/river-worker/src/persistence.rs` — Add selective persistence (assistant/system only), add inbox read/write
- `crates/river-worker/src/worker_loop.rs` — Change persistence logic, add context rebuild on channel switch
- `crates/river-worker/src/tools.rs` — Add inbox writes to read_history, create_move, create_moment, search_embeddings
- `crates/river-context/src/format.rs` — Add format_inbox_item function
- `crates/river-context/src/workspace.rs` — Add InboxItem type
- `crates/river-context/src/assembly.rs` — Include inbox items in timeline for current channel
- `crates/river-context/src/request.rs` — Add inbox field to ChannelContext
- `crates/river-context/src/lib.rs` — Re-export InboxItem

**New files:**
- `crates/river-worker/src/inbox.rs` — Inbox read/write utilities

---

### Task 1: Add InboxItem Type

**Files:**
- Modify: `crates/river-context/src/workspace.rs`
- Modify: `crates/river-context/src/lib.rs`

- [ ] **Step 1: Write the test for InboxItem serialization**

```rust
// Add to crates/river-context/src/workspace.rs at the end

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inbox_item_serialization() {
        let item = InboxItem {
            id: "discord_chan123_2026-04-01T07-28-00Z_read_channel".into(),
            timestamp: "2026-04-01T07:28:00Z".into(),
            tool: "read_channel".into(),
            channel_adapter: "discord".into(),
            channel_id: "chan123".into(),
            summary: "msg1150-msg1200".into(),
        };

        let json = serde_json::to_string(&item).unwrap();
        let parsed: InboxItem = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tool, "read_channel");
        assert_eq!(parsed.summary, "msg1150-msg1200");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-context test_inbox_item_serialization`
Expected: FAIL with "cannot find type `InboxItem`"

- [ ] **Step 3: Add InboxItem struct**

Add to `crates/river-context/src/workspace.rs` after the Embedding struct:

```rust
/// An inbox item recording a tool result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboxItem {
    /// Unique ID (filename stem).
    pub id: String,
    /// ISO8601 timestamp.
    pub timestamp: String,
    /// Tool name (read_channel, create_move, etc).
    pub tool: String,
    /// Channel adapter.
    pub channel_adapter: String,
    /// Channel ID.
    pub channel_id: String,
    /// Human-readable summary (e.g., "msg1150-msg1200").
    pub summary: String,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-context test_inbox_item_serialization`
Expected: PASS

- [ ] **Step 5: Re-export InboxItem from lib.rs**

In `crates/river-context/src/lib.rs`, change the workspace re-export line:

```rust
pub use workspace::{ChatMessage, Embedding, Flash, InboxItem, Moment, Move};
```

- [ ] **Step 6: Verify compilation**

Run: `cargo build -p river-context`
Expected: Compiles successfully

- [ ] **Step 7: Commit**

```bash
git add crates/river-context/src/workspace.rs crates/river-context/src/lib.rs
git commit -m "feat(river-context): add InboxItem type for tool results"
```

---

### Task 2: Add format_inbox_item Function

**Files:**
- Modify: `crates/river-context/src/format.rs`

- [ ] **Step 1: Write the test for format_inbox_item**

Add to the tests module in `crates/river-context/src/format.rs`:

```rust
    #[test]
    fn test_format_inbox_item() {
        let item = crate::workspace::InboxItem {
            id: "discord_chan123_2026-04-01T07-28-00Z_read_channel".into(),
            timestamp: "2026-04-01T07:28:00Z".into(),
            tool: "read_channel".into(),
            channel_adapter: "discord".into(),
            channel_id: "chan123".into(),
            summary: "msg1150-msg1200".into(),
        };

        let msg = format_inbox_item(&item);

        assert_eq!(msg.role, "system");
        assert!(msg.content.as_ref().unwrap().contains("[inbox]"));
        assert!(msg.content.as_ref().unwrap().contains("07:28"));
        assert!(msg.content.as_ref().unwrap().contains("read_channel"));
        assert!(msg.content.as_ref().unwrap().contains("msg1150-msg1200"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-context test_format_inbox_item`
Expected: FAIL with "cannot find function `format_inbox_item`"

- [ ] **Step 3: Add format_inbox_item function**

Add to `crates/river-context/src/format.rs` after format_embedding:

```rust
use crate::workspace::InboxItem;

/// Format an inbox item as a system message.
pub fn format_inbox_item(item: &InboxItem) -> OpenAIMessage {
    // Extract time portion from timestamp (e.g., "07:28" from "2026-04-01T07:28:00Z")
    let time = item.timestamp
        .split('T')
        .nth(1)
        .map(|t| {
            let parts: Vec<&str> = t.split(':').take(2).collect();
            parts.join(":")
        })
        .unwrap_or_else(|| item.timestamp.clone());

    OpenAIMessage::system(format!(
        "[inbox] {} {}: {}",
        time, item.tool, item.summary
    ))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-context test_format_inbox_item`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/river-context/src/format.rs
git commit -m "feat(river-context): add format_inbox_item for tool result display"
```

---

### Task 3: Add inbox Field to ChannelContext

**Files:**
- Modify: `crates/river-context/src/request.rs`

- [ ] **Step 1: Read current ChannelContext struct**

Read `crates/river-context/src/request.rs` to see current structure.

- [ ] **Step 2: Add inbox field to ChannelContext**

In `crates/river-context/src/request.rs`, add the inbox field to ChannelContext:

```rust
use crate::workspace::{ChatMessage, Embedding, InboxItem, Moment, Move};

/// Context for a single channel.
#[derive(Clone, Debug)]
pub struct ChannelContext {
    pub channel: Channel,
    pub moments: Vec<Moment>,
    pub moves: Vec<Move>,
    pub messages: Vec<ChatMessage>,
    pub embeddings: Vec<Embedding>,
    pub inbox: Vec<InboxItem>,
}
```

- [ ] **Step 3: Fix compilation errors in tests**

Update any test code that constructs ChannelContext to include `inbox: vec![]`.

Run: `cargo build -p river-context`
Fix any compilation errors by adding the inbox field.

- [ ] **Step 4: Verify all tests pass**

Run: `cargo test -p river-context`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-context/src/request.rs
git commit -m "feat(river-context): add inbox field to ChannelContext"
```

---

### Task 4: Include Inbox Items in Timeline Assembly

**Files:**
- Modify: `crates/river-context/src/assembly.rs`

- [ ] **Step 1: Write test for inbox items in timeline**

Add to tests in `crates/river-context/src/assembly.rs`:

```rust
    #[test]
    fn test_inbox_items_interspersed_by_timestamp() {
        use crate::workspace::InboxItem;

        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![
                    test_moment(&make_id(1000), "Moment 1"),
                    test_moment(&make_id(3000), "Moment 2"),
                ],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
                inbox: vec![InboxItem {
                    id: make_id(2000),
                    timestamp: "2026-04-01T07:28:00Z".into(),
                    tool: "read_channel".into(),
                    channel_adapter: "discord".into(),
                    channel_id: "main".into(),
                    summary: "msg1150-msg1200".into(),
                }],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        assert_eq!(result.messages.len(), 3);

        // Inbox item should be between the two moments
        let content_0 = result.messages[0].content.as_ref().unwrap();
        let content_1 = result.messages[1].content.as_ref().unwrap();
        let content_2 = result.messages[2].content.as_ref().unwrap();

        assert!(content_0.contains("Moment 1")); // timestamp 1000
        assert!(content_1.contains("[inbox]")); // timestamp 2000
        assert!(content_2.contains("Moment 2")); // timestamp 3000
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-context test_inbox_items_interspersed`
Expected: FAIL (inbox items not included)

- [ ] **Step 3: Add inbox items to current channel collection**

In `crates/river-context/src/assembly.rs`, add the import and modify `collect_channel_summary` or add a new function:

Add import at top:
```rust
use crate::format::format_inbox_item;
```

Add new function after `collect_channel_embeddings`:
```rust
fn collect_channel_inbox(
    timeline: &mut Vec<TimelineItem>,
    ctx: &ChannelContext,
) {
    for item in &ctx.inbox {
        timeline.push(TimelineItem::new(
            &item.id,
            format_inbox_item(item),
        ));
    }
}
```

In `build_context`, after collecting embeddings for current channel (around line 87), add:
```rust
    // Process current channel (index 0): moments + moves + embeddings + inbox
    let current_ctx = &request.channels[0];
    collect_channel_summary(&mut timeline, current_ctx);
    collect_channel_embeddings(&mut timeline, current_ctx, &now_dt);
    collect_channel_inbox(&mut timeline, current_ctx);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-context test_inbox_items_interspersed`
Expected: PASS

- [ ] **Step 5: Run all river-context tests**

Run: `cargo test -p river-context`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-context/src/assembly.rs
git commit -m "feat(river-context): include inbox items in timeline for current channel"
```

---

### Task 5: Create Inbox Module in river-worker

**Files:**
- Create: `crates/river-worker/src/inbox.rs`
- Modify: `crates/river-worker/src/main.rs`

- [ ] **Step 1: Create inbox.rs with write function**

Create `crates/river-worker/src/inbox.rs`:

```rust
//! Inbox utilities for storing and loading tool results.

use river_context::InboxItem;
use std::path::Path;
use tokio::fs;

/// Write a tool result to the inbox.
///
/// Filename format: {adapter}_{channel_id}_{timestamp}_{tool}.json
pub async fn write_inbox_item(
    workspace: &Path,
    adapter: &str,
    channel_id: &str,
    tool: &str,
    summary: &str,
) -> std::io::Result<InboxItem> {
    let timestamp = chrono::Utc::now();
    let timestamp_str = timestamp.format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let timestamp_iso = timestamp.to_rfc3339();

    let filename = format!("{}_{}_{}_{}",
        adapter,
        channel_id,
        timestamp_str,
        tool
    );

    let item = InboxItem {
        id: filename.clone(),
        timestamp: timestamp_iso,
        tool: tool.to_string(),
        channel_adapter: adapter.to_string(),
        channel_id: channel_id.to_string(),
        summary: summary.to_string(),
    };

    let inbox_dir = workspace.join("inbox");
    fs::create_dir_all(&inbox_dir).await?;

    let path = inbox_dir.join(format!("{}.json", filename));
    let json = serde_json::to_string_pretty(&item)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    fs::write(&path, json).await?;

    Ok(item)
}

/// Load all inbox items for a channel.
pub async fn load_inbox_items(
    workspace: &Path,
    adapter: &str,
    channel_id: &str,
) -> Vec<InboxItem> {
    let inbox_dir = workspace.join("inbox");
    let prefix = format!("{}_{}_", adapter, channel_id);

    let mut items = Vec::new();

    let mut entries = match fs::read_dir(&inbox_dir).await {
        Ok(e) => e,
        Err(_) => return items,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let filename = entry.file_name();
        let filename_str = filename.to_string_lossy();

        if filename_str.starts_with(&prefix) && filename_str.ends_with(".json") {
            if let Ok(content) = fs::read_to_string(entry.path()).await {
                if let Ok(item) = serde_json::from_str::<InboxItem>(&content) {
                    items.push(item);
                }
            }
        }
    }

    // Sort by timestamp
    items.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_write_and_load_inbox_item() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();

        let item = write_inbox_item(
            workspace,
            "discord",
            "chan123",
            "read_channel",
            "msg1150-msg1200",
        ).await.unwrap();

        assert_eq!(item.tool, "read_channel");
        assert_eq!(item.summary, "msg1150-msg1200");

        let loaded = load_inbox_items(workspace, "discord", "chan123").await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].summary, "msg1150-msg1200");
    }

    #[tokio::test]
    async fn test_load_inbox_items_filters_by_channel() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();

        write_inbox_item(workspace, "discord", "chan123", "read_channel", "a").await.unwrap();
        write_inbox_item(workspace, "discord", "chan456", "read_channel", "b").await.unwrap();

        let loaded = load_inbox_items(workspace, "discord", "chan123").await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].summary, "a");
    }
}
```

- [ ] **Step 2: Add mod declaration to main.rs**

In `crates/river-worker/src/main.rs`, add after other mod declarations:

```rust
mod inbox;
```

- [ ] **Step 3: Run tests to verify**

Run: `cargo test -p river-worker test_write_and_load_inbox_item`
Expected: PASS

Run: `cargo test -p river-worker test_load_inbox_items_filters`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/river-worker/src/inbox.rs crates/river-worker/src/main.rs
git commit -m "feat(river-worker): add inbox module for tool result storage"
```

---

### Task 6: Add Inbox Writes to read_history Tool

**Files:**
- Modify: `crates/river-worker/src/tools.rs`

- [ ] **Step 1: Add inbox write to execute_read_history**

In `crates/river-worker/src/tools.rs`, add at the top with other imports:

```rust
use crate::inbox::write_inbox_item;
```

In `execute_read_history`, after the success result is built (around line 1295), add inbox write before returning:

```rust
    // Write to inbox
    let summary = format!("{}-{}",
        oldest_id.as_deref().unwrap_or("?"),
        newest_id.as_deref().unwrap_or("?")
    );
    let _ = write_inbox_item(&workspace, &adapter, &channel, "read_history", &summary).await;

    ToolResult::Success(serde_json::to_value(ReadHistoryResult {
        success: true,
        messages_fetched: messages.len(),
        oldest_id,
        newest_id,
        error: None,
        retry_after_ms: None,
    }).unwrap())
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build -p river-worker`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add crates/river-worker/src/tools.rs
git commit -m "feat(river-worker): write inbox item on read_history"
```

---

### Task 7: Add Inbox Writes to create_move Tool

**Files:**
- Modify: `crates/river-worker/src/tools.rs`

- [ ] **Step 1: Add inbox write to execute_create_move**

In `execute_create_move`, after successfully writing the move (around line 967), add inbox write:

```rust
    // Write to inbox
    let summary = format!("{}-{}", start_id, end_id);
    let _ = write_inbox_item(&workspace, adapter, channel_id, "create_move", &summary).await;

    ToolResult::Success(serde_json::json!({
        "id": id_str,
        "created": true
    }))
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build -p river-worker`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add crates/river-worker/src/tools.rs
git commit -m "feat(river-worker): write inbox item on create_move"
```

---

### Task 8: Add Inbox Writes to create_moment Tool

**Files:**
- Modify: `crates/river-worker/src/tools.rs`

- [ ] **Step 1: Add inbox write to execute_create_moment**

In `execute_create_moment`, after successfully writing the moment (around line 1069), add inbox write:

```rust
    // Write to inbox
    let summary = format!("{}-{}", start_id, end_id);
    let _ = write_inbox_item(&workspace, adapter, channel_id, "create_moment", &summary).await;

    ToolResult::Success(serde_json::json!({
        "id": id_str,
        "created": true
    }))
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build -p river-worker`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add crates/river-worker/src/tools.rs
git commit -m "feat(river-worker): write inbox item on create_moment"
```

---

### Task 9: Modify Persistence to Store Only LLM Outputs

**Files:**
- Modify: `crates/river-worker/src/persistence.rs`

- [ ] **Step 1: Add helper function to check if message should be persisted**

Add to `crates/river-worker/src/persistence.rs`:

```rust
/// Check if a message should be persisted to context.jsonl.
///
/// Only persist:
/// - Assistant messages (LLM outputs)
/// - System messages that are context pressure warnings
pub fn should_persist(message: &OpenAIMessage) -> bool {
    match message.role.as_str() {
        "assistant" => true,
        "system" => {
            // Only persist context pressure warnings
            message.content.as_ref()
                .map(|c| c.contains("Context at"))
                .unwrap_or(false)
        }
        _ => false,
    }
}
```

- [ ] **Step 2: Add test for should_persist**

Add to tests module:

```rust
    #[test]
    fn test_should_persist_assistant() {
        let msg = OpenAIMessage::assistant("I'll help you with that.");
        assert!(should_persist(&msg));
    }

    #[test]
    fn test_should_persist_system_warning() {
        let msg = OpenAIMessage::system("Context at 80%. Consider wrapping up.");
        assert!(should_persist(&msg));
    }

    #[test]
    fn test_should_not_persist_user() {
        let msg = OpenAIMessage::user("Hello");
        assert!(!should_persist(&msg));
    }

    #[test]
    fn test_should_not_persist_tool_result() {
        let msg = OpenAIMessage::tool("call_123", "result");
        assert!(!should_persist(&msg));
    }

    #[test]
    fn test_should_not_persist_regular_system() {
        let msg = OpenAIMessage::system("You are a helpful assistant.");
        assert!(!should_persist(&msg));
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-worker should_persist`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-worker/src/persistence.rs
git commit -m "feat(river-worker): add should_persist filter for context.jsonl"
```

---

### Task 10: Update worker_loop to Use Selective Persistence

**Files:**
- Modify: `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 1: Import should_persist**

Add to imports in worker_loop.rs:

```rust
use crate::persistence::should_persist;
```

- [ ] **Step 2: Change append_to_context calls to check should_persist**

Find all calls to `append_to_context` in the file. For each one, wrap in a condition:

Change from:
```rust
append_to_context(&context_path, &msg).ok();
```

To:
```rust
if should_persist(&msg) {
    append_to_context(&context_path, &msg).ok();
}
```

Do this for ALL append_to_context calls except where explicitly needed.

- [ ] **Step 3: Verify compilation**

Run: `cargo build -p river-worker`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/river-worker/src/worker_loop.rs
git commit -m "feat(river-worker): use selective persistence in worker loop"
```

---

### Task 11: Load Inbox Items in workspace_loader

**Files:**
- Modify: `crates/river-worker/src/workspace_loader.rs`

- [ ] **Step 1: Add inbox loading to load_channel_context**

Add import at top:
```rust
use crate::inbox::load_inbox_items;
```

In `load_channel_context`, after loading messages, add inbox loading:

```rust
    // Load inbox items
    let inbox = load_inbox_items(workspace, &channel.adapter, &channel.id).await;

    Ok(ChannelContext {
        channel: channel.clone(),
        moments,
        moves,
        messages,
        embeddings: vec![],
        inbox,
    })
```

Also update the error fallback in `load_channels` to include `inbox: vec![]`.

- [ ] **Step 2: Verify compilation**

Run: `cargo build -p river-worker`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add crates/river-worker/src/workspace_loader.rs
git commit -m "feat(river-worker): load inbox items in workspace_loader"
```

---

### Task 12: Add Context Rebuild Flag for Channel Switch

**Files:**
- Modify: `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 1: Add rebuild flag handling**

In the `run_loop` function, after handling ChannelSwitch result (around line 303), set a flag that triggers rebuild:

Find the channel_switched handling:
```rust
                if channel_switched {
                    tracing::debug!("Channel switched, workspace context will be re-rendered next turn");
                }
```

Change to:
```rust
                // Handle channel switch - trigger full context rebuild
                if channel_switched {
                    tracing::debug!("Channel switched, rebuilding context");
                    // Clear live context - it will be rebuilt on next loop iteration
                    // The llm_history (stream of consciousness) is preserved
                    // but the assembled context needs to be rebuilt fresh
                }
```

The rebuild happens naturally because `assemble_full_context` is called every loop iteration.

- [ ] **Step 2: Verify the flow**

Read through run_loop to confirm that:
1. llm_history persists across channel switches (stored in context.jsonl)
2. assemble_full_context is called each loop, rebuilding from workspace
3. Channel switch updates state.current_channel, which affects next assembly

- [ ] **Step 3: Commit**

```bash
git add crates/river-worker/src/worker_loop.rs
git commit -m "docs(river-worker): clarify context rebuild on channel switch"
```

---

### Task 13: Update New Message Handling

**Files:**
- Modify: `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 1: Verify new message format uses conversation format**

Check that notifications are formatted correctly. In run_loop, around line 116-127, the notification handling should append messages in conversation format.

Current code creates a system message summarizing notifications. For the new architecture, new messages should be appended as user messages using the conversation format.

This may require changes to how notifications are processed. For now, leave as-is since the full message appears in the conversation file and is loaded via workspace_loader.

- [ ] **Step 2: Commit (documentation only)**

```bash
git add crates/river-worker/src/worker_loop.rs
git commit -m "docs(river-worker): document new message handling in context architecture"
```

---

### Task 14: Run Full Test Suite

**Files:**
- None (verification only)

- [ ] **Step 1: Run all river-context tests**

Run: `cargo test -p river-context`
Expected: All tests pass

- [ ] **Step 2: Run all river-worker tests**

Run: `cargo test -p river-worker`
Expected: All tests pass

- [ ] **Step 3: Run full workspace build**

Run: `cargo build --workspace`
Expected: Compiles successfully

- [ ] **Step 4: Run clippy**

Run: `cargo clippy --workspace`
Expected: No errors (warnings acceptable)

---

### Task 15: Final Integration Verification

**Files:**
- None (manual verification)

- [ ] **Step 1: Verify context.jsonl format**

Create a test context.jsonl and verify only assistant messages and context warnings are present:

```jsonl
{"role":"assistant","content":"I'll help with that.","tool_calls":null,"tool_call_id":null}
{"role":"system","content":"Context at 80%. Consider wrapping up.","tool_calls":null,"tool_call_id":null}
{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_channel","arguments":"{}"}}],"tool_call_id":null}
```

- [ ] **Step 2: Verify inbox structure**

Create test inbox files and verify they load correctly:

```
workspace/inbox/discord_chan123_2026-04-01T07-28-00Z_read_history.json
```

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "feat: complete context architecture redesign implementation"
```

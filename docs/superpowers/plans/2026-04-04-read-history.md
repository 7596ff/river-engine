# Read History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `read_history` tool that fetches message history from adapters, persists to conversation files, and returns pagination info to the LLM.

**Architecture:** Worker exposes tool gated by adapter feature. Tool calls adapter's `/execute` endpoint, adapter fetches from platform API (Discord), worker writes messages to conversation files using existing logic. Compaction handles deduplication.

**Tech Stack:** Rust, axum, reqwest, serde, existing river-* crates

---

## File Structure

| File | Responsibility |
|------|----------------|
| `river-adapter/src/feature.rs` | Add `after` param to `ReadHistory` |
| `river-adapter/src/response.rs` | Add `retry_after_ms`, extend `HistoryMessage` |
| `river-protocol/src/conversation/mod.rs` | Add dedupe to `compact()` |
| `river-worker/src/tools.rs` | Add `read_history` tool |
| `river-worker/src/state.rs` | Remove unused `since_id` |
| `river-discord/src/execute.rs` | Handle `ReadHistory` request |

---

## Task 1: Add `after` Parameter to ReadHistory Request

**Files:**
- Modify: `crates/river-adapter/src/feature.rs`

- [ ] **Step 1: Add `after` field to ReadHistory variant**

In `/home/cassie/river-engine/crates/river-adapter/src/feature.rs`, find `ReadHistory` and add the `after` field:

```rust
    ReadHistory {
        channel: String,
        limit: Option<u32>,
        before: Option<String>,
        after: Option<String>,
    },
```

- [ ] **Step 2: Update test in lib.rs**

In `/home/cassie/river-engine/crates/river-adapter/src/lib.rs`, find the `ReadHistory` test case around line 161 and update:

```rust
            (OutboundRequest::ReadHistory { channel: "ch".into(), limit: Some(10), before: None, after: None }, FeatureId::ReadHistory),
```

- [ ] **Step 3: Verify build**

Run: `cargo build -p river-adapter`
Expected: Success

- [ ] **Step 4: Commit**

```bash
git add crates/river-adapter/src/feature.rs crates/river-adapter/src/lib.rs
git commit -m "feat(river-adapter): add after param to ReadHistory request"
```

---

## Task 2: Extend HistoryMessage and Add Rate Limit Field

**Files:**
- Modify: `crates/river-adapter/src/response.rs`

- [ ] **Step 1: Extend HistoryMessage struct**

In `/home/cassie/river-engine/crates/river-adapter/src/response.rs`, update `HistoryMessage`:

```rust
/// Message from history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct HistoryMessage {
    pub message_id: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}
```

- [ ] **Step 2: Add retry_after_ms to ResponseError**

In the same file, add the field to `ResponseError`:

```rust
/// Error response details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ResponseError {
    pub code: ErrorCode,
    pub message: String,
    /// Rate limit backoff hint in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}
```

- [ ] **Step 3: Update ResponseError::new**

Update the constructor to default retry_after_ms to None:

```rust
impl ResponseError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after_ms: None,
        }
    }

    pub fn rate_limited(message: impl Into<String>, retry_after_ms: u64) -> Self {
        Self {
            code: ErrorCode::RateLimited,
            message: message.into(),
            retry_after_ms: Some(retry_after_ms),
        }
    }
}
```

- [ ] **Step 4: Verify build**

Run: `cargo build -p river-adapter`
Expected: Success

- [ ] **Step 5: Commit**

```bash
git add crates/river-adapter/src/response.rs
git commit -m "feat(river-adapter): extend HistoryMessage and add rate limit support"
```

---

## Task 3: Add Dedupe to Compaction

**Files:**
- Modify: `crates/river-protocol/src/conversation/mod.rs`
- Test: same file (inline tests)

- [ ] **Step 1: Write test for dedupe**

In `/home/cassie/river-engine/crates/river-protocol/src/conversation/mod.rs`, add test in the `mod tests` block:

```rust
    #[test]
    fn test_compact_dedupes_by_message_id() {
        let mut convo = Conversation::default();
        // Add same message ID twice with different content
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "first".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:01".to_string(),
            id: "msg1".to_string(), // same ID
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "duplicate".to_string(),
            reactions: vec![],
        }));

        convo.compact();

        assert_eq!(convo.lines.len(), 1);
        if let Line::Message(msg) = &convo.lines[0] {
            assert_eq!(msg.content, "first"); // first occurrence wins
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_compact_keeps_unique_messages() {
        let mut convo = Conversation::default();
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author { name: "a".to_string(), id: "1".to_string(), bot: false },
            content: "first".to_string(),
            reactions: vec![],
        }));
        convo.lines.push(Line::Message(Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:01".to_string(),
            id: "msg2".to_string(), // different ID
            author: Author { name: "b".to_string(), id: "2".to_string(), bot: false },
            content: "second".to_string(),
            reactions: vec![],
        }));

        convo.compact();

        assert_eq!(convo.lines.len(), 2);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-protocol compact_dedupes`
Expected: FAIL (duplicates not removed yet)

- [ ] **Step 3: Update compact() to dedupe**

In the same file, update the `compact` method:

```rust
    /// Compact: apply read receipts to messages, sort by timestamp, remove receipts, dedupe by ID.
    pub fn compact(&mut self) {
        // 1. Collect all read receipt message IDs
        let read_ids: HashSet<String> = self
            .lines
            .iter()
            .filter_map(|line| match line {
                Line::ReadReceipt { message_id, .. } => Some(message_id.clone()),
                _ => None,
            })
            .collect();

        // 2. Filter to messages, apply read status, dedupe by ID
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut messages: Vec<Message> = self
            .lines
            .iter()
            .filter_map(|line| match line {
                Line::Message(msg) => {
                    // Skip duplicates (first occurrence wins)
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

        // 4. Replace lines with compacted messages
        self.lines = messages.into_iter().map(Line::Message).collect();
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-protocol conversation`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-protocol/src/conversation/mod.rs
git commit -m "feat(river-protocol): add message ID dedupe to compaction"
```

---

## Task 4: Remove Unused since_id Field

**Files:**
- Modify: `crates/river-worker/src/state.rs`
- Modify: `crates/river-worker/src/http.rs`

- [ ] **Step 1: Remove since_id from Notification struct**

In `/home/cassie/river-engine/crates/river-worker/src/state.rs`, update `Notification`:

```rust
/// Notification about new messages.
#[derive(Debug, Clone)]
pub struct Notification {
    pub channel: Channel,
    pub count: usize,
}
```

- [ ] **Step 2: Remove since_id from notification creation in http.rs**

In `/home/cassie/river-engine/crates/river-worker/src/http.rs`, find where `Notification` is created (around line 122) and update:

```rust
            s.pending_notifications.push(crate::state::Notification {
                channel,
                count: 1,
            });
```

- [ ] **Step 3: Verify build**

Run: `cargo build -p river-worker`
Expected: Success (one less warning)

- [ ] **Step 4: Commit**

```bash
git add crates/river-worker/src/state.rs crates/river-worker/src/http.rs
git commit -m "refactor(river-worker): remove unused since_id from Notification"
```

---

## Task 5: Add read_history Tool to Worker

**Files:**
- Modify: `crates/river-worker/src/tools.rs`

- [ ] **Step 1: Add ReadHistoryResult struct**

In `/home/cassie/river-engine/crates/river-worker/src/tools.rs`, add near the top with other structs:

```rust
/// Result from read_history tool.
#[derive(Debug, Serialize, Default)]
pub struct ReadHistoryResult {
    pub success: bool,
    pub messages_fetched: usize,
    pub oldest_id: Option<String>,
    pub newest_id: Option<String>,
    pub error: Option<String>,
    pub retry_after_ms: Option<u64>,
}
```

- [ ] **Step 2: Add execute_read_history function**

Add the implementation after `execute_embed_search`:

```rust
async fn execute_read_history(
    args: &serde_json::Value,
    state: &SharedState,
) -> ToolResult {
    // Extract required params
    let channel = match args.get("channel").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "channel".into(),
            });
        }
    };

    let adapter = match args.get("adapter").and_then(|v| v.as_str()) {
        Some(a) => a.to_string(),
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "adapter".into(),
            });
        }
    };

    // Optional params
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|l| l.min(100) as u32);
    let before = args.get("before").and_then(|v| v.as_str()).map(String::from);
    let after = args.get("after").and_then(|v| v.as_str()).map(String::from);

    // Check mutual exclusivity
    if before.is_some() && after.is_some() {
        return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
            success: false,
            error: Some("Cannot specify both 'before' and 'after'".into()),
            ..Default::default()
        }).unwrap());
    }

    let s = state.read().await;

    // Find adapter in registry
    let adapter_entry = s.registry.processes.iter().find(|p| {
        if let river_protocol::ProcessEntry::Adapter { adapter_type, .. } = p {
            adapter_type == &adapter
        } else {
            false
        }
    });

    let adapter_endpoint = match adapter_entry {
        Some(river_protocol::ProcessEntry::Adapter { endpoint, features, .. }) => {
            // Check ReadHistory feature
            if !features.contains(&(river_adapter::FeatureId::ReadHistory as u16)) {
                return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                    success: false,
                    error: Some("Adapter does not support ReadHistory".into()),
                    ..Default::default()
                }).unwrap());
            }
            endpoint.clone()
        }
        _ => {
            return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                success: false,
                error: Some(format!("Adapter '{}' not found", adapter)),
                ..Default::default()
            }).unwrap());
        }
    };

    let workspace = s.workspace.clone();
    drop(s);

    // Build request
    let request = river_adapter::OutboundRequest::ReadHistory {
        channel: channel.clone(),
        limit,
        before,
        after,
    };

    // Call adapter
    let client = reqwest::Client::new();
    let response = match client
        .post(format!("{}/execute", adapter_endpoint))
        .json(&request)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                success: false,
                error: Some(format!("Request failed: {}", e)),
                ..Default::default()
            }).unwrap());
        }
    };

    // Parse response
    let resp: river_adapter::OutboundResponse = match response.json().await {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                success: false,
                error: Some(format!("Invalid response: {}", e)),
                ..Default::default()
            }).unwrap());
        }
    };

    if !resp.ok {
        let err = resp.error.unwrap_or_else(|| river_adapter::ResponseError::new(
            river_adapter::ErrorCode::PlatformError,
            "Unknown error",
        ));
        return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
            success: false,
            error: Some(err.message),
            retry_after_ms: err.retry_after_ms,
            ..Default::default()
        }).unwrap());
    }

    // Extract messages
    let messages = match resp.data {
        Some(river_adapter::ResponseData::History { messages }) => messages,
        _ => {
            return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                success: false,
                error: Some("Unexpected response format".into()),
                ..Default::default()
            }).unwrap());
        }
    };

    // Write to conversation file
    let conv_channel = river_protocol::Channel {
        adapter: adapter.clone(),
        id: channel.clone(),
        name: None,
    };
    let path = crate::conversation::conversation_path_for_channel(&workspace, &conv_channel);

    let mut oldest_id: Option<String> = None;
    let mut newest_id: Option<String> = None;

    for msg in &messages {
        // Track oldest/newest
        if oldest_id.is_none() || msg.message_id < *oldest_id.as_ref().unwrap() {
            oldest_id = Some(msg.message_id.clone());
        }
        if newest_id.is_none() || msg.message_id > *newest_id.as_ref().unwrap() {
            newest_id = Some(msg.message_id.clone());
        }

        let line = river_protocol::conversation::Message {
            direction: river_protocol::conversation::MessageDirection::Unread,
            timestamp: msg.timestamp.clone(),
            id: msg.message_id.clone(),
            author: river_protocol::Author {
                name: msg.author.name.clone(),
                id: msg.author.id.clone(),
                bot: msg.author.bot,
            },
            content: msg.content.clone(),
            reactions: vec![],
        };

        if let Err(e) = river_protocol::conversation::Conversation::append_line(
            &path,
            &river_protocol::conversation::Line::Message(line),
        ) {
            tracing::warn!(error = %e, "Failed to write history message to conversation file");
        }
    }

    ToolResult::Success(serde_json::to_value(ReadHistoryResult {
        success: true,
        messages_fetched: messages.len(),
        oldest_id,
        newest_id,
        error: None,
        retry_after_ms: None,
    }).unwrap())
}
```

- [ ] **Step 3: Add tool to dispatch**

In the `execute_tool` function, add the new case:

```rust
        "read_history" => execute_read_history(args, state).await,
```

- [ ] **Step 4: Verify build**

Run: `cargo build -p river-worker`
Expected: Success

- [ ] **Step 5: Commit**

```bash
git add crates/river-worker/src/tools.rs
git commit -m "feat(river-worker): add read_history tool"
```

---

## Task 6: Implement ReadHistory in Discord Adapter

**Files:**
- Modify: `crates/river-discord/src/execute.rs` (or equivalent)

- [ ] **Step 1: Find execute handler**

First, locate where `OutboundRequest` is handled:

```bash
grep -r "OutboundRequest::" crates/river-discord/src/
```

- [ ] **Step 2: Add ReadHistory handler**

In the execute match block, add:

```rust
OutboundRequest::ReadHistory { channel, limit, before, after } => {
    let channel_id = channel.parse::<Id<ChannelMarker>>()
        .map_err(|_| ResponseError::new(ErrorCode::InvalidPayload, "Invalid channel ID"))?;

    let limit = limit.unwrap_or(50).min(100) as u16;

    let mut request = client.channel_messages(channel_id).limit(limit);

    if let Some(before_id) = before {
        let id = before_id.parse::<Id<MessageMarker>>()
            .map_err(|_| ResponseError::new(ErrorCode::InvalidPayload, "Invalid before ID"))?;
        request = request.before(id);
    }
    if let Some(after_id) = after {
        let id = after_id.parse::<Id<MessageMarker>>()
            .map_err(|_| ResponseError::new(ErrorCode::InvalidPayload, "Invalid after ID"))?;
        request = request.after(id);
    }

    let response = request.await;

    match response {
        Ok(messages) => {
            let history: Vec<HistoryMessage> = messages.models().await
                .map_err(|e| ResponseError::new(ErrorCode::PlatformError, e.to_string()))?
                .into_iter()
                .map(|m| HistoryMessage {
                    message_id: m.id.to_string(),
                    channel: channel.clone(),
                    author: Author {
                        id: m.author.id.to_string(),
                        name: m.author.name.clone(),
                        bot: m.author.bot,
                    },
                    content: m.content.clone(),
                    timestamp: m.timestamp.to_string(),
                    reply_to: m.reference.as_ref()
                        .and_then(|r| r.message_id)
                        .map(|id| id.to_string()),
                })
                .collect();

            Ok(OutboundResponse::success(ResponseData::History { messages: history }))
        }
        Err(e) => {
            // Check for rate limit
            if let Some(status) = e.status() {
                if status == 429 {
                    // Try to parse retry_after from error
                    return Err(ResponseError::rate_limited(
                        "Rate limited by Discord",
                        5000, // Default 5s if we can't parse
                    ));
                }
            }
            Err(ResponseError::new(ErrorCode::PlatformError, e.to_string()))
        }
    }
}
```

- [ ] **Step 3: Add necessary imports**

Ensure these imports are present:

```rust
use river_adapter::{HistoryMessage, ResponseData, ResponseError, ErrorCode};
```

- [ ] **Step 4: Verify build**

Run: `cargo build -p river-discord`
Expected: Success

- [ ] **Step 5: Commit**

```bash
git add crates/river-discord/
git commit -m "feat(river-discord): implement ReadHistory handler"
```

---

## Task 7: Final Verification

**Files:** None (verification only)

- [ ] **Step 1: Run all tests**

```bash
cargo test --workspace
```

Expected: All tests pass

- [ ] **Step 2: Build entire workspace**

```bash
cargo build --workspace
```

Expected: Success

- [ ] **Step 3: Check warnings**

```bash
cargo build --workspace 2>&1 | grep "warning:" | grep -v "generated" | wc -l
```

Expected: Fewer warnings than before (since_id warning gone)

- [ ] **Step 4: Commit any fixes**

If any issues found, fix and commit.

---

## Summary

| Task | Description | Key Files |
|------|-------------|-----------|
| 1 | Add `after` param to ReadHistory | river-adapter/feature.rs |
| 2 | Extend HistoryMessage, add rate limit | river-adapter/response.rs |
| 3 | Add dedupe to compaction | river-protocol/conversation/mod.rs |
| 4 | Remove unused since_id | river-worker/state.rs, http.rs |
| 5 | Add read_history tool | river-worker/tools.rs |
| 6 | Implement Discord handler | river-discord/execute.rs |
| 7 | Final verification | - |

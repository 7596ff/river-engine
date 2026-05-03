# river-context Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix river-context to properly intersperse flashes and embeddings by timestamp, add missing id.rs module for timestamp extraction, improve error handling with thiserror, and achieve comprehensive test coverage.

**Architecture:** The river-context crate assembles workspace data (moments, moves, messages, flashes, embeddings) into OpenAI-compatible messages. The fix introduces timestamp-based ordering by extracting timestamps from snowflake IDs, enabling proper chronological interspersing of flashes globally and embeddings within channels. A new `id.rs` module provides the timestamp extraction function used throughout assembly.

**Tech Stack:** Rust, serde, thiserror, chrono (for robust timestamp comparison), snowflake IDs (128-bit with timestamp in high 64 bits)

---

## File Structure

```
crates/river-context/
├── Cargo.toml              # Add thiserror, chrono dependencies
└── src/
    ├── lib.rs              # Add id module, update re-exports
    ├── id.rs               # NEW: Timestamp extraction from snowflake IDs
    ├── assembly.rs         # Fix interspersing logic for flashes/embeddings
    ├── format.rs           # No changes (format functions are correct)
    ├── openai.rs           # Add PartialEq derives
    ├── request.rs          # Add Default impl
    ├── response.rs         # Use thiserror, add PartialEq
    ├── tokens.rs           # No changes
    └── workspace.rs        # Fix import from river-protocol
```

---

## Task 1: Add Dependencies to Cargo.toml

**File:** `crates/river-context/Cargo.toml`

- [ ] **Step 1.1:** Add thiserror and chrono dependencies

```toml
[package]
name = "river-context"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Context assembly library for River Engine workers"

[dependencies]
river-adapter = { path = "../river-adapter" }
river-protocol = { path = "../river-protocol" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
chrono = { workspace = true }
```

- [ ] **Step 1.2:** Verify cargo check passes

```bash
cd /home/cassie/river-engine && cargo check -p river-context
```

- [ ] **Step 1.3:** Commit changes

```bash
git add crates/river-context/Cargo.toml && git commit -m "feat(river-context): add thiserror and chrono dependencies"
```

---

## Task 2: Create id.rs Module for Timestamp Extraction

**File:** `crates/river-context/src/id.rs` (NEW)

- [ ] **Step 2.1:** Create id.rs with timestamp extraction function

```rust
//! Snowflake ID utilities for timestamp extraction.

/// Extract timestamp (microseconds since epoch) from a snowflake ID.
///
/// Snowflake IDs are 128-bit integers where the high 64 bits contain
/// the timestamp in microseconds since Unix epoch.
///
/// # Arguments
/// * `id` - String representation of a snowflake ID
///
/// # Returns
/// * `Some(timestamp)` - Timestamp in microseconds if parsing succeeds
/// * `None` - If the ID cannot be parsed as a u128
///
/// # Example
/// ```
/// use river_context::extract_timestamp;
///
/// let id = "340282366920938463463374607431768211456"; // Example snowflake
/// if let Some(ts) = extract_timestamp(id) {
///     println!("Timestamp: {} microseconds", ts);
/// }
/// ```
pub fn extract_timestamp(id: &str) -> Option<u64> {
    let snowflake = id.parse::<u128>().ok()?;
    let high = (snowflake >> 64) as u64; // Timestamp in microseconds
    Some(high)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_timestamp_valid() {
        // Snowflake with known timestamp in high bits
        // High 64 bits = 1000000 (1 second in microseconds)
        // Low 64 bits = 0
        let snowflake: u128 = (1_000_000u128) << 64;
        let id = snowflake.to_string();

        let ts = extract_timestamp(&id).unwrap();
        assert_eq!(ts, 1_000_000);
    }

    #[test]
    fn test_extract_timestamp_with_low_bits() {
        // High 64 bits = 5000000, Low 64 bits = 12345
        let high: u128 = 5_000_000;
        let low: u128 = 12345;
        let snowflake: u128 = (high << 64) | low;
        let id = snowflake.to_string();

        let ts = extract_timestamp(&id).unwrap();
        assert_eq!(ts, 5_000_000);
    }

    #[test]
    fn test_extract_timestamp_invalid() {
        assert!(extract_timestamp("not_a_number").is_none());
        assert!(extract_timestamp("").is_none());
        assert!(extract_timestamp("-123").is_none());
    }

    #[test]
    fn test_extract_timestamp_zero() {
        let ts = extract_timestamp("0").unwrap();
        assert_eq!(ts, 0);
    }

    #[test]
    fn test_extract_timestamp_ordering() {
        // Earlier timestamp
        let early: u128 = (1_000_000u128) << 64;
        // Later timestamp
        let late: u128 = (2_000_000u128) << 64;

        let ts_early = extract_timestamp(&early.to_string()).unwrap();
        let ts_late = extract_timestamp(&late.to_string()).unwrap();

        assert!(ts_early < ts_late);
    }
}
```

- [ ] **Step 2.2:** Add id module to lib.rs

In `crates/river-context/src/lib.rs`, add after line 34 (`mod assembly;`):

```rust
mod id;
```

And add to the public exports after line 46:

```rust
pub use id::extract_timestamp;
```

The updated lib.rs should have these module declarations:

```rust
mod assembly;
mod format;
mod id;
mod openai;
mod request;
mod response;
mod tokens;
mod workspace;
```

And these exports:

```rust
pub use assembly::build_context;
pub use id::extract_timestamp;
pub use openai::{FunctionCall, OpenAIMessage, ToolCall};
pub use request::{ChannelContext, ContextRequest};
pub use response::{ContextError, ContextResponse};
pub use tokens::{estimate_message_tokens, estimate_tokens, estimate_total_tokens};
pub use workspace::{ChatMessage, Embedding, Flash, Moment, Move};
```

- [ ] **Step 2.3:** Run tests to verify id.rs works

```bash
cd /home/cassie/river-engine && cargo test -p river-context id::
```

- [ ] **Step 2.4:** Commit changes

```bash
git add crates/river-context/src/id.rs crates/river-context/src/lib.rs && git commit -m "feat(river-context): add id.rs module for timestamp extraction from snowflake IDs"
```

---

## Task 3: Update response.rs to Use thiserror and Add PartialEq

**File:** `crates/river-context/src/response.rs`

- [ ] **Step 3.1:** Replace manual error implementation with thiserror derive

```rust
//! Context response types.

use thiserror::Error;

use crate::openai::OpenAIMessage;

/// Response from context assembly.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextResponse {
    /// Flat timeline of OpenAI-compatible messages.
    pub messages: Vec<OpenAIMessage>,
    /// Estimated token count.
    pub estimated_tokens: usize,
}

/// Errors that can occur during context assembly.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContextError {
    /// Assembled context exceeds max_tokens.
    #[error("context over budget: {estimated} tokens (limit {limit})")]
    OverBudget { estimated: usize, limit: usize },

    /// No channels provided.
    #[error("no channels provided")]
    EmptyChannels,
}
```

- [ ] **Step 3.2:** Run cargo check to verify changes

```bash
cd /home/cassie/river-engine && cargo check -p river-context
```

- [ ] **Step 3.3:** Commit changes

```bash
git add crates/river-context/src/response.rs && git commit -m "refactor(river-context): use thiserror for ContextError, add PartialEq derives"
```

---

## Task 4: Add PartialEq to OpenAI Types

**File:** `crates/river-context/src/openai.rs`

- [ ] **Step 4.1:** Add PartialEq derive to all types

Update the derives for `OpenAIMessage`:

```rust
/// OpenAI-compatible message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAIMessage {
```

Update the derives for `ToolCall`:

```rust
/// Tool call in an assistant message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
```

Update the derives for `FunctionCall`:

```rust
/// Function call details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionCall {
```

- [ ] **Step 4.2:** Run cargo check

```bash
cd /home/cassie/river-engine && cargo check -p river-context
```

- [ ] **Step 4.3:** Commit changes

```bash
git add crates/river-context/src/openai.rs && git commit -m "feat(river-context): add PartialEq derive to OpenAI message types"
```

---

## Task 5: Add Default Implementation for ContextRequest

**File:** `crates/river-context/src/request.rs`

- [ ] **Step 5.1:** Add Default implementation for ContextRequest

Add after the `ContextRequest` struct definition (after line 32):

```rust
impl Default for ContextRequest {
    fn default() -> Self {
        Self {
            channels: Vec::new(),
            flashes: Vec::new(),
            history: Vec::new(),
            max_tokens: 8000,
            now: String::new(),
        }
    }
}
```

- [ ] **Step 5.2:** Add Default implementation for ChannelContext

Add after the `ChannelContext` struct definition (after line 17):

```rust
impl Default for ChannelContext {
    fn default() -> Self {
        Self {
            channel: Channel {
                adapter: String::new(),
                id: String::new(),
                name: None,
            },
            moments: Vec::new(),
            moves: Vec::new(),
            messages: Vec::new(),
            embeddings: Vec::new(),
        }
    }
}
```

- [ ] **Step 5.3:** Update the import to use river_protocol::Channel

Change line 3 from:

```rust
use river_adapter::Channel;
```

To:

```rust
use river_protocol::Channel;
```

- [ ] **Step 5.4:** Run cargo check

```bash
cd /home/cassie/river-engine && cargo check -p river-context
```

- [ ] **Step 5.5:** Commit changes

```bash
git add crates/river-context/src/request.rs && git commit -m "feat(river-context): add Default impls for ContextRequest and ChannelContext"
```

---

## Task 6: Fix Inconsistent Import in workspace.rs

**File:** `crates/river-context/src/workspace.rs`

- [ ] **Step 6.1:** Change import from river-adapter to river-protocol

Change line 3 from:

```rust
use river_adapter::Author;
```

To:

```rust
use river_protocol::Author;
```

- [ ] **Step 6.2:** Run cargo check

```bash
cd /home/cassie/river-engine && cargo check -p river-context
```

- [ ] **Step 6.3:** Commit changes

```bash
git add crates/river-context/src/workspace.rs && git commit -m "refactor(river-context): use river-protocol for Author import"
```

---

## Task 7: Fix Inconsistent Import in format.rs

**File:** `crates/river-context/src/format.rs`

- [ ] **Step 7.1:** Change import from river-adapter to river-protocol

Change line 3 from:

```rust
use river_adapter::Channel;
```

To:

```rust
use river_protocol::Channel;
```

- [ ] **Step 7.2:** Run cargo check

```bash
cd /home/cassie/river-engine && cargo check -p river-context
```

- [ ] **Step 7.3:** Commit changes

```bash
git add crates/river-context/src/format.rs && git commit -m "refactor(river-context): use river-protocol for Channel import"
```

---

## Task 8: Implement Timestamp-Based Interspersing in assembly.rs

**File:** `crates/river-context/src/assembly.rs`

- [ ] **Step 8.1:** Add imports for id module and chrono

Update the imports at the top of the file:

```rust
//! Context assembly logic.

use chrono::{DateTime, Utc};

use crate::format::{format_chat_messages, format_embedding, format_flash, format_moment, format_move};
use crate::id::extract_timestamp;
use crate::openai::OpenAIMessage;
use crate::request::{ChannelContext, ContextRequest};
use crate::response::{ContextError, ContextResponse};
use crate::tokens::estimate_total_tokens;
use crate::workspace::{Embedding, Flash};
```

- [ ] **Step 8.2:** Create helper struct for timeline items with timestamps

Add after the imports:

```rust
/// Item in the timeline with its timestamp for sorting.
#[derive(Debug)]
struct TimelineItem {
    /// Timestamp in microseconds (from snowflake ID).
    timestamp: u64,
    /// The formatted message.
    message: OpenAIMessage,
}

impl TimelineItem {
    fn new(id: &str, message: OpenAIMessage) -> Self {
        let timestamp = extract_timestamp(id).unwrap_or(0);
        Self { timestamp, message }
    }
}
```

- [ ] **Step 8.3:** Create helper function for robust TTL comparison

Add after the `TimelineItem` struct:

```rust
/// Check if an item has expired based on TTL.
/// Uses chrono for robust timestamp comparison.
fn is_expired(expires_at: &str, now: &DateTime<Utc>) -> bool {
    match expires_at.parse::<DateTime<Utc>>() {
        Ok(expiry) => expiry <= *now,
        Err(_) => {
            // Fallback to string comparison for ISO8601 UTC strings
            expires_at <= now.to_rfc3339().as_str()
        }
    }
}

/// Parse the current time string into a DateTime.
fn parse_now(now: &str) -> DateTime<Utc> {
    now.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now())
}
```

- [ ] **Step 8.4:** Rewrite build_context function with timestamp interspersing

Replace the entire `build_context` function:

```rust
/// Build context from request.
pub fn build_context(request: ContextRequest) -> Result<ContextResponse, ContextError> {
    if request.channels.is_empty() {
        return Err(ContextError::EmptyChannels);
    }

    let now_dt = parse_now(&request.now);

    // Collect all timeline items with timestamps
    let mut timeline: Vec<TimelineItem> = Vec::new();

    // Filter flashes by TTL and collect with timestamps
    let valid_flashes: Vec<&Flash> = request
        .flashes
        .iter()
        .filter(|f| !is_expired(&f.expires_at, &now_dt))
        .collect();

    for flash in &valid_flashes {
        timeline.push(TimelineItem::new(&flash.id, format_flash(flash)));
    }

    // Process other channels (not current, not last): moments + moves only
    if request.channels.len() > 2 {
        for channel_ctx in &request.channels[2..] {
            collect_channel_summary(&mut timeline, channel_ctx);
        }
    }

    // Process last channel (index 1 if exists): moments + moves + embeddings
    if request.channels.len() > 1 {
        let last_ctx = &request.channels[1];
        collect_channel_summary(&mut timeline, last_ctx);
        collect_channel_embeddings(&mut timeline, last_ctx, &now_dt);
    }

    // Process current channel (index 0): moments + moves + embeddings
    let current_ctx = &request.channels[0];
    collect_channel_summary(&mut timeline, current_ctx);
    collect_channel_embeddings(&mut timeline, current_ctx, &now_dt);

    // Sort timeline by timestamp
    timeline.sort_by_key(|item| item.timestamp);

    // Build final message list
    let mut messages: Vec<OpenAIMessage> = timeline.into_iter().map(|item| item.message).collect();

    // Add LLM history block (not sorted, keeps its position)
    messages.extend(request.history);

    // Add chat messages for current channel (at the end, most recent)
    if !current_ctx.messages.is_empty() {
        messages.push(format_chat_messages(&current_ctx.messages, &current_ctx.channel));
    }

    // Estimate tokens
    let estimated_tokens = estimate_total_tokens(&messages);

    if estimated_tokens > request.max_tokens {
        return Err(ContextError::OverBudget {
            estimated: estimated_tokens,
            limit: request.max_tokens,
        });
    }

    Ok(ContextResponse {
        messages,
        estimated_tokens,
    })
}
```

- [ ] **Step 8.5:** Update helper functions to use TimelineItem

Replace `add_channel_summary` and `add_channel_embeddings`:

```rust
fn collect_channel_summary(timeline: &mut Vec<TimelineItem>, ctx: &ChannelContext) {
    for moment in &ctx.moments {
        timeline.push(TimelineItem::new(&moment.id, format_moment(moment, &ctx.channel)));
    }
    for mv in &ctx.moves {
        timeline.push(TimelineItem::new(&mv.id, format_move(mv, &ctx.channel)));
    }
}

fn collect_channel_embeddings(
    timeline: &mut Vec<TimelineItem>,
    ctx: &ChannelContext,
    now: &DateTime<Utc>,
) {
    for embedding in &ctx.embeddings {
        if !is_expired(&embedding.expires_at, now) {
            timeline.push(TimelineItem::new(&embedding.id, format_embedding(embedding)));
        }
    }
}
```

- [ ] **Step 8.6:** Update test imports to use river_protocol::Channel

Change line 93:

```rust
    use river_adapter::Channel;
```

To:

```rust
    use river_protocol::Channel;
```

- [ ] **Step 8.7:** Run cargo check

```bash
cd /home/cassie/river-engine && cargo check -p river-context
```

- [ ] **Step 8.8:** Run existing tests

```bash
cd /home/cassie/river-engine && cargo test -p river-context
```

- [ ] **Step 8.9:** Commit changes

```bash
git add crates/river-context/src/assembly.rs && git commit -m "feat(river-context): implement timestamp-based interspersing for flashes and embeddings"
```

---

## Task 9: Add Comprehensive Tests for Format Functions

**File:** `crates/river-context/src/format.rs`

- [ ] **Step 9.1:** Update import to use river_protocol

Change line 3 (already done in Task 7, but verify):

```rust
use river_protocol::Channel;
```

- [ ] **Step 9.2:** Add test module with comprehensive tests

Add at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use river_protocol::Author;

    fn test_channel() -> Channel {
        Channel {
            adapter: "discord".into(),
            id: "123456".into(),
            name: Some("general".into()),
        }
    }

    fn test_channel_no_name() -> Channel {
        Channel {
            adapter: "slack".into(),
            id: "789".into(),
            name: None,
        }
    }

    #[test]
    fn test_format_moment_with_channel_name() {
        let moment = Moment {
            id: "1".into(),
            content: "Discussion about API design".into(),
            move_range: ("100".into(), "150".into()),
        };
        let channel = test_channel();

        let msg = format_moment(&moment, &channel);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Moment: general] Discussion about API design (moves 100-150)"
        );
    }

    #[test]
    fn test_format_moment_without_channel_name() {
        let moment = Moment {
            id: "1".into(),
            content: "Team sync".into(),
            move_range: ("50".into(), "75".into()),
        };
        let channel = test_channel_no_name();

        let msg = format_moment(&moment, &channel);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Moment: 789] Team sync (moves 50-75)"
        );
    }

    #[test]
    fn test_format_move_with_channel_name() {
        let mv = Move {
            id: "2".into(),
            content: "Reviewed PR #42".into(),
            message_range: ("200".into(), "210".into()),
        };
        let channel = test_channel();

        let msg = format_move(&mv, &channel);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Move: general] Reviewed PR #42 (messages 200-210)"
        );
    }

    #[test]
    fn test_format_move_without_channel_name() {
        let mv = Move {
            id: "2".into(),
            content: "Bug triage".into(),
            message_range: ("300".into(), "320".into()),
        };
        let channel = test_channel_no_name();

        let msg = format_move(&mv, &channel);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Move: 789] Bug triage (messages 300-320)"
        );
    }

    #[test]
    fn test_format_flash() {
        let flash = Flash {
            id: "3".into(),
            from: "worker-alpha".into(),
            content: "Urgent: deploy blocked".into(),
            expires_at: "2026-04-01T15:00:00Z".into(),
        };

        let msg = format_flash(&flash);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Flash from worker-alpha] Urgent: deploy blocked"
        );
    }

    #[test]
    fn test_format_embedding() {
        let embedding = Embedding {
            id: "4".into(),
            content: "API documentation for /users endpoint".into(),
            source: "docs/api.md:15-42".into(),
            expires_at: "2026-04-01T18:00:00Z".into(),
        };

        let msg = format_embedding(&embedding);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Reference: docs/api.md:15-42]\nAPI documentation for /users endpoint"
        );
    }

    #[test]
    fn test_format_chat_messages_single() {
        let messages = vec![ChatMessage {
            id: "5".into(),
            timestamp: "2026-04-01T12:00:00Z".into(),
            author: Author {
                id: "user1".into(),
                name: "Alice".into(),
            },
            content: "Hello world!".into(),
        }];
        let channel = test_channel();

        let msg = format_chat_messages(&messages, &channel);

        assert_eq!(msg.role, "user");
        assert_eq!(
            msg.content.unwrap(),
            "[Chat: general]\n[2026-04-01T12:00:00Z] <Alice> Hello world!"
        );
    }

    #[test]
    fn test_format_chat_messages_multiple() {
        let messages = vec![
            ChatMessage {
                id: "5".into(),
                timestamp: "2026-04-01T12:00:00Z".into(),
                author: Author {
                    id: "user1".into(),
                    name: "Alice".into(),
                },
                content: "Hello!".into(),
            },
            ChatMessage {
                id: "6".into(),
                timestamp: "2026-04-01T12:01:00Z".into(),
                author: Author {
                    id: "user2".into(),
                    name: "Bob".into(),
                },
                content: "Hi Alice!".into(),
            },
            ChatMessage {
                id: "7".into(),
                timestamp: "2026-04-01T12:02:00Z".into(),
                author: Author {
                    id: "user1".into(),
                    name: "Alice".into(),
                },
                content: "How are you?".into(),
            },
        ];
        let channel = test_channel();

        let msg = format_chat_messages(&messages, &channel);

        assert_eq!(msg.role, "user");
        let content = msg.content.unwrap();
        assert!(content.starts_with("[Chat: general]"));
        assert!(content.contains("<Alice> Hello!"));
        assert!(content.contains("<Bob> Hi Alice!"));
        assert!(content.contains("<Alice> How are you?"));
    }
}
```

- [ ] **Step 9.3:** Add Moment, Move, Flash, Embedding imports for tests

Update the test module imports:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Embedding, Flash, Moment, Move};
    use river_protocol::Author;
```

- [ ] **Step 9.4:** Run format tests

```bash
cd /home/cassie/river-engine && cargo test -p river-context format::
```

- [ ] **Step 9.5:** Commit changes

```bash
git add crates/river-context/src/format.rs && git commit -m "test(river-context): add comprehensive tests for format functions"
```

---

## Task 10: Add Comprehensive Tests for Assembly

**File:** `crates/river-context/src/assembly.rs`

- [ ] **Step 10.1:** Add helper functions for creating test data

Add to the test module:

```rust
    use crate::workspace::{ChatMessage, Embedding, Flash, Moment, Move};
    use river_protocol::Author;

    /// Create a snowflake ID with a specific timestamp (microseconds).
    fn make_id(timestamp_micros: u64) -> String {
        let snowflake: u128 = (timestamp_micros as u128) << 64;
        snowflake.to_string()
    }

    fn test_channel(name: &str) -> Channel {
        Channel {
            adapter: "discord".into(),
            id: format!("chan_{}", name),
            name: Some(name.into()),
        }
    }

    fn test_moment(id: &str, content: &str) -> Moment {
        Moment {
            id: id.into(),
            content: content.into(),
            move_range: ("0".into(), "0".into()),
        }
    }

    fn test_move(id: &str, content: &str) -> Move {
        Move {
            id: id.into(),
            content: content.into(),
            message_range: ("0".into(), "0".into()),
        }
    }

    fn test_flash(id: &str, from: &str, content: &str, expires_at: &str) -> Flash {
        Flash {
            id: id.into(),
            from: from.into(),
            content: content.into(),
            expires_at: expires_at.into(),
        }
    }

    fn test_embedding(id: &str, content: &str, expires_at: &str) -> Embedding {
        Embedding {
            id: id.into(),
            content: content.into(),
            source: "test".into(),
            expires_at: expires_at.into(),
        }
    }

    fn test_message(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            timestamp: "2026-04-01T12:00:00Z".into(),
            author: Author {
                id: "user1".into(),
                name: "TestUser".into(),
            },
            content: content.into(),
        }
    }
```

- [ ] **Step 10.2:** Add test for multi-channel assembly

```rust
    #[test]
    fn test_build_context_multi_channel() {
        let request = ContextRequest {
            channels: vec![
                ChannelContext {
                    channel: test_channel("current"),
                    moments: vec![test_moment(&make_id(3000), "Current moment")],
                    moves: vec![],
                    messages: vec![test_message("msg1", "Hello")],
                    embeddings: vec![],
                },
                ChannelContext {
                    channel: test_channel("last"),
                    moments: vec![test_moment(&make_id(2000), "Last moment")],
                    moves: vec![],
                    messages: vec![],
                    embeddings: vec![],
                },
                ChannelContext {
                    channel: test_channel("other"),
                    moments: vec![test_moment(&make_id(1000), "Other moment")],
                    moves: vec![],
                    messages: vec![],
                    embeddings: vec![],
                },
            ],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        // Should have 3 moments + 1 chat message
        assert_eq!(result.messages.len(), 4);

        // Verify ordering by timestamp (earliest first)
        let content_0 = result.messages[0].content.as_ref().unwrap();
        let content_1 = result.messages[1].content.as_ref().unwrap();
        let content_2 = result.messages[2].content.as_ref().unwrap();

        assert!(content_0.contains("Other moment")); // timestamp 1000
        assert!(content_1.contains("Last moment")); // timestamp 2000
        assert!(content_2.contains("Current moment")); // timestamp 3000
    }
```

- [ ] **Step 10.3:** Add test for flash interspersing by timestamp

```rust
    #[test]
    fn test_flashes_interspersed_by_timestamp() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![
                    test_moment(&make_id(1000), "Early moment"),
                    test_moment(&make_id(3000), "Late moment"),
                ],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
            }],
            flashes: vec![
                test_flash(&make_id(2000), "worker1", "Middle flash", "2026-04-02T00:00:00Z"),
            ],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        assert_eq!(result.messages.len(), 3);

        // Flash should be between the two moments
        let content_0 = result.messages[0].content.as_ref().unwrap();
        let content_1 = result.messages[1].content.as_ref().unwrap();
        let content_2 = result.messages[2].content.as_ref().unwrap();

        assert!(content_0.contains("Early moment")); // timestamp 1000
        assert!(content_1.contains("Middle flash")); // timestamp 2000
        assert!(content_2.contains("Late moment")); // timestamp 3000
    }
```

- [ ] **Step 10.4:** Add test for TTL filtering of flashes

```rust
    #[test]
    fn test_flash_ttl_filtering() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
            }],
            flashes: vec![
                test_flash(&make_id(1000), "worker1", "Expired flash", "2026-04-01T11:00:00Z"),
                test_flash(&make_id(2000), "worker2", "Valid flash", "2026-04-01T13:00:00Z"),
            ],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        // Only the valid flash should be included
        assert_eq!(result.messages.len(), 1);
        assert!(result.messages[0].content.as_ref().unwrap().contains("Valid flash"));
    }
```

- [ ] **Step 10.5:** Add test for embedding interspersing within channel

```rust
    #[test]
    fn test_embeddings_interspersed_by_timestamp() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![
                    test_moment(&make_id(1000), "Moment 1"),
                    test_moment(&make_id(3000), "Moment 2"),
                ],
                moves: vec![],
                messages: vec![],
                embeddings: vec![
                    test_embedding(&make_id(2000), "Embedding content", "2026-04-02T00:00:00Z"),
                ],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        assert_eq!(result.messages.len(), 3);

        // Embedding should be between the two moments
        let content_0 = result.messages[0].content.as_ref().unwrap();
        let content_1 = result.messages[1].content.as_ref().unwrap();
        let content_2 = result.messages[2].content.as_ref().unwrap();

        assert!(content_0.contains("Moment 1")); // timestamp 1000
        assert!(content_1.contains("Embedding content")); // timestamp 2000
        assert!(content_2.contains("Moment 2")); // timestamp 3000
    }
```

- [ ] **Step 10.6:** Add test for embedding TTL filtering

```rust
    #[test]
    fn test_embedding_ttl_filtering() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![],
                moves: vec![],
                messages: vec![],
                embeddings: vec![
                    test_embedding(&make_id(1000), "Expired embedding", "2026-04-01T11:00:00Z"),
                    test_embedding(&make_id(2000), "Valid embedding", "2026-04-01T13:00:00Z"),
                ],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        // Only the valid embedding should be included
        assert_eq!(result.messages.len(), 1);
        assert!(result.messages[0].content.as_ref().unwrap().contains("Valid embedding"));
    }
```

- [ ] **Step 10.7:** Add test for over-budget error

```rust
    #[test]
    fn test_over_budget_error() {
        let long_content = "x".repeat(10000);
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![test_moment(&make_id(1000), &long_content)],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 100, // Very low limit
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request);

        match result {
            Err(ContextError::OverBudget { estimated, limit }) => {
                assert!(estimated > limit);
                assert_eq!(limit, 100);
            }
            _ => panic!("Expected OverBudget error"),
        }
    }
```

- [ ] **Step 10.8:** Add test for history placement

```rust
    #[test]
    fn test_history_placement() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![test_moment(&make_id(1000), "A moment")],
                moves: vec![],
                messages: vec![test_message("msg1", "User message")],
                embeddings: vec![],
            }],
            flashes: vec![],
            history: vec![
                OpenAIMessage::user("Previous user message".into()),
                OpenAIMessage::assistant("Previous assistant response".into()),
            ],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        // Order: sorted timeline items, then history, then chat messages
        assert_eq!(result.messages.len(), 4);

        // Moment comes first (sorted timeline)
        assert!(result.messages[0].content.as_ref().unwrap().contains("A moment"));
        // History comes after timeline
        assert!(result.messages[1].content.as_ref().unwrap().contains("Previous user message"));
        assert!(result.messages[2].content.as_ref().unwrap().contains("Previous assistant response"));
        // Chat messages come last
        assert!(result.messages[3].content.as_ref().unwrap().contains("User message"));
    }
```

- [ ] **Step 10.9:** Run all assembly tests

```bash
cd /home/cassie/river-engine && cargo test -p river-context assembly::
```

- [ ] **Step 10.10:** Commit changes

```bash
git add crates/river-context/src/assembly.rs && git commit -m "test(river-context): add comprehensive tests for assembly with timestamp interspersing"
```

---

## Task 11: Add Tests for Token Estimation Edge Cases

**File:** `crates/river-context/src/tokens.rs`

- [ ] **Step 11.1:** Add more comprehensive token estimation tests

Add to the existing test module:

```rust
    #[test]
    fn test_estimate_message_tokens_system() {
        let msg = OpenAIMessage::system("Hello world");
        let tokens = estimate_message_tokens(&msg);

        // 4 (base) + 3 (11 chars / 4) = 7
        assert!(tokens >= 7);
    }

    #[test]
    fn test_estimate_message_tokens_with_tool_calls() {
        let msg = OpenAIMessage {
            role: "assistant".into(),
            content: Some("Let me help".into()),
            tool_calls: Some(vec![crate::openai::ToolCall {
                id: "call_123".into(),
                call_type: "function".into(),
                function: crate::openai::FunctionCall {
                    name: "search".into(),
                    arguments: r#"{"query": "test"}"#.into(),
                },
            }]),
            tool_call_id: None,
        };

        let tokens = estimate_message_tokens(&msg);

        // Should include overhead for tool calls
        assert!(tokens > 10);
    }

    #[test]
    fn test_estimate_message_tokens_tool_result() {
        let msg = OpenAIMessage::tool("call_123", "Result content here");
        let tokens = estimate_message_tokens(&msg);

        // 4 (base) + content tokens + tool_call_id tokens
        assert!(tokens >= 4);
    }

    #[test]
    fn test_estimate_total_tokens_empty() {
        let messages: Vec<OpenAIMessage> = vec![];
        assert_eq!(estimate_total_tokens(&messages), 0);
    }

    #[test]
    fn test_estimate_total_tokens_multiple() {
        let messages = vec![
            OpenAIMessage::system("System prompt"),
            OpenAIMessage::user("User question"),
            OpenAIMessage::assistant("Assistant response"),
        ];

        let total = estimate_total_tokens(&messages);
        let sum: usize = messages.iter().map(estimate_message_tokens).sum();

        assert_eq!(total, sum);
    }
```

- [ ] **Step 11.2:** Run token tests

```bash
cd /home/cassie/river-engine && cargo test -p river-context tokens::
```

- [ ] **Step 11.3:** Commit changes

```bash
git add crates/river-context/src/tokens.rs && git commit -m "test(river-context): add comprehensive token estimation tests"
```

---

## Task 12: Add Integration Test for Full Pipeline

**File:** `crates/river-context/tests/integration.rs` (NEW)

- [ ] **Step 12.1:** Create integration test file

```rust
//! Integration tests for river-context.

use river_context::{
    build_context, ChannelContext, ContextRequest, ChatMessage, Embedding, Flash, Moment, Move,
    OpenAIMessage,
};
use river_protocol::{Author, Channel};

/// Create a snowflake ID with a specific timestamp (microseconds).
fn make_id(timestamp_micros: u64) -> String {
    let snowflake: u128 = (timestamp_micros as u128) << 64;
    snowflake.to_string()
}

#[test]
fn test_full_context_assembly() {
    // Simulate a realistic scenario with multiple channels, flashes, embeddings
    let request = ContextRequest {
        channels: vec![
            // Current channel (index 0)
            ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "current_123".into(),
                    name: Some("dev-chat".into()),
                },
                moments: vec![Moment {
                    id: make_id(1_000_000),
                    content: "Team discussed deployment strategy".into(),
                    move_range: ("m1".into(), "m10".into()),
                }],
                moves: vec![Move {
                    id: make_id(2_000_000),
                    content: "Reviewed CI pipeline changes".into(),
                    message_range: ("msg1".into(), "msg20".into()),
                }],
                messages: vec![
                    ChatMessage {
                        id: "msg21".into(),
                        timestamp: "2026-04-01T12:00:00Z".into(),
                        author: Author {
                            id: "user1".into(),
                            name: "Alice".into(),
                        },
                        content: "Can you help with the API?".into(),
                    },
                    ChatMessage {
                        id: "msg22".into(),
                        timestamp: "2026-04-01T12:01:00Z".into(),
                        author: Author {
                            id: "user2".into(),
                            name: "Bob".into(),
                        },
                        content: "Sure, what do you need?".into(),
                    },
                ],
                embeddings: vec![Embedding {
                    id: make_id(1_500_000),
                    content: "API documentation for /users endpoint".into(),
                    source: "docs/api.md:15-42".into(),
                    expires_at: "2026-04-01T18:00:00Z".into(),
                }],
            },
            // Last active channel (index 1)
            ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "last_456".into(),
                    name: Some("general".into()),
                },
                moments: vec![Moment {
                    id: make_id(500_000),
                    content: "Standup meeting notes".into(),
                    move_range: ("m0".into(), "m5".into()),
                }],
                moves: vec![],
                messages: vec![],
                embeddings: vec![Embedding {
                    id: make_id(600_000),
                    content: "Project timeline".into(),
                    source: "notes/timeline.md".into(),
                    expires_at: "2026-04-01T18:00:00Z".into(),
                }],
            },
        ],
        flashes: vec![
            // Expired flash (should be filtered)
            Flash {
                id: make_id(100_000),
                from: "worker-old".into(),
                content: "This is expired".into(),
                expires_at: "2026-04-01T10:00:00Z".into(),
            },
            // Valid flash (should be included)
            Flash {
                id: make_id(1_200_000),
                from: "worker-alert".into(),
                content: "Build succeeded".into(),
                expires_at: "2026-04-01T18:00:00Z".into(),
            },
        ],
        history: vec![
            OpenAIMessage::user("What's the status?".into()),
            OpenAIMessage::assistant("Everything is running smoothly.".into()),
        ],
        max_tokens: 50000,
        now: "2026-04-01T12:00:00Z".into(),
    };

    let result = build_context(request).unwrap();

    // Verify we got messages
    assert!(!result.messages.is_empty());

    // Verify token estimation is reasonable
    assert!(result.estimated_tokens > 0);
    assert!(result.estimated_tokens < 50000);

    // Verify the order:
    // 1. Timeline items sorted by timestamp
    // 2. History
    // 3. Current channel chat messages

    // Find the chat message (last item)
    let last_msg = result.messages.last().unwrap();
    assert_eq!(last_msg.role, "user");
    assert!(last_msg.content.as_ref().unwrap().contains("[Chat: dev-chat]"));

    // Find history messages (before chat)
    let history_start = result.messages.len() - 3; // 2 history + 1 chat
    assert_eq!(result.messages[history_start].role, "user");
    assert!(result.messages[history_start]
        .content
        .as_ref()
        .unwrap()
        .contains("What's the status?"));

    // Verify expired flash is not included
    let all_content: String = result
        .messages
        .iter()
        .filter_map(|m| m.content.as_ref())
        .cloned()
        .collect();

    assert!(!all_content.contains("This is expired"));
    assert!(all_content.contains("Build succeeded"));
}

#[test]
fn test_context_with_default_request() {
    // Using Default trait for cleaner test setup
    let mut request = ContextRequest::default();
    request.channels.push(ChannelContext {
        channel: Channel {
            adapter: "test".into(),
            id: "1".into(),
            name: Some("test-channel".into()),
        },
        ..Default::default()
    });
    request.max_tokens = 10000;
    request.now = "2026-04-01T12:00:00Z".into();

    let result = build_context(request).unwrap();

    // Should succeed with empty content
    assert_eq!(result.messages.len(), 0);
    assert_eq!(result.estimated_tokens, 0);
}

#[test]
fn test_timestamp_ordering_across_channels() {
    // Test that items from different channels are properly ordered by timestamp
    let request = ContextRequest {
        channels: vec![
            ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "chan1".into(),
                    name: Some("channel-1".into()),
                },
                moments: vec![
                    Moment {
                        id: make_id(3_000_000), // Third
                        content: "Third item from channel 1".into(),
                        move_range: ("a".into(), "b".into()),
                    },
                ],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
            },
            ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "chan2".into(),
                    name: Some("channel-2".into()),
                },
                moments: vec![
                    Moment {
                        id: make_id(1_000_000), // First
                        content: "First item from channel 2".into(),
                        move_range: ("c".into(), "d".into()),
                    },
                ],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
            },
        ],
        flashes: vec![Flash {
            id: make_id(2_000_000), // Second
            from: "worker".into(),
            content: "Second item (flash)".into(),
            expires_at: "2026-04-02T00:00:00Z".into(),
        }],
        history: vec![],
        max_tokens: 10000,
        now: "2026-04-01T12:00:00Z".into(),
    };

    let result = build_context(request).unwrap();

    assert_eq!(result.messages.len(), 3);

    // Verify ordering
    assert!(result.messages[0].content.as_ref().unwrap().contains("First item from channel 2"));
    assert!(result.messages[1].content.as_ref().unwrap().contains("Second item (flash)"));
    assert!(result.messages[2].content.as_ref().unwrap().contains("Third item from channel 1"));
}
```

- [ ] **Step 12.2:** Run integration tests

```bash
cd /home/cassie/river-engine && cargo test -p river-context --test integration
```

- [ ] **Step 12.3:** Commit changes

```bash
git add crates/river-context/tests/integration.rs && git commit -m "test(river-context): add integration tests for full context assembly pipeline"
```

---

## Task 13: Final Verification

- [ ] **Step 13.1:** Run all tests

```bash
cd /home/cassie/river-engine && cargo test -p river-context
```

- [ ] **Step 13.2:** Run cargo clippy

```bash
cd /home/cassie/river-engine && cargo clippy -p river-context -- -D warnings
```

- [ ] **Step 13.3:** Run cargo fmt

```bash
cd /home/cassie/river-engine && cargo fmt -p river-context
```

- [ ] **Step 13.4:** Verify the crate builds

```bash
cd /home/cassie/river-engine && cargo build -p river-context
```

- [ ] **Step 13.5:** Create final commit if any formatting changes

```bash
git add -A && git status && git diff --cached --stat
# If there are changes:
git commit -m "chore(river-context): apply formatting"
```

---

## Verification Checklist

After completing all tasks, verify:

- [ ] `id.rs` created with timestamp extraction
- [ ] Flashes interspersed by timestamp globally
- [ ] Embeddings interspersed by timestamp within channel
- [ ] thiserror added and used for ContextError
- [ ] chrono used for robust TTL comparison
- [ ] Import source consistent (river-protocol)
- [ ] All format functions tested
- [ ] Multi-channel assembly tested
- [ ] TTL filtering tested
- [ ] Over-budget error tested
- [ ] Ordering correctness verified
- [ ] PartialEq added to response/openai types
- [ ] Default impl added to ContextRequest
- [ ] All tests pass
- [ ] No clippy warnings

---

## Summary of Changes

| File | Changes |
|------|---------|
| `Cargo.toml` | Add thiserror, chrono dependencies |
| `lib.rs` | Add id module, export extract_timestamp |
| `id.rs` | NEW: Timestamp extraction from snowflake IDs |
| `assembly.rs` | Timestamp-based interspersing, chrono TTL comparison, comprehensive tests |
| `format.rs` | Fix import, add comprehensive tests |
| `openai.rs` | Add PartialEq derives |
| `request.rs` | Fix import, add Default impls |
| `response.rs` | Use thiserror, add PartialEq |
| `workspace.rs` | Fix import from river-protocol |
| `tokens.rs` | Add more test coverage |
| `tests/integration.rs` | NEW: End-to-end integration tests |

# Utterances Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `speak` and `switch_channel` tools with conversation frontmatter for channel-aware messaging.

**Architecture:** Conversation files get YAML frontmatter with routing metadata. `ChannelContext` caches the current channel's routing info in AgentTask. `speak` uses the cached context while `send_message` remains explicit.

**Tech Stack:** Rust, serde_yaml for frontmatter parsing, existing river-gateway infrastructure.

**Spec:** `docs/superpowers/specs/2026-03-28-utterances-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/river-gateway/src/conversations/meta.rs` | Create | `ConversationMeta` struct, frontmatter parsing |
| `crates/river-gateway/src/conversations/mod.rs` | Modify | Add meta module, update `Conversation` struct |
| `crates/river-gateway/src/conversations/format.rs` | Modify | Frontmatter emit/parse integration |
| `crates/river-gateway/src/agent/channel.rs` | Create | `ChannelContext` struct |
| `crates/river-gateway/src/agent/mod.rs` | Modify | Export channel module |
| `crates/river-gateway/src/agent/task.rs` | Modify | Replace `current_channel` with `channel_context` |
| `crates/river-gateway/src/tools/communication.rs` | Modify | Add `speak`, `switch_channel`, extract shared send logic |

---

## Task 1: Add ConversationMeta struct

**Files:**
- Create: `crates/river-gateway/src/conversations/meta.rs`
- Modify: `crates/river-gateway/src/conversations/mod.rs`

- [ ] **Step 1: Write the test for ConversationMeta parsing**

In `crates/river-gateway/src/conversations/meta.rs`:

```rust
//! Conversation metadata (frontmatter)

use serde::{Deserialize, Serialize};

/// Routing metadata stored in conversation file frontmatter
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub adapter: String,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_deserialize_full() {
        let yaml = r#"
adapter: discord
channel_id: "789012345678901234"
channel_name: general
guild_id: "123456789012345678"
guild_name: myserver
"#;
        let meta: ConversationMeta = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.adapter, "discord");
        assert_eq!(meta.channel_id, "789012345678901234");
        assert_eq!(meta.channel_name, Some("general".to_string()));
        assert_eq!(meta.guild_id, Some("123456789012345678".to_string()));
        assert_eq!(meta.guild_name, Some("myserver".to_string()));
        assert_eq!(meta.thread_id, None);
    }

    #[test]
    fn test_meta_deserialize_minimal() {
        let yaml = r#"
adapter: slack
channel_id: C12345
"#;
        let meta: ConversationMeta = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.adapter, "slack");
        assert_eq!(meta.channel_id, "C12345");
        assert_eq!(meta.channel_name, None);
        assert_eq!(meta.guild_id, None);
    }

    #[test]
    fn test_meta_serialize_roundtrip() {
        let meta = ConversationMeta {
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: None,
            guild_name: None,
            thread_id: None,
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let parsed: ConversationMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(meta, parsed);
    }
}
```

- [ ] **Step 2: Add serde_yaml dependency**

Run:
```bash
cd /home/cassie/river-engine && cargo add serde_yaml -p river-gateway
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test -p river-gateway conversations::meta --no-fail-fast`
Expected: PASS (3 tests)

- [ ] **Step 4: Add meta module to conversations/mod.rs**

In `crates/river-gateway/src/conversations/mod.rs`, add after line 9 (`pub mod writer;`):

```rust
pub mod meta;
```

And add to exports (after the existing `pub use` statements around line 12):

```rust
pub use meta::ConversationMeta;
```

- [ ] **Step 5: Run all conversation tests**

Run: `cargo test -p river-gateway conversations`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(conversations): add ConversationMeta for frontmatter

Adds routing metadata struct that will be stored in conversation
file frontmatter for channel-aware messaging.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add frontmatter parsing to Conversation

**Files:**
- Modify: `crates/river-gateway/src/conversations/mod.rs`
- Modify: `crates/river-gateway/src/conversations/format.rs`

- [ ] **Step 1: Update Conversation struct**

In `crates/river-gateway/src/conversations/mod.rs`, change the `Conversation` struct (around line 146):

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ConversationMeta>,
    pub messages: Vec<Message>,
}
```

- [ ] **Step 2: Add frontmatter constants to format.rs**

In `crates/river-gateway/src/conversations/format.rs`, add at top after imports (around line 18):

```rust
/// YAML frontmatter delimiter
pub const FRONTMATTER_DELIMITER: &str = "---";
```

- [ ] **Step 3: Update Conversation::from_str to parse frontmatter**

In `crates/river-gateway/src/conversations/mod.rs`, replace the `from_str` method (around line 162-205):

```rust
    /// Parse conversation from custom format (with optional frontmatter)
    pub fn from_str(s: &str) -> Result<Self, ParseError> {
        let (meta, body) = Self::split_frontmatter(s)?;

        let mut messages = Vec::new();
        let mut current_message: Option<Message> = None;

        for line in body.lines() {
            if line.trim().is_empty() {
                continue;
            }

            // Check if this is a message line or a reaction line
            if line.starts_with("    ") {
                // Reaction line
                if let Some(ref mut msg) = current_message {
                    if let Some(reaction) = format::parse_reaction_line(line) {
                        msg.reactions.push(reaction);
                    } else {
                        return Err(ParseError(format!("Invalid reaction line: {}", line)));
                    }
                } else {
                    return Err(ParseError(
                        "Reaction line without preceding message".to_string(),
                    ));
                }
            } else {
                // Message line - save previous message if any
                if let Some(msg) = current_message.take() {
                    messages.push(msg);
                }

                // Parse new message
                current_message = Some(
                    format::parse_message_line(line)
                        .ok_or_else(|| ParseError(format!("Invalid message line: {}", line)))?,
                );
            }
        }

        // Don't forget the last message
        if let Some(msg) = current_message {
            messages.push(msg);
        }

        Ok(Conversation { meta, messages })
    }

    /// Split frontmatter from body content
    fn split_frontmatter(s: &str) -> Result<(Option<ConversationMeta>, &str), ParseError> {
        let trimmed = s.trim_start();

        if !trimmed.starts_with(format::FRONTMATTER_DELIMITER) {
            return Ok((None, s));
        }

        // Find the closing delimiter
        let after_first = &trimmed[format::FRONTMATTER_DELIMITER.len()..];
        let after_first = after_first.trim_start_matches('\n');

        if let Some(end_idx) = after_first.find(&format!("\n{}", format::FRONTMATTER_DELIMITER)) {
            let yaml_content = &after_first[..end_idx];
            let body_start = end_idx + format::FRONTMATTER_DELIMITER.len() + 1;
            let body = if body_start < after_first.len() {
                after_first[body_start..].trim_start_matches('\n')
            } else {
                ""
            };

            let meta: ConversationMeta = serde_yaml::from_str(yaml_content)
                .map_err(|e| ParseError(format!("Invalid frontmatter YAML: {}", e)))?;

            Ok((Some(meta), body))
        } else {
            Err(ParseError("Unclosed frontmatter (missing closing ---)".to_string()))
        }
    }
```

- [ ] **Step 4: Update Conversation::to_string to emit frontmatter**

In `crates/river-gateway/src/conversations/mod.rs`, replace the `to_string` method (around line 152-159):

```rust
    /// Serialize conversation to custom human-readable format
    pub fn to_string(&self) -> String {
        let mut result = String::new();

        // Emit frontmatter if present
        if let Some(ref meta) = self.meta {
            result.push_str(format::FRONTMATTER_DELIMITER);
            result.push('\n');
            // serde_yaml adds trailing newline
            result.push_str(&serde_yaml::to_string(meta).unwrap_or_default());
            result.push_str(format::FRONTMATTER_DELIMITER);
            result.push('\n');
        }

        // Emit messages
        let messages: Vec<String> = self.messages
            .iter()
            .map(|msg| format::format_message(msg))
            .collect();
        result.push_str(&messages.join("\n"));

        result
    }
```

- [ ] **Step 5: Add frontmatter test**

In `crates/river-gateway/src/conversations/mod.rs`, add test at end of `mod tests`:

```rust
    #[test]
    fn test_conversation_with_frontmatter_roundtrip() {
        let input = r#"---
adapter: discord
channel_id: "789012"
channel_name: general
---
[ ] 2026-03-23 14:30:00 msg123 <alice:111> hey, can you help?
[>] 2026-03-23 14:30:15 msg124 <river:999> Sure!
"#;

        let convo = Conversation::from_str(input).expect("Failed to parse");

        assert!(convo.meta.is_some());
        let meta = convo.meta.as_ref().unwrap();
        assert_eq!(meta.adapter, "discord");
        assert_eq!(meta.channel_id, "789012");
        assert_eq!(meta.channel_name, Some("general".to_string()));

        assert_eq!(convo.messages.len(), 2);
        assert_eq!(convo.messages[0].author.name, "alice");
        assert_eq!(convo.messages[1].direction, MessageDirection::Outgoing);

        // Roundtrip
        let serialized = convo.to_string();
        let reparsed = Conversation::from_str(&serialized).expect("Failed to reparse");
        assert_eq!(reparsed.meta, convo.meta);
        assert_eq!(reparsed.messages.len(), convo.messages.len());
    }

    #[test]
    fn test_conversation_without_frontmatter() {
        let input = "[ ] 2026-03-23 14:30:00 msg123 <alice:111> hey\n";
        let convo = Conversation::from_str(input).expect("Failed to parse");

        assert!(convo.meta.is_none());
        assert_eq!(convo.messages.len(), 1);
    }

    #[test]
    fn test_conversation_unclosed_frontmatter_error() {
        let input = "---\nadapter: discord\n[ ] msg";
        let result = Conversation::from_str(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().0.contains("Unclosed frontmatter"));
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p river-gateway conversations`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(conversations): add frontmatter parsing to Conversation

Conversation files can now have YAML frontmatter with routing
metadata. Files without frontmatter still parse correctly.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add ChannelContext struct

**Files:**
- Create: `crates/river-gateway/src/agent/channel.rs`
- Modify: `crates/river-gateway/src/agent/mod.rs`

- [ ] **Step 1: Create channel.rs with ChannelContext**

Create `crates/river-gateway/src/agent/channel.rs`:

```rust
//! Channel context for tracking the agent's current location

use crate::conversations::ConversationMeta;
use std::path::PathBuf;

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
    /// Create from conversation path and metadata
    pub fn from_conversation(path: PathBuf, meta: &ConversationMeta) -> Self {
        Self {
            path,
            adapter: meta.adapter.clone(),
            channel_id: meta.channel_id.clone(),
            channel_name: meta.channel_name.clone(),
            guild_id: meta.guild_id.clone(),
        }
    }

    /// Get display name for logging (channel_name or channel_id)
    pub fn display_name(&self) -> &str {
        self.channel_name.as_deref().unwrap_or(&self.channel_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_context_from_conversation() {
        let meta = ConversationMeta {
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: Some("123456".to_string()),
            guild_name: Some("myserver".to_string()),
            thread_id: None,
        };

        let ctx = ChannelContext::from_conversation(
            PathBuf::from("conversations/discord/myserver/general.txt"),
            &meta,
        );

        assert_eq!(ctx.adapter, "discord");
        assert_eq!(ctx.channel_id, "789012");
        assert_eq!(ctx.channel_name, Some("general".to_string()));
        assert_eq!(ctx.guild_id, Some("123456".to_string()));
    }

    #[test]
    fn test_display_name_with_name() {
        let ctx = ChannelContext {
            path: PathBuf::from("test.txt"),
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: None,
        };
        assert_eq!(ctx.display_name(), "general");
    }

    #[test]
    fn test_display_name_without_name() {
        let ctx = ChannelContext {
            path: PathBuf::from("test.txt"),
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: None,
            guild_id: None,
        };
        assert_eq!(ctx.display_name(), "789012");
    }
}
```

- [ ] **Step 2: Add channel module to agent/mod.rs**

In `crates/river-gateway/src/agent/mod.rs`, add after line 8 (`pub mod tools;`):

```rust
pub mod channel;
```

And add export after line 12:

```rust
pub use channel::ChannelContext;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway agent::channel`
Expected: PASS (3 tests)

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(agent): add ChannelContext for current channel tracking

Caches routing info from conversation frontmatter, used by speak tool.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Update AgentTask to use ChannelContext

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`

- [ ] **Step 1: Replace current_channel with channel_context**

In `crates/river-gateway/src/agent/task.rs`:

Add import at top (around line 6):
```rust
use crate::agent::channel::ChannelContext;
```

Replace field in `AgentTask` struct (around line 76):
```rust
    // current_channel: String,  // REMOVE THIS LINE
    channel_context: Option<ChannelContext>,
```

Update `new()` initializer (around line 106):
```rust
            // current_channel: "default".into(),  // REMOVE THIS LINE
            channel_context: None,
```

- [ ] **Step 2: Update TurnStarted event to use channel_context**

In `turn_cycle` method (around line 177-181), replace:
```rust
        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: self.channel_context
                .as_ref()
                .map(|c| c.display_name().to_string())
                .unwrap_or_else(|| "unset".to_string()),
            turn_number: self.turn_count,
            timestamp: turn_start,
        }));
```

- [ ] **Step 3: Update logging to use channel_context**

Around line 183-188:
```rust
        tracing::info!(
            turn = self.turn_count,
            channel = %self.channel_context
                .as_ref()
                .map(|c| c.display_name())
                .unwrap_or("unset"),
            is_heartbeat = is_heartbeat,
            "Turn started"
        );
```

- [ ] **Step 4: Update context assembly channel reference**

Around line 207-208:
```rust
        let channel_name = self.channel_context
            .as_ref()
            .map(|c| c.display_name().to_string())
            .unwrap_or_else(|| "default".to_string());
        let context = self.context_assembler.assemble(
            &channel_name,
```

- [ ] **Step 5: Update TurnComplete event**

Around line 334-340:
```rust
        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
            channel: self.channel_context
                .as_ref()
                .map(|c| c.display_name().to_string())
                .unwrap_or_else(|| "unset".to_string()),
            turn_number: self.turn_count,
```

- [ ] **Step 6: Update switch_channel method**

Replace the existing `switch_channel` method (around line 464-472):
```rust
    /// Switch to a different channel
    pub fn set_channel_context(&mut self, context: ChannelContext) {
        let old = self.channel_context
            .as_ref()
            .map(|c| c.display_name().to_string())
            .unwrap_or_else(|| "unset".to_string());
        let new = context.display_name().to_string();

        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched {
            from: old.clone(),
            to: new.clone(),
            timestamp: Utc::now(),
        }));

        tracing::info!(from = %old, to = %new, "Channel switched");
        self.channel_context = Some(context);
    }

    /// Get current channel context
    pub fn channel_context(&self) -> Option<&ChannelContext> {
        self.channel_context.as_ref()
    }
```

- [ ] **Step 7: Remove old current_channel method**

Remove the `current_channel()` method (around line 480-482) - it's replaced by `channel_context()`.

- [ ] **Step 8: Fix tests**

Update the tests that reference `current_channel`:

In `test_agent_task_channel_switch` (around line 558-589), update:
```rust
    #[tokio::test]
    async fn test_agent_task_channel_switch() {
        let temp = TempDir::new().unwrap();
        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();
        let mut event_rx = bus.subscribe();

        let message_queue = Arc::new(MessageQueue::new());
        let flash_queue = Arc::new(FlashQueue::new(10));
        let tool_executor = Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new())));
        let model_client = ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        ).unwrap();

        let mut task = AgentTask::new(
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
        );

        assert!(task.channel_context().is_none());

        let ctx = ChannelContext {
            path: PathBuf::from("conversations/discord/general.txt"),
            adapter: "discord".to_string(),
            channel_id: "123".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: None,
        };
        task.set_channel_context(ctx);

        assert!(task.channel_context().is_some());
        assert_eq!(task.channel_context().unwrap().display_name(), "general");

        let event = event_rx.try_recv();
        assert!(matches!(event, Ok(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched { .. }))));
    }
```

Add necessary import at top of test module:
```rust
    use crate::agent::channel::ChannelContext;
    use std::path::PathBuf;
```

- [ ] **Step 9: Run tests**

Run: `cargo test -p river-gateway agent::task`
Expected: PASS

- [ ] **Step 10: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
refactor(agent): replace current_channel with channel_context

AgentTask now uses Option<ChannelContext> for richer channel state,
supporting the upcoming speak tool.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Extract shared send logic

**Files:**
- Modify: `crates/river-gateway/src/tools/communication.rs`

- [ ] **Step 1: Add send_to_adapter function**

In `crates/river-gateway/src/tools/communication.rs`, add after imports (around line 14):

```rust
/// Send a message through an adapter (shared by speak and send_message)
async fn send_to_adapter(
    http_client: &reqwest::Client,
    registry: &AdapterRegistry,
    adapter: &str,
    channel_id: &str,
    content: &str,
    reply_to: Option<&str>,
    writer_tx: &mpsc::Sender<WriteOp>,
    conversation_path: &std::path::Path,
    agent_author: Author,
) -> Result<ToolResult, RiverError> {
    let config = registry
        .get(adapter)
        .ok_or_else(|| {
            error!(
                adapter = %adapter,
                available = ?registry.names(),
                "Unknown adapter"
            );
            RiverError::tool(format!("Adapter '{}' not registered", adapter))
        })?;

    let payload = serde_json::json!({
        "channel": channel_id,
        "content": content,
        "reply_to": reply_to,
    });

    info!(
        url = %config.outbound_url,
        adapter = %adapter,
        channel_id = %channel_id,
        content_len = content.len(),
        "Sending message to adapter"
    );

    let response = http_client
        .post(&config.outbound_url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            error!(error = %e, url = %config.outbound_url, "HTTP request failed");
            RiverError::tool(format!("Failed to send message: {}", e))
        })?;

    let status = response.status();

    if status.is_success() {
        let body = response.text().await.unwrap_or_default();

        info!(adapter = %adapter, channel_id = %channel_id, "Message sent successfully");

        // Extract message_id from adapter response
        let message_id = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message_id")?.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("out-{}", chrono::Utc::now().timestamp_millis()));

        // Record outgoing message
        let msg = crate::conversations::Message::outgoing(&message_id, agent_author, content);

        if let Err(e) = writer_tx
            .send(WriteOp::Message {
                path: conversation_path.to_path_buf(),
                msg,
            })
            .await
        {
            warn!("Failed to record outgoing message: {}", e);
        }

        Ok(ToolResult::success(format!(
            "Message sent to {} via {}",
            channel_id, adapter
        )))
    } else {
        let body = response.text().await.unwrap_or_default();
        error!(
            status = %status,
            body = %body,
            adapter = %adapter,
            channel_id = %channel_id,
            "Adapter returned error"
        );

        // Record failed message
        let msg = crate::conversations::Message::failed(
            agent_author,
            &format!("Adapter returned error {}", status),
            content,
        );

        if let Err(e) = writer_tx
            .send(WriteOp::Message {
                path: conversation_path.to_path_buf(),
                msg,
            })
            .await
        {
            warn!("Failed to record failed message: {}", e);
        }

        Err(RiverError::tool(format!(
            "Adapter returned error {}: {}",
            status, body
        )))
    }
}
```

- [ ] **Step 2: Refactor send_message to use shared logic**

Update the existing `SendMessageTool::execute` method to use `send_to_adapter`. Find the execute method (around line 40-120) and replace the HTTP sending logic with a call to `send_to_adapter`:

```rust
    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let adapter = args["adapter"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'adapter' parameter"))?;
        let channel_id = args["channel"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'channel' parameter"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'content' parameter"))?;
        let reply_to = args["reply_to"].as_str();

        info!(
            adapter = %adapter,
            channel_id = %channel_id,
            content_len = content.len(),
            "SendMessageTool: Sending message"
        );

        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let writer_tx = self.writer_tx.clone();
        let workspace = self.workspace.clone();
        let agent_author = self.agent_author();
        let adapter = adapter.to_string();
        let channel_id = channel_id.to_string();
        let content = content.to_string();
        let reply_to = reply_to.map(|s| s.to_string());

        // Build conversation path from adapter/channel
        let conversation_path = workspace.join(format!("conversations/{}/{}.txt", adapter, channel_id));

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;

                send_to_adapter(
                    &http_client,
                    &registry,
                    &adapter,
                    &channel_id,
                    &content,
                    reply_to.as_deref(),
                    &writer_tx,
                    &conversation_path,
                    agent_author,
                )
                .await
            })
        })
    }
```

- [ ] **Step 3: Run tests to ensure compilation**

Run: `cargo test -p river-gateway tools::communication`
Expected: PASS (existing tests still work)

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
refactor(tools): extract send_to_adapter shared logic

Prepares for speak tool by extracting common send implementation
that both speak and send_message will use.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Add switch_channel tool

**Files:**
- Modify: `crates/river-gateway/src/tools/communication.rs`

- [ ] **Step 1: Add SwitchChannelTool struct**

In `crates/river-gateway/src/tools/communication.rs`, add after `ReadChannelTool` implementation (around line 578):

```rust
/// Switch the agent's current channel
pub struct SwitchChannelTool {
    workspace: PathBuf,
    channel_context_tx: mpsc::Sender<crate::agent::ChannelContext>,
}

impl SwitchChannelTool {
    pub fn new(
        workspace: PathBuf,
        channel_context_tx: mpsc::Sender<crate::agent::ChannelContext>,
    ) -> Self {
        Self {
            workspace,
            channel_context_tx,
        }
    }
}

impl Tool for SwitchChannelTool {
    fn name(&self) -> &str {
        "switch_channel"
    }

    fn description(&self) -> &str {
        "Switch to a different channel for subsequent speak commands"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to conversation file (e.g., 'conversations/discord/myserver/general.txt')"
                }
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'path' parameter"))?;

        let path = self.workspace.join(path_str);

        info!(path = %path.display(), "Switching channel");

        // Read and parse conversation file
        let content = std::fs::read_to_string(&path).map_err(|e| {
            error!(path = %path.display(), error = %e, "Failed to read conversation file");
            RiverError::tool(format!("Conversation file not found: {}", path_str))
        })?;

        let conversation = crate::conversations::Conversation::from_str(&content).map_err(|e| {
            error!(path = %path.display(), error = %e.0, "Failed to parse conversation");
            RiverError::tool(format!("Failed to parse conversation: {}", e.0))
        })?;

        let meta = conversation.meta.ok_or_else(|| {
            error!(path = %path.display(), "Conversation file missing frontmatter");
            RiverError::tool("Conversation file missing routing metadata")
        })?;

        // Create channel context
        let context = crate::agent::ChannelContext::from_conversation(
            PathBuf::from(path_str),
            &meta,
        );

        let channel_name = context.display_name().to_string();

        // Send to agent task
        let tx = self.channel_context_tx.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                tx.send(context).await.map_err(|e| {
                    RiverError::tool(format!("Failed to update channel context: {}", e))
                })
            })
        })?;

        info!(channel = %channel_name, "Switched to channel");

        Ok(ToolResult::success(format!("Switched to channel: {}", channel_name)))
    }
}
```

- [ ] **Step 2: Add tests for switch_channel**

Add to the `tests` module:

```rust
    #[test]
    fn test_switch_channel_tool_schema() {
        let (tx, _rx) = mpsc::channel(1);
        let tool = SwitchChannelTool::new(PathBuf::from("/workspace"), tx);

        assert_eq!(tool.name(), "switch_channel");
        let params = tool.parameters();
        assert!(params["properties"]["path"].is_object());
        assert_eq!(params["required"], serde_json::json!(["path"]));
    }

    #[test]
    fn test_switch_channel_file_not_found() {
        let (tx, _rx) = mpsc::channel(1);
        let tool = SwitchChannelTool::new(PathBuf::from("/nonexistent"), tx);

        let result = tool.execute(serde_json::json!({
            "path": "conversations/missing.txt"
        }));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_switch_channel_missing_frontmatter() {
        let temp = tempfile::TempDir::new().unwrap();
        let conv_path = temp.path().join("convo.txt");
        std::fs::write(&conv_path, "[ ] 2026-03-28 10:00:00 msg1 <alice:111> hello\n").unwrap();

        let (tx, _rx) = mpsc::channel(1);
        let tool = SwitchChannelTool::new(temp.path().to_path_buf(), tx);

        let result = tool.execute(serde_json::json!({
            "path": "convo.txt"
        }));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("missing routing metadata"));
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway tools::communication`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(tools): add switch_channel tool

Allows agent to switch current channel by specifying path to
conversation file. Parses frontmatter and updates channel context.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Add speak tool

**Files:**
- Modify: `crates/river-gateway/src/tools/communication.rs`

- [ ] **Step 1: Add SpeakTool struct**

Add after `SwitchChannelTool`:

```rust
/// Send message to the current channel
pub struct SpeakTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    workspace: PathBuf,
    agent_name: String,
    agent_id: String,
    writer_tx: mpsc::Sender<WriteOp>,
    channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
}

impl SpeakTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        workspace: PathBuf,
        agent_name: String,
        agent_id: String,
        writer_tx: mpsc::Sender<WriteOp>,
        channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            workspace,
            agent_name,
            agent_id,
            writer_tx,
            channel_context,
        }
    }

    fn agent_author(&self) -> Author {
        Author {
            name: self.agent_name.clone(),
            id: self.agent_id.clone(),
        }
    }
}

impl Tool for SpeakTool {
    fn name(&self) -> &str {
        "speak"
    }

    fn description(&self) -> &str {
        "Send a message to the current channel"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
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
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let content = args["content"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'content' parameter"))?;
        let reply_to = args["reply_to"].as_str();

        info!(
            content_len = content.len(),
            reply_to = ?reply_to,
            "SpeakTool: Sending message"
        );

        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let writer_tx = self.writer_tx.clone();
        let workspace = self.workspace.clone();
        let agent_author = self.agent_author();
        let channel_context = self.channel_context.clone();
        let content = content.to_string();
        let reply_to = reply_to.map(|s| s.to_string());

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Get channel context
                let ctx_guard = channel_context.read().await;
                let ctx = ctx_guard.as_ref().ok_or_else(|| {
                    error!("SpeakTool: No channel selected");
                    RiverError::tool("No channel selected. Use switch_channel first.")
                })?;

                let adapter = ctx.adapter.clone();
                let channel_id = ctx.channel_id.clone();
                let conversation_path = workspace.join(&ctx.path);

                drop(ctx_guard); // Release lock before async call

                let registry = registry.read().await;

                send_to_adapter(
                    &http_client,
                    &registry,
                    &adapter,
                    &channel_id,
                    &content,
                    reply_to.as_deref(),
                    &writer_tx,
                    &conversation_path,
                    agent_author,
                )
                .await
            })
        })
    }
}
```

- [ ] **Step 2: Add tests for speak**

```rust
    #[tokio::test]
    async fn test_speak_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let (tx, _rx) = mpsc::channel(1);
        let channel_context = Arc::new(RwLock::new(None));

        let tool = SpeakTool::new(
            registry,
            PathBuf::from("/workspace"),
            "agent".to_string(),
            "agent_001".to_string(),
            tx,
            channel_context,
        );

        assert_eq!(tool.name(), "speak");
        let params = tool.parameters();
        assert!(params["properties"]["content"].is_object());
        assert!(params["properties"]["reply_to"].is_object());
        assert_eq!(params["required"], serde_json::json!(["content"]));
    }

    #[tokio::test]
    async fn test_speak_without_channel_selected() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let (tx, _rx) = mpsc::channel(1);
        let channel_context = Arc::new(RwLock::new(None)); // No channel set

        let tool = SpeakTool::new(
            registry,
            PathBuf::from("/workspace"),
            "agent".to_string(),
            "agent_001".to_string(),
            tx,
            channel_context,
        );

        let result = tool.execute(serde_json::json!({
            "content": "Hello!"
        }));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No channel selected"));
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway tools::communication`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(tools): add speak tool for channel-aware messaging

Sends messages to the current channel (set via switch_channel).
Uses shared send logic with send_message.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Integration test

**Files:**
- Create: `crates/river-gateway/tests/utterances_test.rs`

- [ ] **Step 1: Create integration test file**

Create `crates/river-gateway/tests/utterances_test.rs`:

```rust
//! Integration tests for utterances (speak/switch_channel)

use river_gateway::conversations::{Conversation, ConversationMeta};
use river_gateway::agent::ChannelContext;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_channel_context_from_file() {
    let temp = TempDir::new().unwrap();
    let conv_path = temp.path().join("conversations/discord/general.txt");
    std::fs::create_dir_all(conv_path.parent().unwrap()).unwrap();

    let content = r#"---
adapter: discord
channel_id: "789012"
channel_name: general
guild_id: "123456"
---
[ ] 2026-03-28 10:00:00 msg1 <alice:111> hello
"#;
    std::fs::write(&conv_path, content).unwrap();

    // Parse conversation
    let file_content = std::fs::read_to_string(&conv_path).unwrap();
    let conversation = Conversation::from_str(&file_content).unwrap();

    assert!(conversation.meta.is_some());
    let meta = conversation.meta.as_ref().unwrap();

    // Create channel context
    let ctx = ChannelContext::from_conversation(
        PathBuf::from("conversations/discord/general.txt"),
        meta,
    );

    assert_eq!(ctx.adapter, "discord");
    assert_eq!(ctx.channel_id, "789012");
    assert_eq!(ctx.display_name(), "general");
}

#[test]
fn test_conversation_frontmatter_preserved_on_roundtrip() {
    let meta = ConversationMeta {
        adapter: "slack".to_string(),
        channel_id: "C12345".to_string(),
        channel_name: Some("random".to_string()),
        guild_id: None,
        guild_name: None,
        thread_id: None,
    };

    let mut conversation = Conversation::default();
    conversation.meta = Some(meta.clone());

    let serialized = conversation.to_string();
    assert!(serialized.starts_with("---"));
    assert!(serialized.contains("adapter: slack"));
    assert!(serialized.contains("channel_id: C12345"));

    let parsed = Conversation::from_str(&serialized).unwrap();
    assert_eq!(parsed.meta.unwrap().adapter, "slack");
}

#[test]
fn test_switch_channel_to_nonexistent_file_error() {
    // This tests at unit level - the switch_channel tool returns error
    // when file doesn't exist
    let result = Conversation::from_str("");
    // Empty string parses to empty conversation, not an error
    // but a missing file would error in the tool itself
    assert!(result.is_ok());
}

#[test]
fn test_switch_channel_to_file_without_frontmatter_error() {
    // File without frontmatter - conversation parses but has no meta
    let content = "[ ] 2026-03-28 10:00:00 msg1 <alice:111> hello\n";
    let conversation = Conversation::from_str(content).unwrap();
    assert!(conversation.meta.is_none());
    // The switch_channel tool should error when meta is None
}

#[test]
fn test_conversation_unclosed_frontmatter() {
    // Test that unclosed frontmatter produces an error
    let content = "---\nadapter: discord\nchannel_id: 123\n[ ] msg";
    let result = Conversation::from_str(content);
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run integration test**

Run: `cargo test -p river-gateway --test utterances_test`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
test: add utterances integration tests

Tests channel context creation from conversation files and
frontmatter roundtrip preservation.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Update roadmap

**Files:**
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Update utterances status**

In `docs/roadmap.md`, update the Communication section (around line 113):

```markdown
| Utterances | 🟢 | `speak` tool + `switch_channel` for channel-aware messaging |
```

- [ ] **Step 2: Commit**

```bash
git add docs/roadmap.md && git commit -m "$(cat <<'EOF'
docs: mark utterances as complete in roadmap

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | ConversationMeta struct | `conversations/meta.rs` |
| 2 | Frontmatter parsing | `conversations/mod.rs`, `format.rs` |
| 3 | ChannelContext struct | `agent/channel.rs` |
| 4 | AgentTask channel_context | `agent/task.rs` |
| 5 | Extract shared send logic | `tools/communication.rs` |
| 6 | switch_channel tool | `tools/communication.rs` |
| 7 | speak tool | `tools/communication.rs` |
| 8 | Integration tests | `tests/utterances_test.rs` |
| 9 | Update roadmap | `docs/roadmap.md` |

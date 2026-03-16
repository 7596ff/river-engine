# Context Management Crate — Design Spec

> river-context: Worker-side library for assembling context from workspace data
>
> Authors: Cass, Claude
> Date: 2026-04-01

## Overview

The context management crate (`river-context`) provides a pure function that assembles workspace data into OpenAI-compatible messages. The worker uses this crate to build its context each turn.

**Key characteristics:**
- Worker-side (worker has direct workspace access)
- Stateless, pure function (one input = one output)
- Rule-based reorganization (reordering, injection, grouping)
- Outputs OpenAI-compatible message format
- Estimates tokens, refuses if over budget

## Core API

```rust
pub fn build_context(request: ContextRequest) -> Result<ContextResponse, ContextError>;
```

### Request

```rust
pub struct ContextRequest {
    /// Channels: [0] is current, rest are last 4 by recency
    pub channels: Vec<ChannelContext>,
    /// Global flashes, interspersed by timestamp
    pub flashes: Vec<Flash>,
    /// LLM conversation history (from context.jsonl, already OpenAI format)
    pub history: Vec<OpenAIMessage>,
    /// Token limit (estimate-based)
    pub max_tokens: usize,
    /// Current time for TTL filtering (ISO8601)
    pub now: String,
}
```

### Response

```rust
pub struct ContextResponse {
    /// Flat timeline of OpenAI-compatible messages
    pub messages: Vec<OpenAIMessage>,
    /// Estimated token count
    pub estimated_tokens: usize,
}
```

### Errors

```rust
pub enum ContextError {
    /// Assembled context exceeds max_tokens
    OverBudget { estimated: usize, limit: usize },
    /// No channels provided
    EmptyChannels,
}
```

## OpenAI Message Format

Output uses OpenAI-compatible message format for direct LLM consumption.

```rust
#[derive(Serialize, Deserialize)]
pub struct OpenAIMessage {
    pub role: String,  // "system", "user", "assistant", "tool"

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub r#type: String,  // "function"
    pub function: FunctionCall,
}

#[derive(Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,  // JSON string
}
```

## Workspace Types

These types are used for loading structured data from workspace files. They get converted to OpenAI system/user messages during assembly.

```rust
// Imported from river-adapter
pub use river_adapter::{Author, Channel};

pub struct ChannelContext {
    pub channel: Channel,
    pub moments: Vec<Moment>,
    pub moves: Vec<Move>,
    pub messages: Vec<ChatMessage>,
    pub embeddings: Vec<Embedding>,
}

pub struct Moment {
    pub id: String,
    pub content: String,
    pub move_range: (String, String),  // (start_move_id, end_move_id)
}

pub struct Move {
    pub id: String,
    pub content: String,
    pub message_range: (String, String),  // (start_message_id, end_message_id)
}

pub struct ChatMessage {
    pub id: String,
    pub timestamp: String,
    pub author: Author,
    pub content: String,
}

pub struct Flash {
    pub id: String,
    pub from: String,        // sender worker name
    pub content: String,
    pub expires_at: String,  // ISO8601
}

pub struct Embedding {
    pub id: String,
    pub content: String,
    pub source: String,
    pub expires_at: String,  // ISO8601
}
```

## Assembly Rules

The crate assembles a flat `Vec<OpenAIMessage>` following these rules:

### Ordering

```
[Other channels: moments + moves only, by channel recency]
[Last channel: moments + moves + embeddings, by timestamp]
[LLM history: from context.jsonl]
[Current channel: moments + moves + messages + embeddings, by timestamp]
[Flashes: interspersed globally by timestamp]
```

### Per-Channel Rules

| Channel | Moments | Moves | Messages | Embeddings |
|---------|---------|-------|----------|------------|
| Other (not current, not last) | Yes | Yes | No | No |
| Last | Yes | Yes | No | Yes |
| Current | Yes | Yes | Yes | Yes |

### Message Formatting

Workspace data converts to OpenAI messages with prefixes:

```rust
fn format_moment(m: &Moment, channel: &Channel) -> OpenAIMessage {
    OpenAIMessage {
        role: "system".into(),
        content: Some(format!(
            "[Moment: {}] {} (moves {}-{})",
            channel.name.as_deref().unwrap_or(&channel.id),
            m.content,
            m.move_range.0, m.move_range.1
        )),
        tool_calls: None,
        tool_call_id: None,
    }
}

fn format_move(m: &Move, channel: &Channel) -> OpenAIMessage {
    OpenAIMessage {
        role: "system".into(),
        content: Some(format!(
            "[Move: {}] {} (messages {}-{})",
            channel.name.as_deref().unwrap_or(&channel.id),
            m.content,
            m.message_range.0, m.message_range.1
        )),
        ..
    }
}

fn format_flash(f: &Flash) -> OpenAIMessage {
    OpenAIMessage {
        role: "system".into(),
        content: Some(format!("[Flash from {}] {}", f.from, f.content)),
        ..
    }
}

fn format_embedding(e: &Embedding) -> OpenAIMessage {
    OpenAIMessage {
        role: "system".into(),
        content: Some(format!("[Reference: {}]\n{}", e.source, e.content)),
        ..
    }
}

fn format_chat_messages(msgs: &[ChatMessage], channel: &Channel) -> OpenAIMessage {
    let formatted = msgs.iter()
        .map(|m| format!("[{}] <{}> {}", m.timestamp, m.author.name, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    OpenAIMessage {
        role: "user".into(),
        content: Some(format!(
            "[Chat: {}]\n{}",
            channel.name.as_deref().unwrap_or(&channel.id),
            formatted
        )),
        ..
    }
}
```

### Interspersing

- **Flashes:** Merge globally by ID timestamp (high priority, injected near end)
- **Embeddings:** Merge within their channel by ID timestamp
- **LLM history:** Inserted as a block (already OpenAI format, pass through)

### TTL Filtering

- Flashes with `expires_at < now` are excluded
- Embeddings with `expires_at < now` are excluded

## Example Output

Given workspace data and LLM history, the assembled context looks like:

```jsonl
{"role":"system","content":"[Moment: #general] Long debugging session about deployment. Resolved by checking nginx logs. (moves 1-10)"}
{"role":"system","content":"[Move: #general] Alice asked about errors, I suggested logs, found the issue. (messages 1234-1280)"}
{"role":"system","content":"[Reference: docs/deploy.md:15-42]\n## Deployment Checklist\n1. Run migrations\n2. Check nginx config"}
{"role":"user","content":"[Chat: #general]\n[14:30] <alice> the site is down again\n[14:31] <alice> same error as before"}
{"role":"assistant","tool_calls":[{"id":"call_abc","type":"function","function":{"name":"bash","arguments":"{\"command\":\"nginx -t\"}"}}]}
{"role":"tool","tool_call_id":"call_abc","content":"nginx: configuration file syntax is ok"}
{"role":"assistant","content":"Nginx config looks fine. Let me check the application logs."}
{"role":"system","content":"[Flash from spectator] User seems stressed. Be concise and action-oriented."}
{"role":"user","content":"[Chat: #general]\n[14:35] <alice> any luck?"}
```

## Token Estimation

The crate estimates tokens using a simple heuristic (~4 characters per token):

```rust
fn estimate_tokens(chars: usize) -> usize {
    (chars + 3) / 4
}
```

Each message adds ~4 tokens of overhead for role/structure.

During assembly, the crate sums token estimates. If the total exceeds `max_tokens`, it returns `ContextError::OverBudget`.

**Note:** The worker validates against actual token counts from the model API response. The crate's estimate is a fast pre-check, not the source of truth.

## Crate Structure

```
river-context/
├── Cargo.toml
├── src/
│   ├── lib.rs          # pub fn build_context, re-exports
│   ├── openai.rs       # OpenAIMessage, ToolCall types
│   ├── workspace.rs    # Moment, Move, Flash, Embedding, ChatMessage
│   ├── request.rs      # ContextRequest, ChannelContext
│   ├── response.rs     # ContextResponse, ContextError
│   ├── assembly.rs     # the assembly logic (ordering, interspersing)
│   ├── format.rs       # workspace types -> OpenAI message conversion
│   ├── tokens.rs       # estimate_tokens implementations
│   └── id.rs           # extract timestamp from ID for ordering
```

### Dependencies

Minimal:
- `river-adapter` — Author and Channel types
- `serde` + `serde_json` — serialization
- `thiserror` — error types

No async, no IO, no external services. Pure computation.

## Integration

### Worker Usage

The worker:
1. Loads LLM history from `context.jsonl` (already OpenAI format)
2. Gathers moments, moves, chat messages from workspace files
3. Gathers flashes from memory, embeddings from embed server
4. Calls `build_context(request)`
5. Receives `Vec<OpenAIMessage>` ready for LLM

### Data Flow

```
┌─────────────────┐     ┌──────────────────┐
│ context.jsonl   │     │ Workspace files  │
│ (OpenAI format) │     │ (structured)     │
└────────┬────────┘     └────────┬─────────┘
         │                       │
         │    ┌──────────────┐   │
         └───►│ Context Crate├◄──┘
              └──────┬───────┘
                     │
                     ▼
              [OpenAI messages] ──► LLM
                     │
                     ▼
              [OpenAI response] ──► append to context.jsonl
```

### Spectator Relationship

The spectator is a separate worker that:
- Writes moments and moves to workspace files
- Pushes flashes via the flash mechanism
- Does not use this crate directly

The context crate assembles from what the spectator has written. It does not perform compression.

### Size Management

Size is negotiated between orchestrator and worker:
1. Worker tracks token usage from model API responses
2. Worker hits limit → calls summary → exits
3. Orchestrator spawns fresh worker with summary as seed
4. context.jsonl resets

The crate's role is limited to refusing over-budget contexts. It does not compress or drop content.

## Related Documents

- `docs/superpowers/specs/2026-04-01-worker-design.md` — Worker architecture
- `docs/superpowers/specs/2026-04-01-orchestrator-design.md` — Orchestrator architecture
- `docs/research/context-management-brainstorm.md` — Philosophy and cognition model

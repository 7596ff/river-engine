# River TUI — Design Spec

> Terminal UI for debugging and testing River Engine
>
> Authors: Cass, Claude
> Date: 2026-04-03

## Overview

`river-tui` is a terminal-based tool that provides a TUI for interacting with River Engine workers. It acts as a mock adapter during development, allowing direct observation of the worker's context.

## Goals

1. **Context visibility:** See the worker's context.jsonl in real-time
2. **Interactive testing:** Send messages as a simulated user
3. **Development workflow:** No external services required (except orchestrator)

## Architecture

The TUI is a mock adapter that:
1. Registers with the orchestrator like any adapter
2. Receives worker binding via `/start`
3. Handles outbound requests via `/execute`
4. Sends user messages to worker via `/notify`
5. Tails both sides' `context.jsonl` files for display

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│                 │     │                 │     │                 │
│  TUI Thread     │◄───►│  Adapter State  │◄───►│  HTTP Server    │
│  (ratatui)      │     │                 │     │  (axum)         │
│                 │     │                 │     │                 │
└─────────────────┘     └─────────────────┘     └─────────────────┘
        │                       │                       │
        ▼                       ▼                       ▼
   User Input              Context Tail            Worker HTTP
   Key Events         left/context.jsonl          /start, /execute
                     right/context.jsonl

        │
        ▼
   POST /notify
   (to worker)
```

## TUI Layout

Single-pane layout with header, messages, and input. Shows both sides of the dyad interleaved:

```
┌─────────────────────────────────────────────────────────────────────────┐
│ River Mock Adapter  dyad:river  channel:general  [connected]  L:21 R:19 │
├─────────────────────────────────────────────────────────────────────────┤
│ Context                                                                 │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│ [14:32:01] L sys> You are the Actor in a dyad...                       │
│ [14:32:01] R sys> You are the Spectator in a dyad...                   │
│                                                                         │
│ [14:32:01] L user> hey, can you check the logs?                        │
│                                                                         │
│ [14:32:02] L asst> call read {"path": "/var/log/app.log"}              │
│ [14:32:02] L tool> [abc123] Connection refused on line 47...           │
│ [14:32:05] L asst> I found an error on line 47                         │
│                                                                         │
│ [14:32:06] R asst> I see my partner found an error in the logs         │
│                                                                         │
│ [14:32:10] you> can you fix it?                                        │
│                                                                         │
├─────────────────────────────────────────────────────────────────────────┤
│ Type message (Enter to send, Ctrl+C to quit)                           │
├─────────────────────────────────────────────────────────────────────────┤
│ > _                                                                     │
└─────────────────────────────────────────────────────────────────────────┘
```

### Panels

1. **Header:** Adapter info, dyad, channel, connection status, line counts per side (L:n R:n)
2. **Messages:** Scrollable view of context entries from both sides, interleaved by arrival time
3. **Input:** Single-line input with cursor

### Message Types

| Prefix | Source | Color |
|--------|--------|-------|
| `you>` | Local user input (before sent to worker) | Cyan |
| `sys>` | System messages (connection status, errors) | Yellow |
| `user>` | User messages from context.jsonl | Cyan |
| `asst>` | Assistant messages from context.jsonl | Green |
| `sys>` | System messages from context.jsonl | Magenta |
| `tool>` | Tool results from context.jsonl | Blue |

### Tool Call Display

Tool calls show function name and truncated arguments:
```
[14:32:02] asst> call read {"path": "/var/log/app.log", "lines": 50}
```

Tool results show call ID prefix and truncated content:
```
[14:32:02] tool> [abc123] Connection refused on line 47...
```

## HTTP API

Standard adapter endpoints:

### POST /start
Bind to worker.
```json
{ "worker_endpoint": "http://localhost:52341" }
```

### POST /execute
Receive outbound requests from worker.
```json
{
  "SendMessage": {
    "channel": "general",
    "content": "I found an error...",
    "reply_to": null
  }
}
```

Returns message ID for tracking.

### GET /health
```json
{ "status": "ok" }
```

## Key Bindings

| Key | Action |
|-----|--------|
| Enter | Send message |
| Ctrl+C | Quit |
| Up / Down | Scroll conversation |
| Backspace | Delete character |
| Any char | Append to input |

## Context Tailing

The TUI tails both `left/context.jsonl` and `right/context.jsonl` to show the full dyad conversation:

1. Spawn two tail tasks, one per side
2. Poll each file every 100ms for new lines
3. Parse each line as `OpenAIMessage`
4. Display with side indicator (L/R) and role-appropriate formatting
5. Track lines read per side to avoid re-processing
6. Messages appear interleaved by arrival time

Context entries show:
- **side**: L (left) or R (right)
- **role**: user, assistant, system, tool
- **content**: Message text (truncated for tool calls/results)
- **tool_calls**: Function name and arguments
- **tool_call_id**: For tool results, shows ID prefix

## Event Generation

When user sends a message:

```rust
InboundEvent {
    adapter: "mock".into(),
    metadata: EventMetadata::MessageCreate {
        channel: current_channel.clone(),
        author: Author {
            id: "user-1".into(),
            name: "Human".into(),
            bot: false,
        },
        content: user_input.clone(),
        message_id: generator.next(SnowflakeType::Message).to_string(),
        timestamp: Utc::now().to_rfc3339(),
        reply_to: None,
        attachments: vec![],
    },
}
```

Sent via HTTP POST to worker's `/notify` endpoint.

## CLI

```
river-tui --orchestrator <URL> --dyad <NAME> [OPTIONS]

Options:
  --orchestrator <URL>    Orchestrator endpoint for registration
  --dyad <NAME>           Dyad this adapter serves
  --adapter-type <TYPE>   Adapter type (default: mock)
  --channel <CHANNEL>     Default channel name (default: general)
  --port <PORT>           Port to bind (default: 0 for OS-assigned)
  --workspace <PATH>      Workspace directory (tails both left/context.jsonl and right/context.jsonl)
```

## Features Reported

```rust
vec![
    FeatureId::SendMessage,
    FeatureId::ReceiveMessage,
    FeatureId::EditMessage,
    FeatureId::DeleteMessage,
    FeatureId::ReadHistory,
    FeatureId::AddReaction,
    FeatureId::TypingIndicator,
]
```

## Dependencies

```toml
[dependencies]
river-adapter = { path = "../river-adapter" }
river-context = { path = "../river-context" }
river-snowflake = { path = "../river-snowflake" }

tokio = { workspace = true }
axum = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
clap = { workspace = true }
chrono = { workspace = true }
ratatui = { workspace = true }
crossterm = { workspace = true }
```

## Module Structure

```
river-tui/
├── Cargo.toml
├── src/
│   ├── main.rs      # CLI, startup, orchestrator registration
│   ├── adapter.rs   # Adapter state, message types
│   ├── http.rs      # HTTP server (/start, /execute, /health)
│   └── tui.rs       # Terminal UI, context tailing
```

## State

```rust
pub struct AdapterState {
    // Identity
    pub dyad: String,
    pub adapter_type: String,
    pub channel: String,

    // Worker binding
    pub worker_endpoint: Option<String>,

    // Messages display
    pub messages: Vec<DisplayMessage>,
    pub conversation_scroll: usize,

    // Context tailing (per side)
    pub left_lines_read: usize,
    pub right_lines_read: usize,

    // Input
    pub input: String,

    // Snowflake generation
    pub generator: SnowflakeGenerator,
}

pub enum DisplayMessage {
    /// User input from TUI (not yet in context)
    User { id: String, content: String, timestamp: DateTime<Utc> },
    /// System status messages
    System { content: String, timestamp: DateTime<Utc> },
    /// Context entry from worker's context.jsonl
    Context { side: String, entry: OpenAIMessage, timestamp: DateTime<Utc> },
}
```

## Example Session

```
$ cargo run --package river-tui -- \
    --orchestrator http://localhost:4000 \
    --dyad river \
    --channel general \
    --workspace /var/lib/river/river

┌─────────────────────────────────────────────────────────────────────────┐
│ River Mock Adapter  dyad:river  channel:general  [connected]  L:3 R:2   │
├─────────────────────────────────────────────────────────────────────────┤
│ Context                                                                 │
├─────────────────────────────────────────────────────────────────────────┤
│ [14:30:00] [sys] Connected to orchestrator                             │
│ [14:30:00] [sys] Worker at http://localhost:52341                      │
│ [14:30:01] L sys> You are the Actor in a dyad...                       │
│ [14:30:01] R sys> You are the Spectator in a dyad...                   │
│ [14:32:01] you> hello                                                  │
│ [14:32:03] L asst> Hello! How can I help you today?                    │
│ [14:32:04] R asst> I see my partner greeted the user...                │
├─────────────────────────────────────────────────────────────────────────┤
│ Type message (Enter to send, Ctrl+C to quit)                           │
├─────────────────────────────────────────────────────────────────────────┤
│ > _                                                                     │
└─────────────────────────────────────────────────────────────────────────┘
```

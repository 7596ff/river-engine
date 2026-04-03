# River CLI — Design Spec

> Terminal UI for debugging and testing River Engine
>
> Authors: Cass, Claude
> Date: 2026-04-03

## Overview

`river-cli` is a terminal-based tool that provides a TUI for interacting with River Engine workers. It replaces Discord/Slack during development, allowing direct observation of the think→act loop.

## Goals

1. **Debugging visibility:** See all messages, tool calls, and responses in real-time
2. **Interactive testing:** Send messages as a simulated user
3. **Development workflow:** No external services required
4. **Traffic logging:** Record all adapter traffic for replay/analysis

## TUI Layout

Horizontal split with conversation on top, debug/flash queue on bottom:

```
┌─────────────────────────────────────────────────────────────────────────┐
│ River Mock Adapter — dyad:river channel:general [Actor: left]           │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│ [14:32:01] user> hey, can you check the logs?                          │
│                                                                         │
│ [14:32:05] worker> I checked the logs. There's an error on line 47:    │
│            "Connection refused to database"                             │
│                                                                         │
│ [14:32:10] user> can you fix it?                                       │
│                                                                         │
├─────────────────────────────────────────────────────────────────────────┤
│ > Type a message... (Enter twice to send)                               │
│   _                                                                     │
├─────────────────────────────────────────────────────────────────────────┤
│ Debug [THINKING...]                                                     │
├─────────────────────────────────────────────────────────────────────────┤
│ ──── TOOL: read ────                 │ ──── FLASH from river:right ──── │
│ path: /var/log/app.log               │ Check the database connection    │
│ lines: 50                            │ string in .env                   │
│ Result: [50 lines...]                │ TTL: 45s                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Panels (top to bottom)

1. **Header:** Adapter info, dyad, channel, which side is Actor
2. **Conversation:** Scrollable message history with timestamps
3. **Input:** Multi-line scrollable input (Enter twice or Ctrl+Enter to send)
4. **Debug header:** Shows THINKING indicator when waiting
5. **Debug split:** Tool calls on left, flash queue on right

### Color Scheme

- `user>` — cyan
- `worker>` — green
- Tool calls — dim/gray with magenta headers
- Errors — red
- System messages — yellow
- Flashes — magenta background
- THINKING — yellow, blinking

## Architecture

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│                 │     │                 │     │                 │
│  TUI Thread     │◄───►│  Adapter Core   │◄───►│  HTTP Server    │
│  (ratatui)      │     │  (state)        │     │  (axum)         │
│                 │     │                 │     │                 │
└─────────────────┘     └─────────────────┘     └─────────────────┘
        │                       │                       │
        │                       │                       │
        ▼                       ▼                       ▼
   User Input            Event Queue              Worker HTTP
   Key Events            Message Log              /notify calls
                         Tool Traces              /execute calls
                         Flash Queue              /debug calls (NEW)

                         ▼
                    Traffic Log
                    (JSONL file)
```

### Components

**TUI Thread:**
- Handles keyboard input
- Renders split-pane UI
- Receives updates via channel
- Shows THINKING state

**Adapter Core:**
- Maintains conversation state
- Tracks current channel
- Manages flash queue with TTL
- Logs all traffic to file
- Generates snowflake IDs for messages

**HTTP Server:**
- `POST /start` — bind to worker
- `POST /execute` — receive outbound requests (speak, etc.)
- `POST /debug` — receive tool call traces (NEW)
- `POST /flash` — receive flash messages (NEW)
- `GET /health` — health check

## Worker Debug Taps

The worker needs to forward tool call information to the adapter for display. Add a debug endpoint that the worker calls:

```rust
// In worker tools.rs, after executing each tool:
if let Some(adapter_endpoint) = state.read().await.adapter_debug_endpoint() {
    let _ = client.post(format!("{}/debug", adapter_endpoint))
        .json(&DebugEvent::ToolCall {
            tool: tool_name.clone(),
            args: args.clone(),
            result: result.clone(),
            error: error.clone(),
            timestamp: Utc::now().to_rfc3339(),
        })
        .send()
        .await;
}
```

### Debug Event Types

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DebugEvent {
    ToolCall {
        tool: String,
        args: serde_json::Value,
        result: Option<serde_json::Value>,
        error: Option<String>,
        timestamp: String,
    },
    Thinking {
        started: bool,  // true = started thinking, false = done
        timestamp: String,
    },
    LlmRequest {
        model: String,
        token_count: usize,
        timestamp: String,
    },
    LlmResponse {
        token_count: usize,
        has_tool_calls: bool,
        timestamp: String,
    },
}
```

## HTTP API

Same as real adapters per `docs/superpowers/specs/2026-04-01-adapter-library-design.md`, plus debug endpoints:

### POST /start
```json
{ "worker_endpoint": "http://localhost:52341" }
```

### POST /execute
Receives `OutboundRequest` from worker:
```json
{
  "SendMessage": {
    "channel": "general",
    "content": "I checked the logs...",
    "reply_to": null
  }
}
```

### POST /debug (NEW)
Receives debug events from worker:
```json
{
  "type": "ToolCall",
  "tool": "read",
  "args": {"path": "/var/log/app.log"},
  "result": {"content": "..."},
  "error": null,
  "timestamp": "2026-04-03T14:32:02Z"
}
```

### POST /flash (NEW)
Receives flash messages:
```json
{
  "from": "river:right",
  "content": "Check the database connection",
  "expires_at": "2026-04-03T14:33:00Z"
}
```

### GET /health
```json
{ "status": "ok" }
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

## User Input Handling

| Key | Action |
|-----|--------|
| Enter (twice) | Send message |
| Ctrl+Enter | Send message (alternative) |
| Ctrl+C (twice) | Quit (safety) |
| PgUp / PgDn | Scroll conversation |
| Up / Down | Navigate input history |
| Ctrl+D | Toggle debug panel |
| Tab | Switch focus (conversation ↔ input) |
| Esc | Clear input |

## Event Generation

When user types a message and sends:

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

## THINKING Indicator

When the adapter sends a message to the worker:
1. Set `thinking = true`
2. Display `[THINKING...]` in debug panel (yellow, blinking)
3. When worker sends `/execute` with `SendMessage`, set `thinking = false`

## Flash Queue Display

Flashes shown in debug panel with:
- Source (dyad:side)
- Content
- TTL countdown (updates every second)
- Auto-remove when expired

```
──── FLASH from river:right ────
Check the database connection
string in .env
TTL: 45s remaining
```

## Traffic Logging

All traffic logged to `mock-adapter.log` (or `--log` path) in JSONL format:

```jsonl
{"ts":"2026-04-03T14:32:01Z","type":"user_input","content":"hey, can you check the logs?"}
{"ts":"2026-04-03T14:32:01Z","type":"notify_sent","event":{...}}
{"ts":"2026-04-03T14:32:02Z","type":"debug_received","event":{"type":"Thinking","started":true}}
{"ts":"2026-04-03T14:32:02Z","type":"debug_received","event":{"type":"ToolCall","tool":"read",...}}
{"ts":"2026-04-03T14:32:05Z","type":"execute_received","request":{...}}
{"ts":"2026-04-03T14:32:05Z","type":"execute_response","response":{...}}
```

### Log Module

```rust
// log.rs
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

pub struct TrafficLog {
    file: File,
}

impl TrafficLog {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { file })
    }

    pub fn log(&mut self, event: &LogEvent) {
        if let Ok(json) = serde_json::to_string(event) {
            let _ = writeln!(self.file, "{}", json);
        }
    }
}

#[derive(Serialize)]
pub struct LogEvent {
    pub ts: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(flatten)]
    pub data: serde_json::Value,
}
```

## CLI

```
river-cli --orchestrator <URL> --dyad <NAME> [OPTIONS]

Options:
  --orchestrator <URL>    Orchestrator endpoint for registration
  --dyad <NAME>           Dyad this adapter serves
  --type <TYPE>           Adapter type (default: mock)
  --channel <CHANNEL>     Default channel name (default: general)
  --port <PORT>           Port to bind (default: 0 for OS-assigned)
  --log <PATH>            Log file path (default: mock-adapter.log)
  --no-debug              Start with debug panel hidden
```

## Dependencies

```toml
[dependencies]
river-adapter = { path = "../river-adapter" }
river-snowflake = { path = "../river-snowflake" }

tokio = { workspace = true }
axum = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
clap = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
chrono = { workspace = true }

# TUI
ratatui = "0.29"
crossterm = "0.28"
```

## Module Structure

```
river-cli/
├── Cargo.toml
├── src/
│   ├── main.rs      # CLI, startup, tokio runtime
│   ├── adapter.rs   # Core adapter state
│   ├── http.rs      # HTTP server (incl /debug, /flash)
│   ├── tui.rs       # Terminal UI (split pane)
│   └── log.rs       # Traffic logging
```

## State Management

```rust
pub struct AdapterState {
    // Identity
    pub dyad: String,
    pub adapter_type: String,
    pub channel: String,

    // Worker binding
    pub worker_endpoint: Option<String>,

    // Conversation (left pane)
    pub messages: Vec<DisplayMessage>,
    pub conversation_scroll: usize,

    // Debug info (right pane)
    pub tool_traces: Vec<ToolTrace>,
    pub flashes: Vec<ActiveFlash>,
    pub thinking: bool,
    pub debug_scroll: usize,
    pub show_debug: bool,

    // Input (multi-line)
    pub input: String,
    pub input_scroll: usize,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,

    // Quit safety
    pub quit_pressed: bool,  // true after first Ctrl+C

    // Snowflake generation
    pub generator: SnowflakeGenerator,

    // Logging
    pub log: Option<TrafficLog>,
}

pub enum DisplayMessage {
    User { id: String, content: String, timestamp: DateTime<Utc> },
    Worker { id: String, content: String, timestamp: DateTime<Utc> },
    System { content: String, timestamp: DateTime<Utc> },
}

pub struct ToolTrace {
    pub tool: String,
    pub args: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub timestamp: DateTime<Utc>,
}

pub struct ActiveFlash {
    pub from: String,
    pub content: String,
    pub expires_at: DateTime<Utc>,
}
```

## Concurrency Model

```rust
#[tokio::main]
async fn main() {
    // Shared state
    let state = Arc::new(RwLock::new(AdapterState::new()));

    // Channel for TUI updates
    let (ui_tx, ui_rx) = mpsc::channel::<UiEvent>(256);

    // HTTP server task
    let http_state = state.clone();
    let http_tx = ui_tx.clone();
    tokio::spawn(async move {
        run_http_server(http_state, http_tx).await
    });

    // Flash TTL cleanup task
    let flash_state = state.clone();
    let flash_tx = ui_tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let mut s = flash_state.write().await;
            let now = Utc::now();
            s.flashes.retain(|f| f.expires_at > now);
            let _ = flash_tx.send(UiEvent::Refresh).await;
        }
    });

    // TUI runs on main thread (blocking)
    run_tui(state, ui_rx).await;
}
```

## Example Session

```
$ cargo run --package river-cli -- \
    --orchestrator http://localhost:4000 \
    --dyad river \
    --channel general

┌──────────────────────────────────────┬──────────────────────────────────┐
│ River Mock Adapter — dyad:river      │ Debug / Flashes                  │
│ channel:general [connected]          │                                  │
├──────────────────────────────────────┼──────────────────────────────────┤
│                                      │                                  │
│ [System] Connected to orchestrator   │                                  │
│ [System] Worker at localhost:52341   │                                  │
│                                      │                                  │
│ [14:32:01] user> hello               │ [THINKING...]                    │
│                                      │                                  │
│                                      │ ──── TOOL: speak ────            │
│ [14:32:03] worker> Hello! How can I  │ channel: general                 │
│            help you today?           │ content: Hello! How can I...     │
│                                      │                                  │
├──────────────────────────────────────┼──────────────────────────────────┤
│ > hello again                        │                                  │
│   _                                  │                                  │
└──────────────────────────────────────┴──────────────────────────────────┘
```

## Worker Changes Required

To support debug taps, the worker needs:

1. **Config**: Store adapter debug endpoint (same as adapter endpoint + `/debug`)
2. **Tool execution**: After each tool call, POST to `/debug` with ToolCall event
3. **LLM calls**: Before/after LLM request, POST Thinking events
4. **Optional**: POST LlmRequest/LlmResponse for token tracking

This is opt-in - if no debug endpoint, worker skips these calls.

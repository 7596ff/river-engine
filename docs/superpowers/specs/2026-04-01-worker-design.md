# Worker — Design Spec

> river-worker: Binary that runs the worker loop
>
> Authors: Cass, Claude
> Date: 2026-04-01

## Overview

The worker is a binary that runs the think→act loop — calling the LLM, executing tools, handling notifications, and managing context.

**Philosophy:** The worker is a shell. All intelligence lives in the model. The worker provides primitives (tools, context, communication) and lets the model decide what to do.

**Key characteristics:**
- Binary only (no library exports)
- Direct workspace access (no orchestrator file routing)
- Git via bash for file coordination between workers
- Uses `river-adapter` for inbound/outbound types
- Uses `river-context` for context assembly
- Flash peer-to-peer via registry lookup
- Model config (including API key) from orchestrator

## Crate Structure

```
river-worker/
├── Cargo.toml
└── src/
    ├── main.rs          # CLI parsing, startup sequence
    ├── config.rs        # WorkerConfig from CLI args
    ├── state.rs         # WorkerState (current channel, watch list, registry)
    ├── loop.rs          # main think→act loop
    ├── tools.rs         # tool implementations
    ├── http.rs          # axum server (/notify, /flash, /registry, /health)
    ├── llm.rs           # LLM client (OpenAI-compatible chat completions)
    └── persistence.rs   # JSONL context read/write
```

## Dependencies

```toml
[package]
name = "river-worker"
version = "0.1.0"
edition = "2021"

[dependencies]
river-adapter = { path = "../river-adapter" }
river-context = { path = "../river-context" }
river-snowflake = { path = "../river-snowflake" }
tokio = { workspace = true }
axum = { workspace = true }
reqwest = { workspace = true }
clap = { version = "4.0", features = ["derive"] }
serde = { workspace = true }
serde_json = { workspace = true }
```

## CLI

```
river-worker [OPTIONS]

Options:
  --orchestrator <URL>      Orchestrator endpoint
  --dyad <NAME>             Dyad name (e.g., "river")
  --side <SIDE>             Worker side: "left" or "right"
  --port <PORT>             Port to bind (default: 0 for OS-assigned)
  -h, --help                Print help
```

## Configuration

```rust
/// Built from CLI args
pub struct WorkerConfig {
    pub orchestrator_endpoint: String,
    pub dyad: String,
    pub side: Side,
    pub port: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Received from orchestrator on registration
pub struct RegistrationInfo {
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub model: ModelConfig,
    pub ground: Ground,
    pub workspace: PathBuf,
    pub initial_message: Option<String>,
    pub start_sleeping: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Baton {
    Actor,
    Spectator,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Ground {
    pub name: String,
    pub id: String,
    pub adapter: String,
    pub channel: String,
}

/// Received from orchestrator on registration and model switch
pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,
    pub context_limit: usize,
}
```

## Startup Sequence

1. Parse CLI args into `WorkerConfig`
2. Bind HTTP server to port (0 = OS-assigned)
3. Register with orchestrator (`POST /register`) → receive `RegistrationInfo`
4. Initialize `WorkerState` (current channel = ground, model config, role)
5. Load existing context from `workspace/{side}/context.jsonl` if exists
6. Load role definition from `workspace/roles/{role}.md`
7. If actor: wait for first `/notify` to start loop
8. If spectator: wait for first `/flash` to start loop

**Registration request:**
```json
{
  "endpoint": "http://localhost:52341",
  "worker": {
    "dyad": "river",
    "side": "left"
  }
}
```

**Registration response:**
```json
{
  "accepted": true,
  "baton": "actor",
  "partner_endpoint": "http://localhost:52342",
  "model": {
    "endpoint": "https://api.anthropic.com/v1",
    "name": "claude-sonnet-4-20250514",
    "api_key": "sk-...",
    "context_limit": 200000
  },
  "ground": {
    "name": "alice",
    "id": "123456",
    "adapter": "discord",
    "channel": "dm-alice-123"
  },
  "workspace": "/home/user/workspace/river",
  "initial_message": null,
  "start_sleeping": false
}
```

- `baton` — "actor" or "spectator" (initial baton based on dyad config)
- `partner_endpoint` — endpoint of paired worker (null if not yet registered)
- `ground` — the human operator contact info
- `initial_message` — summary from previous session (for `ContextExhausted` or timed `Done` respawn)
- `start_sleeping` — true when respawning after `Done { wake_after_minutes: None }`, worker should call `sleep(None)` immediately

## Worker State

```rust
// Imported from other crates
use river_adapter::{Author, Channel};
use river_context::Flash;

pub struct WorkerState {
    // Identity (from config + registration)
    pub dyad: String,
    pub side: Side,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub ground: Ground,
    pub workspace: PathBuf,

    // Communication
    pub current_channel: Channel,
    pub watch_list: HashSet<Channel>,
    pub registry: Registry,

    // Model
    pub model_config: ModelConfig,
    pub token_count: usize,
    pub context_limit: usize,

    // Loop control
    pub sleeping: bool,
    pub sleep_until: Option<Instant>,
    pub pending_notifications: Vec<Notification>,
    pub pending_flashes: Vec<Flash>,
}

pub struct Notification {
    pub channel: Channel,
    pub count: usize,
    pub since_id: Option<String>,  // read events after this ID
}
```

**Initial state (from config + registration):**
- `dyad`, `side` = from CLI args
- `role`, `partner_endpoint`, `ground`, `workspace` = from orchestrator
- `current_channel` = ground channel
- `watch_list` = empty
- `model_config` = from orchestrator
- `sleeping` = `start_sleeping` from registration
- `pending_*` = empty

## The Main Loop

```rust
pub async fn run_loop(state: &mut WorkerState, config: &WorkerConfig) -> WorkerOutput {
    // Wait for first notification
    wait_for_first_notify(state).await;

    loop {
        // Build context from workspace via river-context
        let context = build_context_from_workspace(config, state)?;

        // Check context pressure
        if state.token_count > state.context_limit * 95 / 100 {
            return force_summary(state, config).await;
        }
        if state.token_count > state.context_limit * 80 / 100 {
            inject_context_warning(state);
        }

        // Inject pending flashes (high priority)
        inject_flashes(state, &mut context);

        // Call LLM
        let response = call_llm(state, &context).await?;
        state.token_count = response.usage.total_tokens;

        // Handle response
        match response.content {
            LlmContent::ToolCalls(calls) => {
                // Execute all tool calls in parallel
                let results = execute_tools_parallel(calls, state, config).await;

                // Persist after each result
                for result in results {
                    append_to_context(config, &result);
                }

                // Check for summary (exits loop)
                if let Some(summary) = find_summary(&results) {
                    return WorkerOutput::done(summary);
                }

                // Check for sleep
                if let Some(duration) = find_sleep(&results) {
                    sleep_until_wake(state, duration).await;
                }
            }
            LlmContent::Text(text) => {
                // Model responded with text, inject status and continue
                inject_status_message(state);
            }
        }
    }
}
```

**Key behaviors:**
- Flashes injected before each LLM call
- All tool calls in single response executed in parallel
- Context persisted after each tool result
- `summary` in any tool batch exits the loop
- `sleep` pauses loop, watched notifications wake early
- Text response → status message → continue (not an exit)

## Tools

## Tools

### File Tools

```rust
/// Read file contents from workspace
pub struct ReadTool {
    pub path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}
// Returns file contents as string
// Error if file doesn't exist

/// Write file to workspace
pub struct WriteTool {
    pub path: String,
    pub content: String,
    pub mode: WriteMode,
    pub at_line: Option<usize>,  // required for Insert
}

pub enum WriteMode {
    Overwrite,
    Append,
    Insert,
}
// Creates parent directories if needed
// If path starts with embeddings/, notifies embed server POST /index
```

### Bash

```rust
/// Execute shell command
pub struct BashTool {
    pub command: String,
    pub timeout_seconds: Option<u64>,  // default 120, max 600
    pub working_directory: Option<String>,
}
// Returns { stdout, stderr, exit_code }
// Git operations happen here
```

### Communication Tools

```rust
/// Send message to a channel
pub struct SpeakTool {
    pub content: String,
    pub adapter: Option<String>,     // defaults to current_channel.adapter
    pub channel: Option<String>,     // defaults to current_channel.id
    pub reply_to: Option<String>,
}
// If adapter/channel omitted, uses current_channel
// Does NOT change current_channel

/// Execute any adapter operation
pub struct AdapterTool {
    pub adapter: String,
    pub request: OutboundRequest,
}
// For edit, delete, react, etc. — full OutboundRequest control

/// Change current channel
pub struct SwitchChannelTool {
    pub adapter: String,
    pub channel: String,
}
```

### Control Tools

```rust
/// Pause loop
pub struct SleepTool {
    pub minutes: Option<u64>,  // None = indefinite
}

/// Manage wake channels
pub struct WatchTool {
    pub add: Option<Vec<Channel>>,
    pub remove: Option<Vec<Channel>>,
}

/// Exit loop with summary
pub struct SummaryTool {
    pub summary: String,
}

/// Create a move (summarizes a range of messages)
pub struct CreateMoveTool {
    pub channel: Channel,             // which channel's messages
    pub content: String,                 // the move summary
    pub start_message_id: String,        // first message in range (platform ID)
    pub end_message_id: String,          // last message in range (platform ID)
}
// Worker generates snowflake ID (SnowflakeType::Move)
// Writes Move to workspace/moves/{channel_id}.jsonl

/// Create a moment (summarizes a range of moves)
pub struct CreateMomentTool {
    pub channel: Channel,             // which channel's moves
    pub content: String,                 // the moment summary
    pub start_move_id: String,           // first move in range (snowflake ID)
    pub end_move_id: String,             // last move in range (snowflake ID)
}
// Worker generates snowflake ID (SnowflakeType::Moment)
// Writes Moment to workspace/moments/{channel_id}.jsonl

/// Send to another worker (peer-to-peer)
pub struct CreateFlashTool {
    pub target: String,         // target worker name
    pub content: String,        // message content
    pub ttl_minutes: Option<u32>, // time-to-live, default 60
}
// Worker generates snowflake ID, calculates expires_at from TTL
// Looks up target endpoint from registry, sends Flash directly

/// Switch LLM model
pub struct RequestModelTool {
    pub model: String,
}
// Calls orchestrator POST /model/switch
// Receives new ModelConfig

/// Switch roles with partner worker
pub struct SwitchRolesTool {}
// No parameters — coordinates with partner to swap roles
// Actor becomes spectator, spectator becomes actor
// Both workers reload their role definition file

/// Delete file from workspace
pub struct DeleteTool {
    pub path: String,
}
// Deletes file, if in embeddings/ notifies embed server

/// Search embeddings, returns first result + cursor
pub struct SearchEmbeddingsTool {
    pub query: String,
}
// Calls embed server POST /search
// Returns: { cursor, result: { id, content, source, score }, remaining }

/// Continue embedding search with cursor
pub struct NextEmbeddingTool {
    pub cursor: String,
}
// Calls embed server POST /next
// Returns: { cursor, result | null, remaining }
```

## HTTP API

### POST /notify

Receive events from adapters.

```rust
pub async fn handle_notify(
    State(state): State<WorkerState>,
    Json(event): Json<InboundEvent>,
) -> StatusCode
```

**Behavior:**
1. Parse event (using `river-adapter::InboundEvent`)
2. Append to conversation file (see Conversation File Format section):
   - Guild channels: `workspace/conversations/{adapter}/{guild_id}-{guild_name}/{channel_id}-{channel_name}.txt`
   - DMs: `workspace/conversations/{adapter}/dm/{user_id}-{user_name}.txt`
   - Status: `[ ]` (unread)
3. Batch notification for next status message
4. If sleeping and channel in watch_list: wake

Adapters fire-and-forget. Worker handles persistence.

### POST /flash

Receive flash from another worker.

```rust
pub async fn handle_flash(
    State(state): State<WorkerState>,
    Json(flash): Json<Flash>,  // Flash type from river_context
) -> StatusCode
```

**Behavior:**
- Queue flash for injection before next LLM call
- If sleeping: wake immediately

### POST /registry

Receive registry updates from orchestrator.

```rust
pub async fn handle_registry(
    State(state): State<WorkerState>,
    Json(registry): Json<Registry>,
) -> StatusCode
```

Updates local registry copy. Used for peer-to-peer flash routing.

### POST /switch_roles

Receive role switch request from partner worker.

```rust
pub async fn handle_switch_roles(
    State(state): State<WorkerState>,
) -> Result<Json<SwitchResponse>, StatusCode>
```

**Behavior:**
1. Validate not mid-operation (reject if busy)
2. Swap role: actor ↔ spectator
3. Reload role definition from `workspace/roles/{new_role}.md`
4. Notify orchestrator of role change
5. Return success with new role

**Response:**
```json
{ "accepted": true, "new_role": "actor" }
```

### GET /health

Health check.

```rust
pub async fn handle_health() -> Json<HealthResponse>

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,  // "ok" or "error"
}
```

**Response:**
```json
{ "status": "ok" }
```

## Conversation File Format

Worker is fully responsible for reading and writing conversation files. Uses a line-based text format for human readability and easy diffing.

### File Paths

```
workspace/conversations/{adapter}/{guild_id}-{guild_name}/{channel_id}-{channel_name}.txt
workspace/conversations/{adapter}/dm/{user_id}-{user_name}.txt
```

### Line Format

```
[status] timestamp message_id <author_name:author_id> content
```

**Examples:**
```
[ ] 2026-03-21T14:30:00Z 1234567890 <alice:111> hey, can you help me?
[>] 2026-03-21T14:30:15Z 1234567891 <river:999> Sure! What do you need?
[x] 2026-03-21T14:30:30Z 1234567892 <alice:111> I'm trying to deploy...
[!] 2026-03-21T14:31:00Z - <river:999> Failed to send: rate limited
```

### Status Markers

| Marker | Meaning |
|--------|---------|
| `[ ]` | Incoming, unread |
| `[x]` | Incoming, read |
| `[>]` | Outgoing (sent by worker) |
| `[!]` | Failed to send (message_id is `-`) |

### Types

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    Unread,    // [ ]
    Read,      // [x]
    Outgoing,  // [>]
    Failed,    // [!]
}

pub struct ConversationLine {
    pub status: MessageStatus,
    pub timestamp: String,       // ISO8601
    pub message_id: String,      // platform message ID, or "-" for failed
    pub author_name: String,
    pub author_id: String,
    pub content: String,
}
```

### Custom Serialization

The line format requires custom `Serialize`/`Deserialize` implementations (not JSON):

```rust
impl ConversationLine {
    /// Parse a line from the conversation file
    pub fn parse(line: &str) -> Result<Self, ParseError>;

    /// Format as a line for the conversation file
    pub fn format(&self) -> String;
}

impl ConversationFile {
    /// Load all lines from a conversation file
    pub fn load(path: &Path) -> Result<Vec<ConversationLine>, IoError>;

    /// Append a single line to a conversation file
    pub fn append(path: &Path, line: &ConversationLine) -> Result<(), IoError>;

    /// Mark a message as read (finds line by message_id, updates status)
    pub fn mark_read(path: &Path, message_id: &str) -> Result<(), IoError>;
}
```

### Parsing Notes

- Lines are newline-delimited (`\n`)
- Content may contain any characters except newline (newlines in content should be escaped as `\n` literal)
- Empty lines are ignored
- Lines starting with `#` are comments (for manual annotations)
- Author name may contain spaces; parsing stops at `:` before author_id
- Timestamp is ISO8601 with timezone (always UTC with `Z` suffix)

## LLM Client

```rust
pub struct LlmClient {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: String,
}

impl LlmClient {
    pub async fn chat(&self, messages: Vec<Message>) -> Result<LlmResponse, LlmError>;
    pub fn update_config(&mut self, config: ModelConfig);
}

pub struct LlmResponse {
    pub content: LlmContent,
    pub usage: Usage,
}

pub enum LlmContent {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

pub struct Usage {
    pub total_tokens: usize,
}
```

**OpenAI-compatible:** Posts to `{endpoint}/chat/completions`. Endpoints must be OpenAI-compatible (OpenAI, Ollama, vLLM, etc.). For other providers (Anthropic, Google), use `river-router` as a translation proxy.

## Context Persistence

Context stored in `workspace/{worker_name}/context.jsonl`.

```rust
pub fn load_context(workspace: &Path, name: &str) -> Vec<Message>;
pub fn append_to_context(workspace: &Path, name: &str, message: &Message);
pub fn save_context(workspace: &Path, name: &str, messages: &[Message]);
```

**Persistence timing:**
- After each tool result
- After model response

Enables crash recovery — worker loads existing context on restart.

## Error Handling

**Malformed tool calls:**
- Retry with backoff: 1 minute, 2 minutes, 5 minutes
- Inject system message explaining error on each retry
- After 3 failures: exit with `Error` status

**Tool execution fails:**
- Return error to model as tool result
- Model decides how to handle
- Worker does not crash

**Adapter unreachable:**
- Return error to model as tool result
- Worker does not crash

**LLM unreachable:**
- Exit with `Error` status

## Output

```rust
pub struct WorkerOutput {
    pub status: ExitStatus,
    pub summary: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ExitStatus {
    Done { wake_after_minutes: Option<u64> },  // None = wait for notifications
    ContextExhausted,
    Error { message: String },
}
```

**Exit conditions:**

| Condition | Status | Behavior |
|-----------|--------|----------|
| `summary` tool called | `Done` | Model's summary used |
| Context at 95% | `ContextExhausted` | Forced summary via final LLM call |
| LLM unreachable | `Error` | Error message in output |
| 3 malformed responses | `Error` | Error message in output |

**Orchestrator notification:**
```
POST /worker/output
{
  "worker_name": "actor",
  "output": { ... WorkerOutput ... }
}
```

## Context Pressure

Worker tracks token count from LLM response `usage.total_tokens`.

**At 80%:**
- Inject system message: "Context at 80%. Consider wrapping up."

**At 95%:**
- Stop loop
- Send final LLM call: "Summarize what you've accomplished and what remains."
- Use response as summary
- Exit with `ContextExhausted`

## File Coordination

Workers sharing a workspace coordinate via git:

1. Worker pulls before reading/writing shared files
2. Worker makes changes
3. Worker commits and pushes
4. If push fails (conflict), worker pulls, resolves, retries

Git operations happen via the `bash` tool. The model is responsible for coordination and conflict resolution.

## Related Documents

- `docs/WORKER-DESIGN.md` — High-level worker design
- `docs/ORCHESTRATOR-DESIGN.md` — Orchestrator architecture
- `docs/superpowers/specs/2026-04-01-adapter-library-design.md` — Adapter types
- `docs/superpowers/specs/2026-04-01-context-management-design.md` — Context assembly
- `docs/superpowers/specs/2026-04-02-embedding-design.md` — Embedding service
- `docs/research/workspace-structure.md` — Workspace layout (draft)

## Appendix: Tool Specifications

Detailed specifications for all 17 tools.

---

### read

Read file contents from workspace.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| path | string | yes | Relative path from workspace root |
| start_line | integer | no | First line to read (1-indexed) |
| end_line | integer | no | Last line to read (inclusive) |

**Returns:**
```json
{ "content": "file contents...", "lines": 42 }
```

**Errors:**
- `file_not_found` — Path does not exist
- `is_directory` — Path is a directory, not a file
- `permission_denied` — Cannot read file

---

### write

Write file to workspace.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| path | string | yes | Relative path from workspace root |
| content | string | yes | Content to write |
| mode | string | no | `overwrite` (default), `append`, or `insert` |
| at_line | integer | no | Line number for insert mode (required if mode=insert) |

**Returns:**
```json
{ "written": true, "bytes": 1234 }
```

**Side effects:**
- Creates parent directories if needed
- If path starts with `embeddings/`, notifies embed server

**Errors:**
- `permission_denied` — Cannot write to path
- `missing_at_line` — Insert mode requires at_line parameter

---

### delete

Delete file from workspace.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| path | string | yes | Relative path from workspace root |

**Returns:**
```json
{ "deleted": true }
```

**Side effects:**
- If path starts with `embeddings/`, notifies embed server to remove chunks

**Errors:**
- `file_not_found` — Path does not exist
- `is_directory` — Use bash `rm -r` for directories
- `permission_denied` — Cannot delete file

---

### bash

Execute shell command.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| command | string | yes | Shell command to execute |
| timeout_seconds | integer | no | Timeout in seconds (default: 120, max: 600) |
| working_directory | string | no | Working directory (default: workspace root) |

**Returns:**
```json
{ "stdout": "...", "stderr": "...", "exit_code": 0 }
```

**Errors:**
- `timeout` — Command exceeded timeout
- `invalid_directory` — Working directory does not exist

---

### speak

Send message to a channel.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| content | string | yes | Message content |
| adapter | string | no | Adapter name (defaults to current_channel) |
| channel | string | no | Channel ID (defaults to current_channel) |
| reply_to | string | no | Message ID to reply to |

**Returns:**
```json
{ "message_id": "1234567890", "sent": true }
```

**Side effects:**
- Appends to conversation file with `[>]` status
- Does NOT change `current_channel`

**Errors:**
- `adapter_not_found` — No adapter with that name in registry
- `adapter_unreachable` — Cannot reach adapter
- `send_failed` — Adapter returned error

---

### adapter

Execute any adapter operation.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| adapter | string | yes | Adapter name |
| request | object | yes | Full OutboundRequest object |

**Returns:** Varies by operation. See adapter library spec.

**Example:**
```json
{
  "adapter": "discord",
  "request": {
    "EditMessage": {
      "channel": "123",
      "message_id": "456",
      "content": "edited content"
    }
  }
}
```

**Errors:**
- `adapter_not_found` — No adapter with that name
- `unsupported_operation` — Adapter doesn't support this operation
- `operation_failed` — Adapter returned error

---

### switch_channel

Change current channel.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| adapter | string | yes | Adapter name |
| channel | string | yes | Channel ID |

**Returns:**
```json
{ "switched": true, "previous": { "adapter": "discord", "channel": "old_id" } }
```

**Side effects:**
- Updates `current_channel` in worker state

---

### sleep

Pause the worker loop.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| minutes | integer | no | Minutes to sleep. None = indefinite (wake on notification) |

**Returns:**
```json
{ "sleeping": true, "until": "2026-04-02T15:30:00Z" }
```

**Side effects:**
- Sets `sleeping = true` in worker state
- Sets `sleep_until` if minutes provided

**Wake conditions:**
- Flash received from another worker
- Notification on watched channel
- Sleep duration elapsed

---

### watch

Manage watched channels for wake notifications.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| add | array | no | Channels to add: `[{ adapter, id, name? }]` |
| remove | array | no | Channels to remove: `[{ adapter, id }]` |

**Returns:**
```json
{ "watching": [{ "adapter": "discord", "id": "123", "name": "general" }] }
```

**Side effects:**
- Updates `watch_list` in worker state

---

### summary

Exit the worker loop with a summary.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| summary | string | yes | Summary of work done, state, next steps |

**Returns:** Tool returns, then worker exits.

**Side effects:**
- Worker sends `POST /worker/output` to orchestrator
- Worker process exits with `Done` status

---

### create_move

Create a move (summarizes a range of messages).

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| channel | object | yes | `{ adapter, id, name? }` |
| content | string | yes | The move summary |
| start_message_id | string | yes | First message in range (platform ID) |
| end_message_id | string | yes | Last message in range (platform ID) |

**Returns:**
```json
{ "id": "0000000000123456-1a2b3c4d5e6f7890", "created": true }
```

**Side effects:**
- Generates snowflake ID (`SnowflakeType::Move`)
- Appends to `workspace/moves/{channel_id}.jsonl`

---

### create_moment

Create a moment (summarizes a range of moves).

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| channel | object | yes | `{ adapter, id, name? }` |
| content | string | yes | The moment summary |
| start_move_id | string | yes | First move in range (snowflake ID) |
| end_move_id | string | yes | Last move in range (snowflake ID) |

**Returns:**
```json
{ "id": "0000000000123456-1a2b3c4d5e6f7890", "created": true }
```

**Side effects:**
- Generates snowflake ID (`SnowflakeType::Moment`)
- Appends to `workspace/moments/{channel_id}.jsonl`

---

### create_flash

Send message to another worker (peer-to-peer).

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| target | string | yes | Target worker name |
| content | string | yes | Message content |
| ttl_minutes | integer | no | Time-to-live in minutes (default: 60) |

**Returns:**
```json
{ "id": "0000000000123456-1a2b3c4d5e6f7890", "sent": true }
```

**Side effects:**
- Generates snowflake ID (`SnowflakeType::Flash`)
- Looks up target endpoint from registry
- POSTs Flash directly to target worker

**Errors:**
- `target_not_found` — No worker with that name in registry
- `target_unreachable` — Cannot reach target worker

---

### request_model

Switch to a different LLM model.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| model | string | yes | Model name (key in orchestrator's models config) |

**Returns:**
```json
{
  "switched": true,
  "model": {
    "endpoint": "https://api.anthropic.com/v1",
    "name": "claude-sonnet-4-20250514",
    "context_limit": 200000
  }
}
```

**Side effects:**
- Calls orchestrator `POST /model/switch`
- Updates `model_config` in worker state

**Errors:**
- `unknown_model` — Model name not in orchestrator config

---

### switch_roles

Switch roles with partner worker in the dyad.

**Parameters:** None

**Returns:**
```json
{
  "switched": true,
  "new_role": "spectator",
  "partner_new_role": "actor"
}
```

**Side effects:**
- Coordinates with partner worker via `/switch_roles` endpoint
- Both workers swap roles atomically
- Each worker reloads role definition from `workspace/roles/{new_role}.md`
- Updates `role` in worker state
- Notifies orchestrator to update registry

**Errors:**
- `partner_unreachable` — Cannot reach partner worker
- `switch_in_progress` — Another switch is already happening
- `partner_rejected` — Partner declined the switch (e.g., mid-operation)

---

### search_embeddings

Search embeddings, returns first result and cursor.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| query | string | yes | Search query text |

**Returns:**
```json
{
  "cursor": "emb_abc123",
  "result": {
    "id": "...",
    "content": "chunk text...",
    "source": "notes/api.md:15-42",
    "score": 0.92
  },
  "remaining": 12
}
```

- `remaining` — number of additional results available after current position

If no results:
```json
{ "cursor": null, "result": null, "remaining": 0 }
```

**Errors:**
- `embed_server_unreachable` — Cannot reach embed server

---

### next_embedding

Continue embedding search with cursor.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| cursor | string | yes | Cursor from previous search |

**Returns:**
```json
{
  "cursor": "emb_abc123",
  "result": { "id": "...", "content": "...", "source": "...", "score": 0.87 },
  "remaining": 11
}
```

When exhausted:
```json
{ "cursor": "emb_abc123", "result": null, "remaining": 0 }
```

**Errors:**
- `invalid_cursor` — Cursor not found or expired (5 min TTL)
- `embed_server_unreachable` — Cannot reach embed server

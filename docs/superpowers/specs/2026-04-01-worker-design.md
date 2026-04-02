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
  --workspace <PATH>        Path to workspace directory
  --name <NAME>             Worker name (e.g., "actor", "spectator")
  --ground <GROUND>         Human operator info: "name,adapter,channel"
  --port <PORT>             Port to bind (default: 0 for OS-assigned)
  -h, --help                Print help
```

## Configuration

```rust
pub struct WorkerConfig {
    pub orchestrator_endpoint: String,
    pub workspace: PathBuf,
    pub name: String,
    pub ground: Ground,
    pub port: u16,
}

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
3. Register with orchestrator (`POST /register`) → receive `ModelConfig`
4. Initialize `WorkerState` (current channel = ground, model config)
5. Load existing context from `workspace/{name}/context.jsonl` if exists
6. Wait for first `/notify` to start loop

**Registration request:**
```json
{
  "endpoint": "http://localhost:52341",
  "worker": {
    "name": "actor",
    "role": "actor",
    "partner": "spectator"
  }
}
```

**Registration response:**
```json
{
  "accepted": true,
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
  "initial_message": null,
  "start_sleeping": false
}
```

- `ground` — the human operator contact info (null if no ground configured)
- `initial_message` — summary from previous session (for `ContextExhausted` or timed `Done` respawn)
- `start_sleeping` — true when respawning after `Done { wake_after: None }`, worker should call `sleep(None)` immediately

## Worker State

```rust
pub struct WorkerState {
    // Communication
    pub current_channel: ChannelRef,
    pub watch_list: HashSet<ChannelRef>,
    pub registry: Registry,

    // Model
    pub model_config: ModelConfig,
    pub token_count: usize,
    pub context_limit: usize,

    // Loop control
    pub sleeping: bool,
    pub sleep_until: Option<Instant>,
    pub pending_notifications: Vec<Notification>,
    pub pending_flashes: Vec<FlashMessage>,
}

pub struct ChannelRef {
    pub adapter: String,
    pub id: String,
    pub name: Option<String>,
}

pub struct Notification {
    pub channel: ChannelRef,
    pub count: usize,
}
```

**Initial state:**
- `current_channel` = ground
- `watch_list` = empty
- `model_config` = from orchestrator
- `sleeping` = false
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

Eleven tools:

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
/// Send to current channel
pub struct SpeakTool {
    pub request: OutboundRequest,  // from river-adapter
}
// Worker overwrites the channel field with current_channel before sending
// Model can provide any channel value (it will be replaced)

/// Send to any channel
pub struct SendMessageTool {
    pub adapter: String,
    pub request: OutboundRequest,
}
// Uses channel from the OutboundRequest as-is
// Does NOT change current_channel

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
    pub add: Option<Vec<ChannelRef>>,
    pub remove: Option<Vec<ChannelRef>>,
}

/// Exit loop with summary
pub struct SummaryTool {
    pub summary: String,
}

/// Send to another worker (peer-to-peer)
pub struct FlashTool {
    pub target: String,         // target worker name
    pub content: String,        // message content
    pub ttl_minutes: Option<u32>, // time-to-live, default 60
}
// Worker generates snowflake ID, calculates expires_at from TTL
// Looks up target endpoint from registry, sends FlashMessage directly

/// Switch LLM model
pub struct RequestModelTool {
    pub model: String,
}
// Calls orchestrator POST /model/switch
// Receives new ModelConfig
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
2. Write to `workspace/conversations/{adapter}/{channel}.jsonl`
3. Batch notification for next status message
4. If sleeping and channel in watch_list: wake

Adapters fire-and-forget. Worker handles persistence.

### POST /flash

Receive flash from another worker.

```rust
pub async fn handle_flash(
    State(state): State<WorkerState>,
    Json(flash): Json<FlashMessage>,
) -> StatusCode
```

```rust
pub struct FlashMessage {
    pub id: String,          // snowflake ID (generated by sender)
    pub from: String,        // sender worker name
    pub content: String,     // message content
    pub expires_at: String,  // ISO8601 expiration time
}
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

### GET /health

Health check.

```rust
pub async fn handle_health() -> Json<HealthResponse>
```

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

**OpenAI-compatible:** Posts to `{endpoint}/chat/completions`.

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
    pub last_messages: Vec<Message>,
}

pub enum ExitStatus {
    Done { wake_after: Option<Duration> },  // None = wait for notifications
    ContextExhausted,
    Error(String),
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
- `docs/research/workspace-structure.md` — Workspace layout (draft)

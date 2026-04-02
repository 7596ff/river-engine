# Orchestrator — Design Spec

> river-orchestrator: Binary that supervises workers and adapters
>
> Authors: Cass, Claude
> Date: 2026-04-01

## Overview

The orchestrator is a process supervisor and registry manager. It spawns Workers and adapters, maintains a registry of live processes, handles model assignment, and manages worker respawn policy.

**Key characteristics:**
- Binary only (no library exports)
- JSON config file with env var substitution
- Process supervision with health checks
- Registry pushed to all processes on change
- Model config provided to workers on registration

**Not responsible for:**
- Flash routing (peer-to-peer via registry)
- Context serving (workspace-local JSONL)
- File coordination (git via bash)
- Parsing/understanding messages or flash payloads

## Crate Structure

```
river-orchestrator/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI parsing, startup sequence
│   ├── config.rs         # Config loading, env var substitution
│   ├── registry.rs       # Registry state, push logic
│   ├── supervisor.rs     # Process spawning, health checks, restart
│   ├── respawn.rs        # Respawn policy, wake timers
│   ├── http.rs           # Axum server, all endpoints
│   └── model.rs          # Model config resolution, switch handling
```

## Dependencies

```toml
[package]
name = "river-orchestrator"
version = "0.1.0"
edition = "2021"

[dependencies]
river-adapter = { path = "../river-adapter" }  # for FeatureId validation
tokio = { workspace = true }
axum = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
clap = { version = "4.0", features = ["derive"] }
thiserror = { workspace = true }
```

## CLI

```
river-orchestrator [OPTIONS]

Options:
  -c, --config <PATH>    Config file path [default: river.json]
  -p, --port <PORT>      Override config port
  -h, --help             Print help
```

## Configuration

JSON config file with env var substitution (`$VAR_NAME` syntax) for secrets.

```json
{
  "models": {
    "default": {
      "endpoint": "http://localhost:11434/v1",
      "name": "llama3.2",
      "api_key": "$OLLAMA_API_KEY",
      "context_limit": 8192
    },
    "large": {
      "endpoint": "https://api.anthropic.com/v1",
      "name": "claude-sonnet-4-20250514",
      "api_key": "$ANTHROPIC_API_KEY",
      "context_limit": 200000
    }
  },
  "workers": {
    "river": {
      "role": "actor",
      "partner": "river-spectator",
      "model": "default",
      "workspace": "/home/user/workspace/river",
      "ground": {
        "name": "alice",
        "id": "123456",
        "adapter": "discord",
        "channel": "dm-alice-123"
      },
      "adapters": [
        {
          "type": "discord",
          "binary": "river-discord",
          "config": {
            "token": "$DISCORD_TOKEN",
            "guild_id": "987654"
          }
        }
      ]
    },
    "river-spectator": {
      "role": "spectator",
      "partner": "river",
      "model": "default",
      "workspace": "/home/user/workspace/river"
    }
  },
  "port": 4000
}
```

### Config Types

```rust
pub struct Config {
    pub models: HashMap<String, ModelConfig>,
    pub workers: HashMap<String, WorkerConfig>,
    pub port: u16,
}

pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,        // supports $ENV_VAR syntax
    pub context_limit: usize,
}

pub struct WorkerConfig {
    pub role: Role,
    pub partner: Option<String>,
    pub model: String,          // references key in models map
    pub workspace: PathBuf,
    pub ground: Option<Ground>,
    pub adapters: Vec<AdapterConfig>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Actor,
    Spectator,
}
```

### Actor/Spectator Communication

Spectators have no external adapters — they don't communicate with Discord, Slack, etc. directly.

**Current mechanism (flash only):**
- Spectator shares workspace with actor (same `workspace` path)
- Actor flashes spectator for attention
- Spectator reads workspace files (conversations, context)
- Spectator writes summaries, moments, moves to workspace
- Spectator flashes actor with updates
- Spectator wakes on flash, not external notifications

**Future: Internal backchannel**
A file-based adapter (`river-file-adapter`) will provide persistent internal monologue between actor, spectator, and ground. This enables:
- Append-only conversation log between workers
- Ground (human operator) can read/write to the backchannel
- History preserved across context rotations

The file adapter will be specced separately.

```rust

pub struct Ground {
    pub name: String,
    pub id: String,
    pub adapter: String,
    pub channel: String,
}

pub struct AdapterConfig {
    pub r#type: String,         // "discord", "slack"
    pub binary: String,         // path to adapter binary
    pub config: Value,          // adapter-specific config (passed as JSON to binary)
}
```

## Registry

Tracks all live processes. Pushed to all processes on any change.

```rust
pub struct Registry {
    pub processes: Vec<ProcessEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProcessEntry {
    Worker {
        endpoint: String,
        name: String,
        role: Role,
        partner: Option<String>,
        model: String,
        ground: Option<Ground>,
    },
    Adapter {
        endpoint: String,
        r#type: String,
        worker_name: String,
        features: Vec<u16>,  // FeatureId as u16
    },
}
```

### Registration

**Worker registration request:**
```json
{
  "endpoint": "http://localhost:52341",
  "worker": {
    "name": "river",
    "role": "actor",
    "partner": "river-spectator"
  }
}
```

**Worker registration response:**
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

- `initial_message` — populated when respawning after `ContextExhausted` or timed `Done`, contains the previous summary
- `start_sleeping` — true when respawning after `Done { wake_after: None }`, worker should call `sleep(None)` immediately after loading context

**Adapter registration request:**
```json
{
  "endpoint": "http://localhost:52343",
  "adapter": {
    "type": "discord",
    "worker_name": "river",
    "features": [0, 1, 10, 11, 12, 20, 40]
  }
}
```

**Adapter registration response:**
```json
{
  "accepted": true
}
```

Orchestrator validates adapters have required features (SendMessage=0, ReceiveMessage=1). Registration rejected if missing.

### Registry Push

When registry changes, orchestrator pushes full registry to every live process:

```
POST {process_endpoint}/registry
{
  "processes": [...]
}
```

Each process keeps a local copy for direct routing (e.g., peer-to-peer flash).

## Startup Sequence

1. Parse CLI args, load config file
2. Resolve env vars in config (API keys, tokens)
3. Bind HTTP server to configured port
4. For each worker in config:
   a. Spawn worker binary: `river-worker --orchestrator http://...:4000 --name {name} --workspace {path}`
   b. Worker binds port 0, registers with orchestrator
   c. Orchestrator responds with ModelConfig + Ground
   d. For each adapter in worker config:
      - Spawn adapter binary with config JSON as arg
      - Adapter binds port 0, registers with orchestrator (including features)
      - Orchestrator validates required features, rejects if missing
      - Push updated registry to all processes
5. Workers wait for first `/notify` from adapters to start loop
6. Orchestrator enters supervision loop

## Process Supervision

**Health checks:**
- `GET /health` to every process every 60 seconds
- Process considered dead after 3 consecutive failures (3 minutes)
- Dead processes removed from registry, update pushed to survivors

**Crash handling:**
- Worker crash → respawn per policy, worker loads context from workspace JSONL
- Adapter crash → restart and re-register, push updated registry

**Graceful shutdown:**
1. Orchestrator receives SIGINT/SIGTERM
2. Send SIGTERM to all worker and adapter processes
3. Workers handle SIGTERM by calling `summary` tool and exiting
4. Wait up to 5 minutes for workers to exit cleanly
5. Send SIGKILL to any remaining processes
6. Exit

## Worker Output and Respawn

Worker exits via `POST /worker/output`:

```rust
pub struct WorkerOutput {
    pub status: ExitStatus,
    pub summary: String,
}

pub enum ExitStatus {
    Done { wake_after: Option<Duration> },  // None = wait for notifications
    ContextExhausted,
    Error(String),
}
```

**Respawn behavior:**

| Exit Status | Action |
|-------------|--------|
| `Done { wake_after: None }` | Respawn immediately with `start_sleeping: true`. Worker loads context, calls `sleep(None)`. Wakes only on watched channel notifications. |
| `Done { wake_after: Some(30m) }` | Orchestrator waits 30 minutes, then respawns worker with `initial_message` set to summary. |
| `ContextExhausted` | Respawn immediately with `initial_message` set to summary. |
| `Error` | Respawn immediately. Worker loads existing workspace JSONL. |

The orchestrator manages wake timers — worker process doesn't stay alive during the wait.

## Model Switching

Workers can request a different model mid-session via `request_model` tool.

**Worker calls orchestrator:**
```
POST /model/switch
{
  "worker_name": "river",
  "model": "large"
}
```

**Orchestrator:**
1. Looks up model config by name
2. Resolves API key from env var
3. Updates worker's model assignment in registry
4. Pushes updated registry to all processes
5. Returns new ModelConfig to worker

**Response:**
```json
{
  "endpoint": "https://api.anthropic.com/v1",
  "name": "claude-sonnet-4-20250514",
  "api_key": "sk-...",
  "context_limit": 200000
}
```

Worker uses new model on next LLM call. No restart required.

**Error:** Unknown model name returns 400, worker continues with current model.

## HTTP API

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/register` | Process registration (worker or adapter) |
| POST | `/model/switch` | Worker requests model change |
| POST | `/worker/output` | Worker sends exit status + summary |
| GET | `/registry` | Query current process registry |
| GET | `/health` | Orchestrator health check |

### POST /register

See Registration section above.

### POST /model/switch

```json
// Request
{
  "worker_name": "river",
  "model": "large"
}

// Response 200
{
  "endpoint": "https://api.anthropic.com/v1",
  "name": "claude-sonnet-4-20250514",
  "api_key": "sk-...",
  "context_limit": 200000
}

// Response 400
{
  "error": "unknown model: extra-large"
}
```

### POST /worker/output

```json
// Request
{
  "worker_name": "river",
  "output": {
    "status": { "Done": { "wake_after": null } },
    "summary": "Completed task X, waiting for user response."
  }
}

// Response 200
{
  "acknowledged": true
}
```

### GET /registry

```json
{
  "processes": [
    {
      "endpoint": "http://localhost:52341",
      "name": "river",
      "role": "actor",
      "partner": "river-spectator",
      "model": "default",
      "ground": { "name": "alice", "id": "123456", "adapter": "discord", "channel": "dm-alice-123" }
    },
    {
      "endpoint": "http://localhost:52343",
      "type": "discord",
      "worker_name": "river",
      "features": [0, 1, 10, 11, 12, 20, 40]
    }
  ]
}
```

### GET /health

```json
{
  "status": "ok",
  "workers": 2,
  "adapters": 1
}
```

## Error Handling

**Config errors:**
- Missing required fields → exit with error on startup
- Invalid env var reference → exit with error on startup
- Unknown model reference in worker config → exit with error on startup

**Registration errors:**
- Adapter missing required features → reject with 400
- Unknown worker name in adapter registration → reject with 400
- Duplicate registration → update endpoint, push registry

**Runtime errors:**
- Process unreachable → health check failure tracking
- Model switch for unknown model → 400 error, worker continues
- Worker output for unknown worker → log warning, ignore

## Related Documents

- `docs/ORCHESTRATOR-DESIGN.md` — High-level orchestrator design
- `docs/superpowers/specs/2026-04-01-worker-design.md` — Worker architecture
- `docs/superpowers/specs/2026-04-01-adapter-library-design.md` — Adapter types

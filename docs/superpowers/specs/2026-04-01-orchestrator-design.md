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
river-adapter = { path = "../river-adapter" }  # FeatureId, Baton, Side, Ground
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
    },
    "embed": {
      "endpoint": "http://localhost:11434/api/embeddings",
      "name": "nomic-embed-text",
      "api_key": "$OLLAMA_API_KEY",
      "dimensions": 768
    }
  },
  "embed": {
    "model": "embed"
  },
  "dyads": {
    "river": {
      "workspace": "/home/user/workspace/river",
      "left_model": "large",
      "right_model": "default",
      "left_starts_as": "actor",
      "ground": {
        "name": "alice",
        "id": "123456",
        "channel": { "adapter": "discord", "id": "dm-alice-123", "name": null }
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
    }
  },
  "port": 4000
}
```

### Config Types

```rust
pub struct Config {
    pub models: HashMap<String, ModelConfig>,
    pub embed: Option<EmbedConfig>,
    pub dyads: HashMap<String, DyadConfig>,
    pub port: u16,
}

pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,        // supports $ENV_VAR syntax
    pub context_limit: Option<usize>,   // for LLM models
    pub dimensions: Option<usize>,      // for embedding models
}
// Validation: orchestrator ensures context_limit is present when sending to workers,
// and dimensions is present when sending to embed service.

pub struct EmbedConfig {
    pub model: String,          // references key in models map
}

pub struct DyadConfig {
    pub workspace: PathBuf,
    pub left_model: String,           // references key in models map
    pub right_model: String,          // references key in models map
    pub left_starts_as: Baton,        // which baton left worker starts with
    pub ground: Ground,
    pub adapters: Vec<AdapterConfig>,
}

// Baton imported from river-adapter
pub use river_adapter::Baton;
```

### Dyad Model

A dyad is a pair of workers (left and right) that share a workspace. The orchestrator spawns both workers for each dyad.

**Key properties:**
- Both workers share the same workspace
- Each worker has a fixed model assignment (`left_model`, `right_model`)
- Each worker has a fixed identity (`workspace/left/identity.md`, `workspace/right/identity.md`)
- Workers can switch roles via `switch_roles` tool
- Role determines behavior (`workspace/roles/actor.md`, `workspace/roles/spectator.md`)

**Communication:**
- Workers flash each other for attention (peer-to-peer)
- Spectator reads workspace files (conversations, context)
- Spectator writes summaries, moments, moves to workspace
- Actor handles external communication (adapters)
- Spectator wakes on flash, not external notifications

**Role switching:**
- Either worker can call `switch_roles`
- Both workers coordinate the swap
- Left loads spectator role, right loads actor role (or vice versa)
- Identities and contexts stay with their workers

```rust

// Ground imported from river-adapter
pub use river_adapter::Ground;

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
        dyad: String,            // dyad name (e.g., "river")
        side: Side,              // "left" or "right"
        baton: Baton,            // current baton (actor or spectator)
        model: String,
        ground: Ground,
    },
    Adapter {
        endpoint: String,
        r#type: String,          // adapter type (discord, slack, etc.)
        dyad: String,            // which dyad this adapter serves
        features: Vec<u16>,      // FeatureId as u16
    },
    EmbedService {
        endpoint: String,
        name: String,            // service name (e.g., "embed")
    },
}

// Side imported from river-adapter
pub use river_adapter::Side;
```

### Registration

**Worker registration request:**
```json
{
  "endpoint": "http://localhost:52341",
  "worker": {
    "dyad": "river",
    "side": "left"
  }
}
```

Orchestrator looks up dyad config and returns configuration for this side.

**Worker registration response:**
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
    "channel": { "adapter": "discord", "id": "dm-alice-123", "name": null }
  },
  "workspace": "/home/user/workspace/river",
  "initial_message": null,
  "start_sleeping": false
}
```

- `baton` — initial baton (actor or spectator, based on `left_starts_as` config)
- `partner_endpoint` — endpoint of the other worker in dyad (null if not yet registered)
- `ground` — the human operator contact info
- `workspace` — path to shared workspace
- `initial_message` — populated when respawning after `ContextExhausted` or timed `Done`, contains the previous summary
- `start_sleeping` — true when respawning after `Done { wake_after_minutes: None }`, worker should call `sleep(None)` immediately after loading context

**Adapter registration request:**
```json
{
  "endpoint": "http://localhost:52343",
  "adapter": {
    "dyad": "river",
    "type": "discord",
    "features": [0, 1, 10, 11, 12, 20, 40]
  }
}
```

Orchestrator looks up adapter config from `dyads[dyad].adapters` by type.

**Adapter registration response:**
```json
{
  "accepted": true,
  "worker_endpoint": "http://localhost:52341",
  "validated_features": [0, 1, 10, 11, 12, 20, 40],
  "config": {
    "token": "actual-discord-token-resolved-from-env",
    "guild_id": "987654"
  }
}
```

The `config` field contains adapter-specific configuration from the orchestrator config file, with env vars resolved. Secrets never appear on command line.

**Feature validation:**

```rust
use river_adapter::FeatureId;

fn validate_adapter_features(features: &[u16]) -> Result<Vec<FeatureId>, RegistrationError> {
    let parsed: Vec<FeatureId> = features.iter()
        .map(|&f| FeatureId::try_from(f))
        .collect::<Result<_, _>>()?;

    // Required features
    if !parsed.contains(&FeatureId::SendMessage) {
        return Err(RegistrationError::MissingFeature(FeatureId::SendMessage));
    }
    if !parsed.contains(&FeatureId::ReceiveMessage) {
        return Err(RegistrationError::MissingFeature(FeatureId::ReceiveMessage));
    }

    Ok(parsed)
}
```

Registration rejected if required features missing or unknown feature IDs provided.

**Embed service registration request:**
```json
{
  "endpoint": "http://localhost:52350",
  "embed": {
    "name": "embed"
  }
}
```

**Embed service registration response:**
```json
{
  "accepted": true,
  "model": {
    "endpoint": "http://localhost:11434/api/embeddings",
    "name": "nomic-embed-text",
    "api_key": "...",
    "dimensions": 768
  }
}
```

Orchestrator looks up embed model config by name (from `embed.model` in config) and returns it.

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
4. If embed config present:
   a. Spawn embed service: `river-embed --orchestrator http://...:4000 --name embed`
   b. Embed service binds port 0, registers with orchestrator
   c. Orchestrator responds with embed model config
   d. Push registry with embed endpoint to all processes
5. For each dyad in config:
   a. Spawn left worker: `river-worker --orchestrator http://...:4000 --dyad {name} --side left`
   b. Spawn right worker: `river-worker --orchestrator http://...:4000 --dyad {name} --side right`
   c. Workers bind port 0, register with orchestrator
   d. Orchestrator responds with ModelConfig, baton, ground, workspace
   e. For each adapter in dyad config:
      - Spawn adapter: `river-{type} --orchestrator http://...:4000 --dyad {name} --type {type}`
      - Adapter binds port 0, registers with orchestrator (dyad, type, features)
      - Orchestrator validates required features, responds with config + worker endpoint
      - Adapter initializes connection to platform using received config
      - Push updated registry to all processes
6. Actor waits for first `/notify` from adapters to start loop
7. Spectator waits for first `/flash` from actor to start loop
8. Orchestrator enters supervision loop

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
    Done { wake_after_minutes: Option<u64> },  // None = wait for notifications
    ContextExhausted,
    Error(String),
}
```

**Respawn behavior:**

| Exit Status | Action |
|-------------|--------|
| `Done { wake_after_minutes: None }` | Respawn immediately with `start_sleeping: true`. Worker loads context, calls `sleep(None)`. Wakes only on watched channel notifications. |
| `Done { wake_after_minutes: Some(30) }` | Orchestrator waits 30 minutes, then respawns worker with `initial_message` set to summary. |
| `ContextExhausted` | Respawn immediately with `initial_message` set to summary. |
| `Error` | Respawn immediately. Worker loads existing workspace JSONL. |

The orchestrator manages wake timers — worker process doesn't stay alive during the wait.

**Summary persistence:**

Orchestrator stores worker summaries in memory, keyed by `(dyad, side)`:

```rust
struct OrchestratorState {
    summaries: HashMap<(String, Side), String>,  // (dyad, side) -> summary
    wake_timers: HashMap<(String, Side), Instant>,
}
```

When a worker exits with a summary, orchestrator stores it. On respawn, orchestrator includes the stored summary as `initial_message`. Summaries are lost if orchestrator restarts — worker falls back to loading from workspace JSONL.

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
| POST | `/switch_roles` | Worker requests role switch with partner |
| POST | `/worker/output` | Worker sends exit status + summary |
| GET | `/registry` | Query current process registry |
| GET | `/health` | Orchestrator health check |

### POST /register

See Registration section above.

### POST /model/switch

```json
// Request
{
  "dyad": "river",
  "side": "left",
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

### POST /switch_roles

Orchestrator-mediated role switch between dyad workers. Ensures atomicity.

```json
// Request
{
  "dyad": "river",
  "side": "left"
}

// Response 200
{
  "switched": true,
  "your_new_baton": "spectator",
  "partner_new_baton": "actor"
}

// Response 409 (partner busy or switch in progress)
{
  "error": "partner_busy",
  "message": "Partner worker is mid-operation"
}

// Response 503 (partner unreachable)
{
  "error": "partner_unreachable"
}
```

**Protocol:**

```
Worker                 Orchestrator                Partner
   |--POST /switch_roles-->|                           |
   |                       | [acquire dyad lock]       |
   |                       |--POST /prepare_switch---->|
   |                       |<--{"ready":true}----------|
   |                       |--POST /prepare_switch---->| (to initiator too)
   |<--prepare_switch------|                           |
   |--{"ready":true}------>|                           |
   |                       | [both ready, commit]      |
   |                       |--POST /commit_switch----->|
   |<--commit_switch-------|                           |
   |                       | [update registry]         |
   |<--{"switched":true}---|                           |
   |                       | [push registry to all]    |
```

**Atomicity guarantees:**
- Dyad lock prevents concurrent switch attempts
- Both workers must respond "ready" to prepare
- If either fails prepare, orchestrator aborts (no commit sent)
- If commit fails, orchestrator marks worker unhealthy for restart

**Race handling:**
- First switch request acquires lock
- Concurrent requests get 409 "switch_in_progress"

### POST /worker/output

```json
// Request
{
  "dyad": "river",
  "side": "left",
  "output": {
    "status": { "Done": { "wake_after_minutes": null } },
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
      "dyad": "river",
      "side": "left",
      "baton": "actor",
      "model": "large",
      "ground": { "name": "alice", "id": "123456", "channel": { "adapter": "discord", "id": "dm-alice-123" } }
    },
    {
      "endpoint": "http://localhost:52342",
      "dyad": "river",
      "side": "right",
      "baton": "spectator",
      "model": "default",
      "ground": { "name": "alice", "id": "123456", "channel": { "adapter": "discord", "id": "dm-alice-123" } }
    },
    {
      "endpoint": "http://localhost:52343",
      "type": "discord",
      "dyad": "river",
      "features": [0, 1, 10, 11, 12, 20, 40]
    },
    {
      "endpoint": "http://localhost:52350",
      "name": "embed"
    }
  ]
}
```

### GET /health

```json
{
  "status": "ok",
  "workers": 2,
  "adapters": 1,
  "embed_services": 1
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

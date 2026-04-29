# Remove Dyad Functionality

Date: 2026-04-29

## Goal

Replace the dyad model (two workers per pair, left/right sides, actor/spectator role switching) with a single-worker-per-agent model. The orchestrator remains a process supervisor that can manage multiple agents, but each agent is one worker with one workspace, one model, and one ground.

## What Gets Removed

- `Side` enum (`Left`/`Right`)
- `Baton` enum (`Actor`/`Spectator`)
- All role/baton switching infrastructure: `/switch_roles` endpoint, two-phase prepare/commit/abort, `dyad_locks`, role file loading
- Git worktree creation (`ensure_worktree_exists`, `is_valid_worktree`, `spawn_dyad` worktree logic)
- Partner endpoint tracking (`get_partner_endpoint`, `partner_side`)
- `SideConfig` struct
- `DyadConfig` struct (replaced by `AgentConfig`)
- TUI backchannel (communicated with partner worker)
- `role_content` from worker state and context assembly
- `ToolResult::SwitchRoles` variant

## What Stays

- Process supervision: spawn, health check, respawn
- Service registry + push to all processes
- Adapter system: features, registration, inbound/outbound events
- Worker loop: LLM calls, tool execution, context assembly, sleep/wake, notifications, forced summary
- Embed service (untouched)
- Respawn system: exit status, wake timers, summary injection
- Config with env var substitution and validation
- Snowflake ID generation (untouched)

## Naming Change

All occurrences of `dyad` rename to `agent` across config, CLI args, HTTP payloads, registry keys, struct fields, and variable names.

---

## Crate-by-Crate Changes

### river-protocol

Delete `Side` and `Baton` enums from `identity.rs`. Remove their re-exports from `lib.rs`.

`ProcessEntry::Worker` becomes:
```rust
Worker {
    endpoint: String,
    agent: String,
    model: String,
    ground: Ground,
}
```

`WorkerRegistration` becomes:
```rust
pub struct WorkerRegistration {
    pub agent: String,
}
```

`WorkerRegistrationResponse` becomes:
```rust
pub struct WorkerRegistrationResponse {
    pub accepted: bool,
    pub name: String,
    pub model: ModelConfig,
    pub ground: Ground,
    pub workspace: String,
    pub initial_message: Option<String>,
}
```

Removed from response: `baton`, `partner_endpoint`, `worktree_path`, `start_sleeping`.

`AdapterRegistration.dyad` renames to `agent`.

`Ground`, `Author`, `Channel`, `Attachment`, `ModelConfig`, `Registry` — unchanged.

Update all serde roundtrip tests.

### river-adapter

Remove `Baton` and `Side` from re-exports. Remove them from OpenAPI schema list. Everything else unchanged — `FeatureId`, `OutboundRequest`, `InboundEvent`, `EventMetadata`, `Ground`, `Author`, `Channel`, `Attachment` have no dyad references.

### river-orchestrator

#### Config

Replace `DyadConfig` with:
```rust
pub struct AgentConfig {
    pub workspace: PathBuf,
    pub name: String,
    pub model: String,
    pub ground: Ground,
    pub adapters: Vec<AdapterConfig>,
}
```

Delete `SideConfig`. `Config.dyads` becomes `Config.agents`. `AdapterConfig` loses its `side` field.

Validation: check that each agent's `model` reference exists in `Config.models` and has `context_limit`.

#### Supervisor

`ProcessKey::Worker` becomes `Worker { agent: String }`. `ProcessKey::Adapter` — `dyad` field renames to `agent`.

Delete: `spawn_dyad`, `ensure_worktree_exists`, `is_valid_worktree`, `SupervisorError::WorktreeCreationFailed`.

Add: `spawn_agent(supervisor, orchestrator_url, agent_name, agent_config)` — spawns one worker and its adapters. No worktree creation.

`spawn_worker` — remove `side` parameter, pass `--agent` instead of `--dyad`/`--side`.

`spawn_adapter` — `dyad` param renames to `agent`.

#### Registry

`WorkerKey` becomes `{ agent: String }`. `AdapterKey.dyad` renames to `agent`.

Delete: `get_partner_endpoint`, `update_worker_baton`, `get_worker_baton`.

`register_worker` signature:
```rust
pub fn register_worker(
    &mut self,
    agent: String,
    endpoint: String,
    model: String,
    ground: Ground,
)
```

No `side`, no `baton`.

#### HTTP

Delete endpoints and types:
- `POST /switch_roles` handler and all supporting functions (`prepare_both`, `commit_both`, `send_abort`)
- `SwitchRolesRequest`, `SwitchRolesResponse`, `SwitchRolesError`
- `PrepareResult` enum
- `SWITCH_PHASE_TIMEOUT` constant

Delete from `AppState`: `dyad_locks`.

Rename in all remaining handlers: `dyad` to `agent` in request/response structs and logic.

`WorkerRegistrationResponse` (the orchestrator's version in `http.rs`):
```rust
pub struct WorkerRegistrationResponse {
    pub accepted: bool,
    pub name: String,
    pub model: WorkerModelConfig,
    pub ground: Ground,
    pub workspace: String,
    pub initial_message: Option<String>,
}
```

Registration handler: no baton assignment, no partner lookup, no worktree path construction, no respawn `start_sleeping` check. Look up agent config, return model + ground + workspace + name + initial_message.

#### Respawn

`WorkerKey` and `WorkerOutput` — `dyad` renames to `agent`, `side` field removed.

`RespawnState` — remove `start_sleeping`. Workers that exit with `Done { wake_after_minutes: None }` get respawned immediately without sleep state (the sleep/wake behavior moves to be worker-internal if needed).

`RespawnAction::ImmediateWithSleep` — remove this variant. Replace with just `Immediate`.

#### Model

`ModelSwitchRequest.dyad` renames to `agent`. Remove `side` field.

#### Main

Iterate `config.agents` instead of `config.dyads`. Call `spawn_agent` instead of `spawn_dyad`. No worktree setup. Health check loop and respawn wake loop — same logic, just keyed on agent name instead of dyad+side.

### river-worker

#### CLI

```
--orchestrator URL --agent NAME
```

Remove `--side`.

#### Config

```rust
pub struct WorkerConfig {
    pub orchestrator_endpoint: String,
    pub agent: String,
    pub port: u16,
}
```

Remove `side`. Remove `workspace_path()` side-based derivation — workspace comes directly from registration response. Remove `role_path()` entirely.

#### State

```rust
pub struct WorkerState {
    pub name: String,
    pub agent: String,
    pub ground: Ground,
    pub workspace: PathBuf,
    pub current_channel: Channel,
    pub watch_list: HashSet<String>,
    pub registry: Registry,
    pub model_config: ModelConfig,
    pub token_count: usize,
    pub context_limit: usize,
    pub sleeping: bool,
    pub sleep_until: Option<Instant>,
    pub pending_notifications: Vec<Notification>,
    pub pending_flashes: Vec<Flash>,
    pub identity_content: Option<String>,
    pub initial_message: Option<String>,
}
```

Removed: `side`, `baton`, `partner_endpoint`, `switch_pending`, `role_content`.

Remove `partner_side()` method.

#### Worker loop

Remove `ToolResult::SwitchRoles` variant and all baton/role-switch handling in the tool result processing.

Remove role content from `assemble_full_context`. Context structure becomes: identity + workspace context + history.

`WorkerOutput` — `dyad` renames to `agent`, `side` removed.

#### Main

Remove side parsing. Send `WorkerRegistration { agent }`. No role file loading. No `start_sleeping` from registration.

#### Tools

Remove `switch_roles` tool definition and execution. Remove any tool that references partner endpoint.

#### HTTP

Remove `/prepare_switch`, `/commit_switch`, `/abort_switch` endpoints if they exist on the worker side. Remove `switch_pending` state checks.

### river-tui

Remove backchannel module and any partner-communication code. Rename `dyad` to `agent` in adapter registration args and config.

### river-discord

Rename `dyad` to `agent` in CLI args and registration payload.

### river-embed

No changes.

### river-context

No changes.

### river-snowflake

No changes.

---

## Tests

### Delete

- `e2e_dyad_boot.rs` — fundamentally about the old model
- Baton swap tests in `registry.rs`
- Role switching serde tests in `http.rs`
- Worktree tests in `supervisor.rs`
- `Side` and `Baton` serde roundtrip tests in protocol

### Update

- Registry tests: rename dyad to agent, remove side from keys
- Supervisor tests: rename process keys, remove side
- Respawn tests: rename dyad to agent, remove side
- Config tests: new `AgentConfig` shape validation
- Protocol serde tests: updated types without side/baton
- Worker state tests: updated constructor

### New

- E2E: single agent boot — orchestrator spawns one worker + adapter, worker registers, receives correct config

---

## Config Example

Before:
```json
{
  "models": {
    "claude": {
      "endpoint": "https://api.anthropic.com/v1",
      "name": "claude-sonnet-4-20250514",
      "api_key": "$ANTHROPIC_API_KEY",
      "context_limit": 200000
    }
  },
  "dyads": {
    "river": {
      "workspace": "/home/cassie/stream",
      "left": { "name": "Iris", "model": "claude" },
      "right": { "model": "claude" },
      "initialActor": "left",
      "ground": {
        "name": "Cassie",
        "id": "user123",
        "adapter": "discord",
        "channel": "dm-channel"
      },
      "adapters": [
        { "path": "river-discord", "side": "left", "token": "$DISCORD_TOKEN" }
      ]
    }
  }
}
```

After:
```json
{
  "models": {
    "claude": {
      "endpoint": "https://api.anthropic.com/v1",
      "name": "claude-sonnet-4-20250514",
      "api_key": "$ANTHROPIC_API_KEY",
      "context_limit": 200000
    }
  },
  "agents": {
    "iris": {
      "workspace": "/home/cassie/stream",
      "name": "Iris",
      "model": "claude",
      "ground": {
        "name": "Cassie",
        "id": "user123",
        "adapter": "discord",
        "channel": "dm-channel"
      },
      "adapters": [
        { "path": "river-discord", "token": "$DISCORD_TOKEN" }
      ]
    }
  }
}
```

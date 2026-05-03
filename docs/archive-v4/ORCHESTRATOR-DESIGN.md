# River Orchestrator — Design (WIP)

## Philosophy

The orchestrator is a process supervisor and message router. It spawns Workers and adapters, maintains a registry of live processes, and keeps everyone informed about who else exists. It does not make decisions about what the agent should do — that's the Worker's (model's) job.

## What The Orchestrator Is

Six things:

1. **Process supervisor** — spawns, monitors, and restarts child processes (Workers, adapters)
2. **Registry** — tracks all live processes and their endpoints
3. **Config** — CLI options define what to spawn and how to wire it together
4. **Model manager** — assigns LLM models to Workers, handles switch requests
5. **Flash router** — delivers flash messages between Workers
6. **Context server** — serves context (JSONL) to Workers on startup/restart

## CLI Configuration

All config is provided via CLI options. No config file for v0.

```bash
river-orchestrator \
  --adapter-binary discord=river-discord \
  --adapter-binary slack=river-slack \
  --model default=http://localhost:11434/v1,llama3.2 \
  --model large=https://api.anthropic.com/v1,claude-sonnet-4-20250514 \
  --worker river,actor,river-spectator,default,workspace/river \
  --worker-prompt river=workspace/identity.md,workspace/rules.md \
  --worker-adapter river=discord,token_file=/secrets/discord-token,guild_id=123456 \
  --worker river-spectator,spectator,river,default,workspace/river-spectator \
  --worker-prompt river-spectator=workspace/spectator-identity.md,workspace/spectator-rules.md \
  --ground alice,discord,dm-alice-123 \
  --port 4000
```

The exact CLI format will be refined during implementation. The important thing is: no JSON config file, everything is flags.

## Startup Sequence

```
1. Orchestrator parses CLI options
2. For each worker:
   a. Build system prompt from prompt files
   b. Spawn Worker process (passing orchestrator endpoint as CLI arg)
   c. Worker binds to port 0, registers with orchestrator
   d. For each adapter bound to this worker:
      i.   Spawn adapter binary (passing orchestrator endpoint + config as CLI args)
      ii.  Adapter binds to port 0, registers with orchestrator (including features)
      iii. Orchestrator pushes updated registry to all processes
   e. Worker requests its context from the orchestrator:
      - If JSONL context file exists (crash recovery): serve it
      - Otherwise: serve empty context
   f. First real notification arrives from adapters (or ground) → loop begins
3. Orchestrator enters supervision loop
```

### Context Serving

The orchestrator holds the JSONL context files for each Worker. On startup or restart, the Worker requests its context:

**GET /context/{worker_name}** → returns the JSONL context or empty

This means the orchestrator is the source of truth for context, not the Worker's local filesystem. The Worker still persists after every mutation (writing to the orchestrator), and the orchestrator writes to disk.

**POST /context/{worker_name}** → Worker persists updated context

This also enables the orchestrator to serve fresh context with up-to-date adapter endpoints after a restart, avoiding stale endpoint problems.

## Process Registry

The orchestrator maintains a registry of all live processes:

```json
{
  "processes": [
    {
      "endpoint": "http://localhost:52341",
      "worker": {
        "name": "river",
        "role": "actor",
        "partner": "river-spectator",
        "model": "default"
      }
    },
    {
      "endpoint": "http://localhost:52342",
      "worker": {
        "name": "river-spectator",
        "role": "spectator",
        "partner": "river",
        "model": "default"
      }
    },
    {
      "endpoint": "http://localhost:52343",
      "adapter": {
        "type": "discord",
        "worker_name": "river",
        "features": [0, 1, 10, 11, 12, 20, 40]
      }
    }
  ]
}
```

### Registration

**POST /register**

Worker registration:
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

Adapter registration (now includes features):
```json
{
  "endpoint": "http://localhost:52343",
  "adapter": {
    "type": "discord",
    "worker_name": "river",
    "features": [0, 1, 10, 11, 12, 20, 21, 40, 100, 101, 102, 200, 900]
  }
}
```

One of `worker` or `adapter` is present, not both. The orchestrator adds the process to the registry and pushes the updated registry to all live processes.

### Port Assignment

Child processes bind to port 0 on startup. The OS assigns a random available port. The process discovers its port via `getsockname()` and reports it in the `endpoint` field during registration. No port range management needed.

### Registry Push

When the registry changes (process registers, dies, or updates), the orchestrator pushes the full registry to every live process:

**POST {process_endpoint}/registry**
```json
{
  "processes": [...]
}
```

Each process keeps a local copy of the registry so it can route messages directly without asking the orchestrator.

### Feature Negotiation

When building the system prompt for a Worker, the orchestrator includes the feature list from each of the Worker's bound adapters. The model knows from the start what each adapter can do:

```
Your adapters:
- discord: SendMessage(0), ReceiveMessage(1), EditMessage(10), DeleteMessage(11),
  ReadHistory(12), AddReaction(20), RemoveReaction(21), TypingIndicator(40),
  VoiceStateEvents(100), PresenceEvents(101), MemberEvents(102),
  ChannelEvents(200), ConnectionEvents(900)
```

## Actor / Spectator Roles

Workers have roles: **actor** and **spectator**. These are paired via the `partner` field. The orchestrator knows about this relationship and manages the flash mechanism between them.

- **Actor** — the Worker that acts in the world (reads channels, uses tools, speaks to users)
- **Spectator** — the Worker that observes the actor and manages memory, compression, curation

They communicate through the **flash** mechanism (see below). They can also share a workspace for file-based coordination.

## Flash System

> **Note:** The flash system is part of a larger context-building system that will be a separate crate. The orchestrator's role is limited to routing flash messages between Workers. The full design of the context-building system (including how the spectator builds and manages context, compression, curation, and flash generation) will be designed separately.

Flash is a high-priority interrupt mechanism between Workers. Any Worker can flash any other Worker by name. Unlike regular notifications that get batched, a flash is delivered as soon as possible — even between tool executions within a turn. The orchestrator is a dumb router; the receiving Worker handles its own interrupt timing.

### Flow

```
Any Worker                          Orchestrator                    Target Worker
      │                                  │                              │
      ├──flash tool call───────────────▶│                              │
      │  {name:"river", payload:{...}}   │                              │
      │                                  ├──POST /flash───────────────▶│
      │                                  │  {from:"river-spectator",    │
      │                                  │   payload:{...}}             │
      │                                  │                              │
      │                                  │              Target Worker injects flash
      │                                  │              as high-priority system message
      │                                  │              BETWEEN tool calls if needed
```

### Flash Endpoint

**POST /flash** (on orchestrator)
```json
{
  "name": "river",
  "payload": {}
}
```

The orchestrator looks up the target Worker by name, forwards the flash to its endpoint:

**POST {worker_endpoint}/flash**
```json
{
  "from": "river-spectator",
  "payload": {}
}
```

### Flash Delivery Timing

The receiving Worker handles interrupt timing itself. It injects the flash into its message list as a high-priority system message:

- **Between tool calls** — injected before the next tool result is sent to the model
- **Before the next LLM call** — prepended to the message list
- **Immediately on wake** — if the Worker is sleeping, the flash wakes it
- **After a streaming response** — if the LLM is mid-generation, the flash is queued and injected immediately after the response completes, before processing any tool calls

### Worker Flash Tool

```json
{
  "name": "string (required, target worker name)",
  "payload": "object (required)"
}
```

This calls the orchestrator's `/flash` endpoint. The orchestrator routes it to the target Worker.

## Process Supervision

The orchestrator monitors child processes:

- **Health checks** — `GET /health` to every process every **60 seconds**
- **Crash detection** — if a process fails 3 consecutive health checks (**3 minutes**), it's considered dead
- **Restart policy** — on crash:
  - **Worker**: respawn. Worker requests context from orchestrator on startup (fresh registry, fresh adapter endpoints, existing JSONL context).
  - **Adapter**: restart and re-register. The bound Worker continues — it just can't send/receive on that adapter until it comes back. Push updated registry once adapter re-registers.
- **Registry cleanup** — dead processes are removed from the registry, update pushed to all survivors

## Worker Lifecycle

```
Orchestrator                          Worker
    │                                   │
    ├──spawn process──────────────────▶│
    │   (--orchestrator http://...:4000) │
    │                                   │
    │                          (binds port 0, starts HTTP server)
    │                                   │
    │◀──POST /register────────────────│
    │   {endpoint:"...", worker:{...}}  │
    │                                   │
    ├──spawn adapters─────────────────▶(adapter processes)
    │                                   │
    ├──POST /registry────────────────▶│
    │   (here's who's alive)            │
    │                                   │
    │◀──GET /context/river─────────────│
    │   (Worker requests its context)   │
    │──returns JSONL or empty─────────▶│
    │                                   │
    │    ... first notification arrives from adapter or ground ...
    │    ... Worker loop begins ...      │
    │                                   │
    │◀──WorkerOutput (summary called)──│
    │   {status: Done, summary: "..."}  │
    │                                   │
    ├──respawn per policy (see below)   │
    │                                   │
```

### Respawn Policy

When a Worker exits cleanly (via `summary` tool or context exhaustion):

| Exit Status | Action |
|-------------|--------|
| `Done` | Respawn Worker in permanent sleep (waiting for notifications on watched channels). Summary is saved but not injected — the Worker starts fresh but can read its history from workspace files. |
| `ContextExhausted` | Respawn Worker immediately with the summary injected as a user-role message in the initial context. The Worker picks up where it left off. |
| `Error` | Restart from JSONL context if available. Otherwise respawn fresh. |

For `Done`, "permanent sleep" means the Worker starts, requests context (empty for a fresh start), and immediately enters `sleep()` with no timer. It wakes only when a watched channel gets a notification.

### Model Switching

The orchestrator picks the initial model for each Worker from CLI config. The Worker can request a different model via the `request_model` tool:

**Worker tool: `request_model`**
```json
{
  "model": "large"
}
```

The Worker's runtime calls the orchestrator:

**POST {orchestrator}/model/switch**
```json
{
  "worker_name": "river",
  "model": "large"
}
```

The orchestrator:
1. Looks up the model config
2. Updates the Worker's model assignment in the registry
3. Pushes updated registry to all processes
4. Responds with the new endpoint and model name
5. The Worker uses the new model on its next LLM call

### Graceful Shutdown

When the orchestrator receives a shutdown signal:

1. Send a system-role message to each Worker: "The system is shutting down. Call `summary` now."
2. Wait up to **5 minutes** for each Worker to return `WorkerOutput`. Local models may be slow to generate a summary.
3. If a Worker doesn't respond within 5 minutes, force kill and rely on JSONL context for recovery on next start.
4. Shut down adapters.
5. Exit.

### File Operations Through Orchestrator

To avoid race conditions when Workers share workspaces (e.g. actor and spectator), the `read` and `write` tools can route through the orchestrator, which provides file locking:

**POST /file/read**
```json
{
  "worker_name": "river",
  "path": "workspace/river/conversations/discord/general.txt",
  "start_line": 1,
  "end_line": 50
}
```

**POST /file/write**
```json
{
  "worker_name": "river",
  "path": "workspace/river/conversations/discord/general.txt",
  "content": "...",
  "mode": "append"
}
```

The orchestrator acquires a lock on the file path, performs the operation, and releases the lock. This serializes concurrent writes from different Workers.

## Orchestrator HTTP API

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/register` | Process registration |
| POST | `/flash` | Flash message routing between Workers |
| POST | `/model/switch` | Worker requests model change |
| POST | `/worker/output` | Worker sends WorkerOutput on exit |
| GET | `/context/{name}` | Worker requests its JSONL context |
| POST | `/context/{name}` | Worker persists updated context |
| POST | `/file/read` | File read with locking |
| POST | `/file/write` | File write with locking |
| GET | `/registry` | Current process registry |
| GET | `/health` | Orchestrator health |

## Adapter Relationship

(Detailed adapter design in separate document)

Summary:

- **Inbound**: adapter connects to external service, receives all events, forwards to bound Worker
- **Outbound**: adapter receives payloads from bound Worker, forwards to external service
- **No filtering**: the adapter forwards everything. The Worker decides what matters.
- **Per-Worker**: each adapter instance is bound to exactly one Worker. Two Workers on Discord = two adapter processes with different bot tokens.
- **Configured via CLI**: adapters are defined in CLI options, spawned by the orchestrator at startup.
- **Features in registration**: adapters report their supported features during registration. The orchestrator includes these in the Worker's system prompt.

## What The Orchestrator Does NOT Do

- Decide what the agent should work on
- Parse or understand messages
- Filter or prioritize notifications (the Worker decides)
- Understand flash payloads (it just routes them by name)

## Resolved Decisions

1. **Config** — CLI options, no config file for v0.
2. **Port assignment** — child processes bind to port 0 and report their OS-assigned port during registration.
3. **Health checks** — every 60 seconds. Dead after 3 consecutive failures (3 minutes).
4. **Model switching** — `request_model` is a Worker tool. Model assignment tracked in registry.
5. **Worker-to-Worker communication** — flash system. Orchestrator routes by Worker name.
6. **Graceful shutdown** — ask Workers to summarize, wait 5 minutes, then force kill.
7. **Actor/Spectator** — paired via `partner` field. Flash enables high-priority interrupts.
8. **Context serving** — orchestrator serves JSONL context to Workers on startup. Workers persist context back to orchestrator after every mutation.
9. **Respawn policy** — `Done` → permanent sleep. `ContextExhausted` → respawn with summary. `Error` → restart from JSONL.
10. **First notification** — no special "wake up" message. The first real notification from an adapter (or ground) starts the loop.
11. **Feature negotiation** — adapter features included in registration. Orchestrator includes them in Worker system prompt with names and ints.
12. **File locking** — `read` and `write` tools route through orchestrator for file locking when Workers share workspaces.
13. **Adapter tokens** — multiple Workers on the same platform use different bot tokens (one per adapter process).
14. **Auth** — no auth between processes for v0. Localhost only.
15. **Orchestrator crash** — orphans processes. Workers persist JSONL context. Manual recovery.
16. **Logging** — deferred to after initial prototype.
17. **Streaming interrupts** — open question (see below).

## Open Questions

1. **Multiple actor/spectator pairs** — can there be more than one pair? Config supports it, but are there edge cases?
2. **Flash overflow** — should there be a queue limit or backpressure on flashes?
3. **Streaming interrupts** — can the Worker interrupt a streaming LLM response? If a flash arrives mid-stream, should the Worker cancel the stream and re-call the LLM with the flash injected? Or always wait for the stream to complete? This might be important for responsiveness but adds complexity.
4. **Max tool calls per cycle** — should there be a safety limit? Need more information on what happens in practice before deciding. Defer to after initial prototype.

# River Engine v4 — Master Specification

> Consolidated implementation reference for river-engine v4
>
> Authors: Cass, Claude
> Date: 2026-04-02

## Overview

River Engine v4 is a multi-agent system where paired workers (a dyad) collaborate to handle conversations. Each dyad has a left and right worker sharing a workspace. Workers hold batons (actor or spectator) that define their role. The actor handles external communication; the spectator manages memory and reviews the actor's work. A human operator (ground) supervises via a backchannel.

### Architecture Summary

- **Orchestrator:** Process supervisor that spawns workers/adapters, manages registry, handles model config
- **Workers:** LLM-powered agents running think→act loops, one per side of each dyad
- **Adapters:** Platform connectors (Discord, Slack, etc.) that forward events to workers
- **Embed Service:** Vector search service for workspace content
- **Workspace:** Shared filesystem for conversations, memory, and coordination

### Design Principles

- Workers are shells; intelligence lives in the model
- No orchestrator routing for files — workers access workspace directly
- Peer-to-peer communication via flashes (registry provides endpoints)
- OpenAI-compatible message format throughout (zero-conversion LLM calls)
- Registration-based config delivery (no secrets on command line)

---

## Implementation Stages

Six stages in dependency order. Each stage produces working, testable artifacts.

---

### Stage 1: Foundation

**What gets built:**
- **river-snowflake:** Library and binary for 128-bit unique ID generation
- **river-adapter:** Types-only library for adapter communication

**river-snowflake** provides unique IDs for all entities in the system: messages, flashes, moves, moments, embeddings, sessions, tool calls. The ID format encodes timestamp, agent birth, type, and sequence number. The library supports parsing, formatting, and timestamp extraction. The binary runs an HTTP server for ID generation. A generator cache allows embedded generation without the server.

**river-adapter** defines the interface between workers and adapter binaries. It exports the feature system (FeatureId enum for capability registration, OutboundRequest enum for typed requests), inbound event types (InboundEvent, EventMetadata, EventType), and response types. It also exports shared types used across crates: Baton (actor/spectator), Side (left/right), Ground (human operator info), Channel, and Author.

**Dependencies:** None. These crates compile independently.

**Deliverables:**
- river-snowflake library with parsing, formatting, generation
- river-snowflake binary serving HTTP API
- river-adapter library with all shared types
- OpenAPI spec generated from river-adapter types

**Detailed specs:**
- `2026-04-01-snowflake-server-design.md`
- `2026-04-01-adapter-library-design.md`

---

### Stage 2: Services

**What gets built:**
- **river-embed:** Binary for vector search

**river-embed** receives content from workers, chunks it using markdown-aware splitting, generates embeddings via an external model (Ollama or OpenAI-compatible), and stores vectors in sqlite-vec. It provides cursor-based search iteration so workers can fetch results one at a time. On startup, it registers with the orchestrator to receive model config (endpoint, model name, API key, dimensions).

The service is push-based: workers notify it when files in the embeddings/ directory change. No file watching — the worker is the source of truth for what gets embedded.

**Dependencies:** river-snowflake (for chunk IDs)

**Deliverables:**
- river-embed binary with HTTP API (/index, /search, /next, /health)
- sqlite-vec storage with configurable dimensions
- Markdown-aware chunking with line number tracking

**Detailed spec:**
- `2026-04-02-embedding-design.md`

---

### Stage 3: Core

**What gets built:**
- **river-context:** Library for context assembly
- **river-orchestrator:** Binary for process supervision

**river-context** provides a pure function that assembles workspace data into OpenAI-compatible messages. It takes channel contexts (moments, moves, messages, embeddings), flashes, and LLM history, then outputs a flat message list ready for the model. Assembly follows ordering rules: other channels (moments/moves only), last channel (adds embeddings), history block, current channel (full messages). Flashes are interspersed by timestamp. TTL filtering excludes expired flashes and embeddings. Token estimation provides a fast pre-check against budget.

**river-orchestrator** is the process supervisor. It loads JSON config with env var substitution for secrets. On startup, it spawns the embed service (if configured), then both workers for each dyad, then adapters. It maintains a registry of all live processes, pushing updates whenever the registry changes. Workers and adapters register on startup and receive their config in the response. The orchestrator handles model switching requests, coordinates role switching via two-phase commit, manages respawn policy based on worker exit status, and runs health checks.

**Dependencies:**
- river-context depends on river-adapter (Author, Channel types)
- river-orchestrator depends on river-adapter (Baton, Side, Ground, FeatureId)

**Deliverables:**
- river-context library with build_context function
- river-orchestrator binary with HTTP API (/register, /model/switch, /switch_roles, /worker/output, /registry, /health)
- Config loading with env var substitution
- Process spawning and health monitoring
- Registry push mechanism

**Detailed specs:**
- `2026-04-01-context-management-design.md`
- `2026-04-01-orchestrator-design.md`

---

### Stage 4: Runtime

**What gets built:**
- **river-worker:** Binary for the think→act loop

**river-worker** is the agent runtime. On startup, it binds an HTTP server, registers with the orchestrator, and receives its baton, model config, ground, workspace path, and partner endpoint. It loads the role file (actor.md or spectator.md) and identity file into the system prompt.

The main loop builds context via river-context, calls the LLM, executes tool calls in parallel, and persists results to context.jsonl in OpenAI format. Tool results are appended after each execution for crash recovery.

The worker handles notifications from adapters (POST /notify), flashes from partner (POST /flash), and registry updates (POST /registry). It manages conversation files using a hybrid append-only format with periodic compaction.

Available tools: read, write, delete, bash, speak, adapter, switch_channel, sleep, watch, summary, create_move, create_moment, create_flash, request_model, switch_roles, search_embeddings, next_embedding. All tools return standardized errors via ToolError enum.

Role switching uses orchestrator-mediated two-phase commit: worker calls POST /switch_roles on orchestrator, which sends prepare_switch to both workers, waits for ready responses, then sends commit_switch to both.

Workers exit via the summary tool (Done status) or when context reaches 95% (ContextExhausted status). The orchestrator handles respawn based on exit status.

**Dependencies:** river-adapter, river-context, river-snowflake

**Deliverables:**
- river-worker binary with HTTP API (/notify, /flash, /registry, /prepare_switch, /commit_switch, /health)
- Full tool implementations
- Conversation file management with compaction
- Context persistence in OpenAI format
- LLM client for OpenAI-compatible endpoints

**Detailed spec:**
- `2026-04-01-worker-design.md`

---

### Stage 5: Adapters

**What gets built:**
- **river-discord:** First adapter implementation

**river-discord** connects to the Discord gateway and forwards events to the worker. On startup, it binds an HTTP server, registers with the orchestrator (reporting its dyad, type, and supported features), and receives config (token, guild_id) plus the worker endpoint in the response. It then connects to Discord and begins forwarding events.

Inbound events (message create, reactions, typing, etc.) are posted to the worker's /notify endpoint as InboundEvent structs. Outbound requests come via POST /execute with OutboundRequest variants (SendMessage, EditMessage, DeleteMessage, AddReaction, etc.).

The adapter reports its supported features during registration. The orchestrator validates that required features (SendMessage, ReceiveMessage) are present.

**Dependencies:** river-adapter (types only)

**Deliverables:**
- river-discord binary with HTTP API (/execute, /health)
- Discord gateway connection and event handling
- Implementation of core OutboundRequest variants
- Feature reporting during registration

**Detailed spec:**
- `2026-04-01-adapter-library-design.md` (adapter binary CLI and HTTP API sections)

---

### Stage 6: Integration

**What gets built:**
- Workspace structure template
- Role files (actor.md, spectator.md)
- Identity file templates
- Backchannel adapter
- End-to-end testing

**Workspace structure** defines the directory layout for a dyad workspace: roles/ for shared role files, left/ and right/ for per-worker identity and context, shared/ for reference material, conversations/ for chat history by adapter/channel, moves/ and moments/ for compressed history, embeddings/ for searchable content, memory/ for long-term patterns, notes/ for drafts, artifacts/ for generated files.

**Role files** provide behavioral guidance for each baton. Actor handles external communication, reads compressed history, monitors flashes. Spectator manages memory (creates moves/moments), reviews actor's work, flashes guidance. Both can communicate with ground via backchannel. No tool restrictions — guidance only.

**Identity files** define personality and characteristics for each worker. Left and right workers maintain distinct identities even when switching roles.

**Backchannel adapter** is a simple adapter type for actor/spectator/ground coordination. It appears in the registry like other adapters but is recognized as the internal coordination channel.

**End-to-end testing** validates the complete system: orchestrator spawning all processes, workers registering and loading context, adapters forwarding events, workers responding via speak tool, spectator creating moves/moments, role switching, context exhaustion and respawn.

**Deliverables:**
- Workspace directory template
- actor.md and spectator.md role files
- Identity file templates
- Backchannel adapter implementation
- Integration test suite

**Detailed spec:**
- `2026-04-02-role-files-design.md` (on design branch)

---

## System Interactions

### Startup Sequence

1. Orchestrator loads config, resolves env vars, binds HTTP server
2. Orchestrator spawns embed service (if configured)
3. Embed service registers, receives model config, initializes sqlite-vec
4. For each dyad: orchestrator spawns left worker, right worker
5. Workers register, receive baton/config/ground/workspace, load role and identity
6. For each adapter in dyad: orchestrator spawns adapter
7. Adapters register, receive config and worker endpoint, connect to platform
8. Orchestrator pushes registry to all processes
9. Actor waits for first /notify, spectator waits for first /flash
10. System is live

### Runtime Communication

- **Adapter → Worker:** POST /notify with InboundEvent (fire-and-forget)
- **Worker → Adapter:** POST /execute with OutboundRequest, receive OutboundResponse
- **Worker → Worker:** POST /flash with Flash struct (peer-to-peer via registry)
- **Worker → Embed:** POST /search, POST /next, POST /index, DELETE /source
- **Worker → Orchestrator:** POST /model/switch, POST /switch_roles, POST /worker/output
- **Orchestrator → All:** POST /registry when registry changes
- **Orchestrator → Worker:** POST /prepare_switch, POST /commit_switch (during role switch)

### Role Switching Protocol

1. Worker calls POST /switch_roles on orchestrator with dyad and side
2. Orchestrator acquires dyad lock (prevents concurrent switches)
3. Orchestrator sends POST /prepare_switch to both workers
4. Workers check they're not mid-operation, respond ready or busy
5. If both ready: orchestrator sends POST /commit_switch to both
6. Workers swap batons, reload role files
7. Orchestrator updates registry, pushes to all, releases lock
8. Original caller receives success response

### Worker Exit and Respawn

- **Done (wake_after_minutes: None):** Respawn immediately with start_sleeping: true
- **Done (wake_after_minutes: Some(n)):** Wait n minutes, respawn with initial_message set to summary
- **ContextExhausted:** Respawn immediately with initial_message set to summary
- **Error:** Respawn immediately, worker loads from workspace JSONL

---

## Workspace Structure

```
workspace/
├── roles/
│   ├── actor.md              # actor role behavior
│   └── spectator.md          # spectator role behavior
├── left/
│   ├── identity.md           # left worker identity
│   └── context.jsonl         # left worker context history
├── right/
│   ├── identity.md           # right worker identity
│   └── context.jsonl         # right worker context history
├── shared/
│   └── reference.md          # shared reference material
├── conversations/
│   └── {adapter}/
│       ├── {guild_id}-{guild_name}/
│       │   └── {channel_id}-{channel_name}.txt
│       └── dm/
│           └── {user_id}-{user_name}.txt
├── moves/
│   └── {adapter}_{channel_id}.jsonl
├── moments/
│   └── {adapter}_{channel_id}.jsonl
├── embeddings/               # files to embed
├── memory/                   # long-term memory
├── notes/                    # personal notes
└── artifacts/                # generated files
```

**Ownership conventions:**
- roles/: Both (read-only at runtime)
- left/, right/: Respective worker
- shared/: Both (coordinate via git)
- conversations/: Actor
- moves/, moments/: Spectator
- embeddings/: Both (worker notifies embed service on write)

---

## Glossary

**Baton:** The role a worker currently holds — actor or spectator. Workers can switch batons via the switch_roles tool.

**Dyad:** A pair of workers (left and right) sharing a workspace. Each dyad serves one ground.

**Side:** A worker's fixed position in the dyad — left or right. Unlike baton, side never changes.

**Ground:** The human operator supervising the dyad. Accessible via backchannel.

**Flash:** A short-lived message between workers. High priority, injected before next LLM call. Has TTL.

**Move:** A summary of a range of messages in a conversation. Created by spectator to compress history.

**Moment:** A summary of a range of moves. Higher-level compression created by spectator.

**Channel:** A communication endpoint identified by adapter type and channel ID.

**Adapter:** A platform connector that translates between platform events and river types.

**Registry:** The orchestrator's list of all live processes with their endpoints and metadata.

**Context exhaustion:** When a worker's token count exceeds 95% of limit, forcing summarization and respawn.

**Backchannel:** Special adapter for actor/spectator/ground coordination, separate from public channels.

---

## Related Documents

- `2026-04-01-orchestrator-design.md` — Orchestrator details
- `2026-04-01-worker-design.md` — Worker details
- `2026-04-01-adapter-library-design.md` — Adapter types and interface
- `2026-04-01-context-management-design.md` — Context assembly
- `2026-04-01-snowflake-server-design.md` — ID generation
- `2026-04-02-embedding-design.md` — Vector search
- `2026-04-02-role-files-design.md` — Role file content (design branch)
- `2026-04-02-spec-review.md` — Cross-spec review results

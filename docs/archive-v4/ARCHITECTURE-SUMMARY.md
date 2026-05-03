# River Engine Architecture Summary

> Comprehensive summary for context continuity. Last updated: 2026-04-05.

## Project Overview

River Engine is a multi-agent system where paired workers (a dyad) collaborate to handle conversations. It implements the I/You architecture — a philosophical response to the Chinese Room problem that adds internal structure via a witness (spectator) that evaluates the actor's genuineness.

## Core Philosophy

### Design Principles (from DESIGN-PHILOSOPHY.md)

1. **Pure Core / Effectful Shell** — Side effects at edges, logic in the middle
2. **Composition Over Monoliths** — Small services with HTTP APIs
3. **Agent-First Design** — Tools, memory, autonomy, identity
4. **Priority-Based Resource Management** — Interactive > Scheduled > Background
5. **Workspace = Identity** — One workspace directory = one agent instance
6. **Semantic Memory as First-Class** — Conversations searchable by meaning
7. **Test What Matters** — Strong tests for behavior, formal verification for invariants
8. **Documentation Lives With Code** — Design docs in the repo
9. **Open Source From Day One** — Build in public

### The I/You Architecture (from research docs)

The Chinese Room and Turing Test both assume a monolithic system. River has two perspectives:

- **Agent (I)** — Produces outputs, thinks, acts, writes notes
- **Spectator (You)** — Observes, compresses, curates, evaluates honestly

They can disagree. That internal conflict is information a monolithic system can't have. The spectator isn't checking accuracy — it's checking honesty.

**The Triad:**
| Vertex | Role | Question |
|--------|------|----------|
| Actor (I) | Does | "What should I say?" |
| Spectator (You) | Observes | "Was that genuine?" |
| Ground (Human) | Reality-checks | "Does this feel alive?" |

**Success Criteria:**
- Functional (A): Does the system work? — necessary but not sufficient
- Behavioral (B): Does it perform correctly? — evidence, but can be faked
- Qualitative (C): Does it feel genuine? — the actual goal, requires the witness

## System Components

### 1. Orchestrator (river-orchestrator)

Process supervisor and message router. Six responsibilities:

1. **Process supervisor** — spawns, monitors, restarts children
2. **Registry** — tracks all live processes and endpoints
3. **Config** — loads JSON config with env var substitution
4. **Model manager** — assigns LLM models to workers
5. **Flash router** — delivers flash messages between workers
6. **Context server** — serves JSONL context on startup/restart

**HTTP API:**
- POST `/register` — process registration
- POST `/flash` — flash message routing
- POST `/model/switch` — worker requests model change
- POST `/switch_roles` — role switch coordination
- POST `/worker/output` — worker sends exit status
- GET `/context/{name}` — worker requests context
- POST `/context/{name}` — worker persists context
- GET `/registry` — current process registry
- GET `/health` — orchestrator health

**Startup Sequence:**
1. Load config, resolve env vars
2. Spawn embed service (if configured)
3. For each dyad: spawn left worker, right worker
4. Workers register, receive baton/config/ground/workspace
5. Spawn adapters for each dyad
6. Adapters register, receive config and worker endpoint
7. Push registry to all processes
8. Actor waits for first /notify, spectator waits for first /flash

### 2. Worker (river-worker)

The agent runtime. A shell that creates conditions for the model to think and act.

**Input:**
- System prompt (built from workspace files)
- Messages (conversation with the world)
- Model endpoint and name
- Orchestrator endpoint
- Worker name, ground, workspace path

**The Loop:**
1. Call LLM with system prompt + messages
2. LLM responds with tool calls → execute in parallel, append results
3. LLM responds with text → inject status message, continue
4. New notification while active → batch for next status
5. New notification while sleeping on watched channel → wake

**Exit Conditions:**
- `Done` — model called summary tool
- `ContextExhausted` — hit 95%, forced summary
- `Error` — something broke

**Tools (11 total):**
| Tool | Purpose |
|------|---------|
| read | Read file from workspace |
| write | Write file to workspace |
| delete | Delete file from workspace |
| bash | Execute shell commands |
| speak | Send to current channel |
| adapter | Send to any adapter |
| switch_channel | Change current channel |
| sleep | Pause loop, wait for timer or notification |
| watch | Manage wake channels |
| summary | Stop execution, return handoff |
| request_model | Ask orchestrator for model switch |

Plus spectator tools: create_move, create_moment, create_flash, search_embeddings, next_embedding

**HTTP API:**
- POST `/notify` — inbound notifications from adapters
- POST `/registry` — registry updates from orchestrator
- POST `/flash` — flash messages from partner
- POST `/prepare_switch`, `/commit_switch`, `/abort_switch` — role switching
- GET `/health` — health check

**Context Pressure:**
- 80%: inject warning message
- 95%: hard stop, force summary

### 3. Adapter (river-adapter + river-discord)

Dumb pipe connecting to external services. Library crate exports trait, specific adapters are separate binaries.

**Feature System:**
```
Core messaging (0-9): SendMessage=0, ReceiveMessage=1
Message operations (10-19): EditMessage=10, DeleteMessage=11, ReadHistory=12, ...
Reactions (20-29): AddReaction=20, RemoveReaction=21, ...
Typing (40-49): TypingIndicator=40
Threads (50-59): CreateThread=50, ThreadEvents=51
Situational awareness (100-109): VoiceStateEvents=100, PresenceEvents=101, ...
Connection (900-909): ConnectionEvents=900
```

**Inbound Flow:**
External service → Adapter receives event → Normalize to InboundEvent → POST to worker /notify

**Outbound Flow:**
Worker calls speak tool → POST to adapter /request → Adapter calls platform API → Return response

**HTTP API:**
- POST `/request` — handle outbound requests
- GET `/health` — health check
- POST `/registry` — registry updates

### 4. Context Assembly (river-context)

Pure function that assembles workspace data into OpenAI-compatible messages.

**Input:** ChannelContext (moments, moves, messages, embeddings, inbox), flashes, history, max_tokens, now

**Output:** Flat message list ready for LLM

**Assembly Order:**
1. Other channels (moments/moves only)
2. Last channel (adds embeddings)
3. History block (from context.jsonl)
4. Current channel (full messages + inbox items)

**Compression Hierarchy:**
- Moments `[~]` summarize ranges of moves
- Moves `[^]` summarize ranges of messages
- Messages are raw, uncompressed

All sorted by timestamp. Flashes interspersed by timestamp.

### 5. Snowflake IDs (river-snowflake)

128-bit unique identifiers:

| Bits | Field | Description |
|------|-------|-------------|
| 64 | Timestamp | Microseconds since agent birth |
| 36 | Agent Birth | yyyymmddhhmmss packed |
| 8 | Type | Entity type (Message=0x01, Embedding=0x02, Session=0x03, etc.) |
| 20 | Sequence | Counter for same-microsecond IDs |

**Server Routes:**
- GET `/id/{type}?birth={birth}` — generate single ID
- POST `/ids` — batch generation (up to 10,000)
- GET `/health` — health check

Can also be used as library with embedded GeneratorCache.

### 6. Embedding Service (river-embed)

Vector search service for workspace content.

- Receives content from workers, chunks using markdown-aware splitting
- Generates embeddings via external model (Ollama or OpenAI-compatible)
- Stores vectors in sqlite-vec
- Provides cursor-based search iteration

**HTTP API:**
- POST `/index` — index content
- POST `/search` — search vectors
- POST `/next` — get next result from cursor
- DELETE `/source` — remove indexed content
- GET `/health` — health check

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
├── conversations/            # chat history by adapter/channel
│   └── {adapter}/
│       └── {channel_id}-{channel_name}.txt
├── moves/                    # message summaries [^]
│   └── {adapter}_{channel_id}.jsonl
├── moments/                  # move summaries [~]
│   └── {adapter}_{channel_id}.jsonl
├── inbox/                    # tool results (NEW)
│   └── {adapter}_{channel}_{timestamp}_{tool}.json
├── embeddings/               # files to embed
├── memory/                   # long-term memory
├── notes/                    # personal notes
└── artifacts/                # generated files
```

**Ownership:**
- roles/: Both (read-only at runtime)
- left/, right/: Respective worker
- shared/: Both (coordinate via git)
- conversations/: Actor
- moves/, moments/: Spectator
- embeddings/: Both

## Context Architecture (Implemented 2026-04-05)

### Problem
context.jsonl grew too fast by storing everything: prompts, tool calls, tool results, system messages.

### Solution
Store only what the LLM produces. Assemble context fresh from workspace data.

### Storage

**context.jsonl — Stream of Consciousness**
Stores only:
- Assistant messages (LLM outputs, including tool_calls)
- System warnings (context pressure warnings)

Does NOT store:
- Role/identity prompts (re-rendered each assembly)
- Tool results (live in inbox or are ephemeral)
- Rendered workspace data (moments, moves, messages)

**workspace/inbox/ — Tool Results**
Timestamped JSON files: `{adapter}_{channel_id}_{timestamp}_{tool}.json`

Tools that write to inbox:
- read_history (records message range)
- create_move
- create_moment

Tools that don't write to inbox:
- speak (effect shows in conversation file)
- switch_channel (triggers rebuild)
- sleep, flash (no persistent result)

### Assembly

Happens on:
- Channel switch (switch_channel tool)
- Worker respawn (after force_summary or crash)

Order:
1. Role — from roles/{baton}.md
2. Identity — from shared/identity.md
3. Channel blocks — one per watched channel, current last
4. Stream of consciousness — LLM outputs from context.jsonl
5. New messages — current channel's messages since last assembly

Current channel includes: moments, moves, inbox items (full detail)
Other channels include: moments, moves only

### Persistence Rules

| What | Persisted to context.jsonl |
|------|---------------------------|
| Assistant messages | Yes |
| Context warnings ("Context at X%") | Yes |
| Tool results | No (go to inbox/) |
| User messages | No (in conversations/) |
| System prompts | No (re-rendered) |
| Notifications | No (ephemeral) |

## Key Concepts Glossary

| Term | Definition |
|------|------------|
| **Baton** | Role a worker holds — actor or spectator. Can switch. |
| **Dyad** | Pair of workers (left/right) sharing a workspace |
| **Side** | Worker's fixed position — left or right. Never changes. |
| **Ground** | Human operator supervising the dyad |
| **Flash** | Short-lived high-priority message between workers. Has TTL. |
| **Move** | Summary of a range of messages `[^]`. Created by spectator. |
| **Moment** | Summary of a range of moves `[~]`. Created by spectator. |
| **Channel** | Communication endpoint (adapter type + channel ID) |
| **Registry** | Orchestrator's list of all live processes |
| **Context exhaustion** | Token count > 95%, forcing summarization and respawn |
| **Backchannel** | Special adapter for actor/spectator/ground coordination |

## Implementation Status

### Completed (river-engine v4)

- river-snowflake: ID generation library and server
- river-adapter: Types library for adapter communication
- river-protocol: Shared types (Author, Channel, Baton, Side, Ground)
- river-context: Context assembly library
- river-orchestrator: Process supervisor binary
- river-worker: Agent runtime binary
- river-discord: Discord adapter binary
- river-embed: Embedding service binary

### Recent Changes (2026-04-05)

Context architecture redesign implemented:
- InboxItem type added to river-context
- format_inbox_item function for display
- inbox field added to ChannelContext
- Inbox items included in timeline assembly
- inbox.rs module in river-worker for read/write
- Inbox writes added to read_history, create_move, create_moment tools
- Selective persistence (should_persist function)
- workspace_loader loads inbox items

8 commits implementing the feature:
```
6a3fdc5 fix(river-context): add inbox field to doctest example
f8191e0 feat(river-worker): add selective persistence for context.jsonl
1ea9d72 feat(river-worker): write inbox items on read_history, create_move, create_moment
d45096c feat(river-worker): add inbox module for tool result storage
6ba2ed3 feat(river-context): include inbox items in timeline for current channel
7c506c8 feat(river-context): add inbox field to ChannelContext
dd8b380 feat(river-context): add format_inbox_item for tool result display
05c5728 feat(river-context): add InboxItem type for tool results
```

## File Locations

### Core Logic
- `crates/river-orchestrator/src/main.rs` — startup, supervision
- `crates/river-orchestrator/src/http.rs` — registration handlers
- `crates/river-orchestrator/src/config.rs` — configuration
- `crates/river-worker/src/main.rs` — worker startup
- `crates/river-worker/src/worker_loop.rs` — main think/act loop
- `crates/river-worker/src/tools.rs` — tool execution
- `crates/river-worker/src/persistence.rs` — context persistence
- `crates/river-worker/src/inbox.rs` — inbox utilities
- `crates/river-worker/src/workspace_loader.rs` — loads workspace data

### Protocol Types
- `crates/river-protocol/src/lib.rs` — Author, Channel types
- `crates/river-adapter/src/lib.rs` — Baton, Side, Ground, features
- `crates/river-context/src/lib.rs` — context assembly exports

### Documentation
- `docs/DESIGN-PHILOSOPHY.md` — core principles
- `docs/ORCHESTRATOR-DESIGN.md` — orchestrator details
- `docs/ADAPTER-DESIGN.md` — adapter interface
- `docs/WORKER-DESIGN.md` — worker details
- `docs/GAP-ANALYSIS.md` — resolved decisions and open questions
- `docs/research/` — philosophy and architecture brainstorms
- `docs/superpowers/specs/` — current implementation specs
- `docs/superpowers/plans/` — implementation plans
- `docs/archive/` — historical specs and plans

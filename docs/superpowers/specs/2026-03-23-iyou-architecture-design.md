# I/You Architecture — Master Design Spec

> Single source of truth for the River Engine restructure.
> Synthesizes: adapter-framework-design, context-assembly-design, gateway-restructure-meta-plan.
>
> Date: 2026-03-23
> Authors: Cass, Claude, Will

---

## 1. Executive Summary
		
River Engine evolves from a monolithic gateway with a single agent loop into a **coordinator + peer tasks** architecture. Two cognitive perspectives — **Agent (I)** and **Spectator (You)** — run as concurrent tasks within the gateway, communicating via an event bus. Memory moves from direct SQLite embeddings to a **zettelkasten** (filesystem + sqlite-vec). A new **adapter framework** decouples platform integrations from the core.

**Three simultaneous shifts:**
1. **Structural:** Monolithic loop → Coordinator + Agent task + Spectator task
2. **Memory:** Direct embeddings → Zettelkasten with hot/warm/cold context assembly
3. **Integration:** Hardcoded Discord → Generic adapter framework

---

## 2. Architecture Overview

```
                    ┌──────────────────────────────────────────┐
                    │               Coordinator                 │
                    │         (event bus, lifecycle)             │
                    ├────────────────────┬─────────────────────┤
                    │                    │                      │
                    ▼                    ▼                      │
             ┌─────────────┐    ┌──────────────┐               │
             │  Agent (I)  │    │ Spectator(You)│               │
             │             │◄───│              │               │
             │ think/act   │    │ observe/     │               │
             │ context asm │    │ compress/    │               │
             │             │    │ curate       │               │
             └──────┬──────┘    └──────┬───────┘               │
                    │                  │                        │
                    ▼                  ▼                        │
             ┌─────────────────────────────────┐               │
             │          Shared State            │               │
             │  embeddings/  sqlite-vec  git    │               │
             └─────────────────────────────────┘               │
                    │                                           │
                    │         ┌──────────────┐                  │
                    │         │ river-adapter │                  │
                    │         │ (types/trait) │                  │
                    │         └──────┬───────┘                  │
                    │                │                           │
                    ▼                ▼                           │
             ┌──────────┐   ┌──────────────┐                   │
             │ river-db  │   │river-discord │                   │
             │ river-tools│   │ (reference)  │                   │
             └──────────┘   └──────────────┘                   │
                    └──────────────────────────────────────────┘
```

---

## 3. Crate Structure (Target)

### New Crates

| Crate | Purpose | Source |
|-------|---------|--------|
| `river-tools` | Tool trait, registry, executor, all tool implementations | Extracted from `river-gateway/src/tools/` |
| `river-db` | Database layer, migrations, schemas | Extracted from `river-gateway/src/db/` |
| `river-adapter` | Adapter types, trait, feature flags, registration | New |

### Modified Crates

| Crate | Changes |
|-------|---------|
| `river-gateway` | Restructured: coordinator + agent + spectator + embeddings + flash |
| `river-discord` | Uses `river-adapter` types, self-registers with gateway |
| `river-core` | Minimal additions (new error variants, event types) |

### Dependency Graph

```
river-core
  ↑
river-db ←── river-tools ←── river-adapter
  ↑              ↑                ↑
  └──── river-gateway ────────────┘
              ↑
         river-discord
         river-orchestrator
```

---

## 4. Gateway Internal Structure (Target)

```
river-gateway/src/
├── coordinator/        # NEW: Event bus, peer task management
│   ├── mod.rs          # Coordinator struct, run loop
│   ├── events.rs       # Agent↔Spectator event types
│   └── bus.rs          # Event dispatch, buffering
│
├── agent/              # NEW: Agent (I) task
│   ├── mod.rs          # Agent task, turn cycle (wake/think/act/settle)
│   ├── context.rs      # Hot/warm/cold assembly (replaces loop/context.rs)
│   └── tools.rs        # Tool dispatch (uses river-tools registry)
│
├── spectator/          # NEW: Spectator (You) task
│   ├── mod.rs          # Spectator task, observation cycle
│   ├── compress.rs     # Moves, moments generation
│   ├── curate.rs       # Flash selection, vector search
│   └── room.rs         # Room notes, witness protocol
│
├── embeddings/         # NEW: Zettelkasten sync layer
│   ├── mod.rs          # Public API
│   ├── sync.rs         # File watcher, hash diffing
│   ├── chunk.rs        # Chunking strategies
│   ├── note.rs         # Note format, frontmatter parsing
│   └── store.rs        # sqlite-vec operations
│
├── flash/              # NEW: Flash queue
│   ├── mod.rs          # Flash struct, queue
│   └── ttl.rs          # TTL tracking, expiry
│
├── api/                # MODIFIED: New routes for adapters, spectator
├── conversations/      # KEEP: Conversation persistence
├── inbox/              # KEEP: Inbox reader/writer
├── session/            # KEEP: Session management
├── git.rs              # KEEP: Git operations
├── heartbeat.rs        # KEEP: Heartbeat scheduling
├── metrics.rs          # KEEP: Agent metrics
├── logging.rs          # KEEP: Structured logging
├── policy.rs           # KEEP: Health policy
├── preferences.rs      # KEEP: User preferences
├── watchdog.rs         # KEEP: Watchdog timer
│
├── memory/             # DEPRECATED: Replaced by embeddings/
├── redis/              # KEEP (for now): Ephemeral memory
├── subagent/           # EVOLVE: May merge with coordinator peer pattern
├── loop/               # DEPRECATED: Replaced by coordinator + agent
└── state.rs            # MODIFIED: Updated for new architecture
```

### What Survives, Moves, or Dies

| Current Module | Lines | Fate |
|----------------|-------|------|
| `tools/` (all) | ~4,100 | **Move** → `river-tools` crate |
| `db/` | ~878 | **Move** → `river-db` crate |
| `loop/mod.rs` | 1,051 | **Rewrite** → `agent/mod.rs` + `coordinator/mod.rs` |
| `loop/context.rs` | 494 | **Rewrite** → `agent/context.rs` (hot/warm/cold layers) |
| `loop/model.rs` | 776 | **Keep** → stays as ModelClient (used by agent + spectator) |
| `loop/state.rs` | 174 | **Evolve** → `coordinator/events.rs` |
| `loop/queue.rs` | 262 | **Keep** → MessageQueue still used |
| `loop/persistence.rs` | 299 | **Keep** → context file persistence |
| `memory/` | 344 | **Replace** → `embeddings/` |
| `redis/` | 792 | **Keep** → ephemeral memory still useful |
| `subagent/` | 1,326 | **Keep** → parent-child pattern for task workers |
| `conversations/` | 1,337 | **Keep** |
| `inbox/` | 633 | **Keep** |
| `api/` | 645 | **Modify** → add adapter registration, spectator routes |
| `policy.rs` | 948 | **Keep** |
| `state.rs` | 153 | **Modify** → updated for coordinator |
| `server.rs` | 385 | **Modify** → creates coordinator instead of loop |

---

## 5. The I/You Architecture

### Agent (I)

The agent is the acting self. It:
- Wakes on events (messages, heartbeat)
- Assembles context from hot/warm/cold layers
- Thinks (model call)
- Acts (tool calls, writes notes)
- Settles (commits, prepares for sleep)
- Emits events: `TurnStarted`, `TurnComplete`, `NoteWritten`, `ChannelSwitched`, `ContextPressure`

### Spectator (You)

The spectator is the observing self. It:
- Watches agent turn transcripts
- Compresses old messages into **moves** (structural) and **moments** (arcs)
- Curates: searches vectors, selects what to surface as **flashes**
- Writes **room notes** (witness observations)
- Never uses "I" — speaks from "You" perspective
- Prefers shaping context over speaking directly

### Communication

Via events on the coordinator's event bus:

**Agent → Spectator:**
| Event | Payload|
|-------|---------|
| `TurnStarted` | channel, context summary |
| `TurnComplete` | transcript |
| `NoteWritten` | path |
| `ChannelSwitched` | old, new |
| `ContextPressure` | usage % |

**Spectator → Agent:**
| Event | Payload |
|-------|---------|
| `Flash` | content, source, ttl |
| `Observation` | content |
| `Warning` | content |
| `MovesUpdated` | channel |
| `MomentCreated` | summary |

---

## 6. Context Assembly

Each turn, the agent builds its context window from layers:

```
┌─────────────────────────────────────────────────┐
│ SYSTEM (~2-4K tokens)                            │
│ IDENTITY.md, RULES.md, time, tool schemas        │
├─────────────────────────────────────────────────┤
│ WARM: Moves (~2-4K) — this channel's arc         │
├─────────────────────────────────────────────────┤
│ WARM: Flashes (~1-2K) — spectator-curated notes  │
├─────────────────────────────────────────────────┤
│ WARM: Retrieved (~2-8K) — semantic search hits    │
├─────────────────────────────────────────────────┤
│ HOT: Recent messages (~8K) — full resolution      │
├─────────────────────────────────────────────────┤
│ Reserved for output (~4-8K)                       │
└─────────────────────────────────────────────────┘
```

**Channel switching:** System persists, moves switch, flashes persist (global), retrieved refreshes, hot switches.

---

## 7. Memory: The Zettelkasten

### Filesystem Layout

```
workspace/embeddings/           # Source of truth (git-tracked)
├── notes/                      # Agent/spectator-written notes
├── moves/                      # Per-channel conversation arcs
│   ├── discord-general.md
│   └── discord-dm-cass.md
├── moments/                    # Compressed arcs
└── room-notes/                 # Spectator witness observations
    └── 2026-03-23-session.md

data/vectors.db                 # Derived state (sqlite-vec)
```

### Note Format

```markdown
---
id: 0x01a2b3c4d5e6f7...
created: 2026-03-23T14:32:07Z
author: agent
type: note
tags: [css, z-index, debugging]
---

# z-index hierarchy
Content here. Links via [[wiki-style]].
```

### Sync Service

File changed → hash → compare → re-chunk → re-embed → update vectors.db.

Delete vectors.db, run sync, get identical state. Filesystem is truth.

### Chunking

| Type | Strategy |
|------|----------|
| Notes | Whole file (small) or split on headers |
| Moves | Per-move |
| Moments | Whole moment |
| Room notes | Per-turn block |
| Large docs | ~400 token chunks with overlap |

---

## 8. Flash Queue

```rust
struct Flash {
    id: Snowflake,
    content: String,           // Full note text
    source: String,            // Path in embeddings/
    ttl: FlashTTL,
}

enum FlashTTL {
    Turns(u8),                 // Expires after N turns
    Duration(Duration),        // Expires after time
}
```

Global (not channel-specific). Spectator pushes, context assembler reads. TTL decrements per turn or clock tick. Duplicates refresh TTL.

---

## 9. Adapter Framework

### `river-adapter` Crate

Provides shared types for all adapters:
- `IncomingEvent` (adapter → gateway)
- `SendRequest` / `SendResponse` (gateway → adapter)
- `AdapterInfo` with `HashSet<Feature>` capabilities
- `Adapter` trait (for gateway-side abstraction)
- OpenAPI generation from Rust types via `utoipa`

### Registration Flow

1. Adapter starts
2. `POST gateway/adapters/register` with `AdapterInfo`
3. Gateway stores in `AdapterRegistry`
4. Adapter receives events, gateway calls `/send`

### Feature Flags

```rust
enum Feature {
    ReadHistory, Reactions, Threads, Attachments, Embeds,
    TypingIndicator, EditMessage, DeleteMessage, Custom(String),
}
```

Gateway checks features before operations. Unsupported → clean error.

### Discord Reference Implementation

`river-discord` refactored to use `river-adapter` types. Maps Discord events to `IncomingEvent` with native metadata preserved in `serde_json::Value`.

---

## 10. Open Questions

| # | Question | Context | Impact |
|---|----------|---------|--------|
| 1 | **Spectator model choice** | Spec says llama-3-8b. Need to test if it can do structural compression well enough. | Phase 6 |
| 2 | **Subagent unification** | Keep parent-child for task workers, or unify with coordinator peer pattern? | Phase 4-5 |
| 3 | **Memory migration** | How to migrate existing SQLite embeddings to zettelkasten format? | Phase 1 |
| 4 | **Redis fate** | Redis ephemeral memory overlaps with flash queue. Keep both? | Phase 2-3 |
| 5 | **API route survival** | Which existing routes survive restructure? New routes for spectator? | Phase 4 |
| 6 | **Git authorship** | Both agent and spectator commit. Separate git identities? | Phase 5-6 |
| 7 | **Tool context dependencies** | Tools like `communication`, `scheduling`, `subagent` depend on gateway internals (AppState, LoopEvent). How to clean the interface when extracting to river-tools? | Phase 0 |
| 8 | **Embedding model** | Which embedding model? Currently uses OpenAI-compatible API. Local model via orchestrator? | Phase 1 |

---

## 11. Conflict Resolutions

### Adapter types vs. existing communication tools

The existing `AdapterRegistry` in `tools/communication.rs` is a simple HashMap of URLs. The new `river-adapter` crate defines a richer `AdapterInfo` with features, health, and trait-based abstraction. **Resolution:** Phase 0 extracts the tool, adapter framework replaces the registry later. The extracted tool keeps the simple interface; the adapter framework plan migrates it.

### Memory system overlap

Current: SQLite direct embeddings (`memory/`) + Redis ephemeral (`redis/`).
New: Zettelkasten + sqlite-vec (`embeddings/`) + flash queue.

**Resolution:** Build new alongside old. Phase 1 adds embeddings layer. Phase 3 context assembly uses both old and new. Once I/You architecture is complete, old memory system can be deprecated. Redis ephemeral may remain useful for cross-process coordination.

### Subagent vs. peer tasks

Current subagent system is parent-child (spawn, wait, kill). New architecture has peer tasks (agent + spectator). **Resolution:** Keep both patterns. Subagents remain for task workers spawned by the agent. The coordinator manages peer tasks (agent, spectator). Different lifecycles, different purposes.

---

## 12. Implementation Phases

| Phase | Name | Goal | Depends On |
|-------|------|------|------------|
| 0 | Extract Crates | `river-tools` + `river-db` as standalone crates | — |
| 1 | Embeddings | Zettelkasten sync + sqlite-vec | Phase 0 |
| 2 | Flash Queue | TTL-based memory surfacing | Phase 0 |
| 3 | Context Assembly | Hot/warm/cold layer builder | Phase 1, 2 |
| 4 | Coordinator | Event bus + peer task lifecycle | Phase 3 |
| 5 | Agent Task | Rewrite agent as peer task | Phase 4 |
| 6 | Spectator Task | The witness | Phase 5 |
| 7 | Integration | Everything working together | Phase 6 |
| — | Adapter Framework | Generic adapter types + Discord migration | Independent |

Phases 1-2 can be somewhat parallel. Phases 4-7 are strictly sequential.

---

## 13. Success Criteria

### A. Functional
- Sync service embeds files, vectors appear
- Context assembles from hot/warm/cold layers
- Spectator runs, events flow, flashes appear
- Moves and moments generate from conversations
- Git tracks changes with correct authorship
- 100+ turns without crash

### B. Behavioral
- Mention topic → related notes surface
- Reference from 200 turns ago → retrievable
- Moves capture conversation structure
- Flashes are timely and relevant
- Channel switching works correctly

### C. Qualitative (the actual goal)
- Compression is honest (includes failures, tensions)
- Retrieval feels relevant
- Agent maintains coherence over long sessions
- Spectator voice: terse, structural, third-person
- Room notes provide useful witness perspective

---

## 14. Related Documents

- `docs/specs/adapter-framework-design.md` — Adapter framework detail
- `docs/specs/context-assembly-design.md` — I/You architecture detail
- `docs/specs/gateway-restructure-meta-plan.md` — Phase roadmap
- `docs/superpowers/specs/2026-03-16-river-engine-design.md` — Original engine design
- `docs/research/two-people-in-the-room.md` — Philosophical basis

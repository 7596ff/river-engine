# Gateway Restructure: Meta-Plan

> Implementation roadmap for I/You architecture
>
> 2026-03-23

## Overview

Restructure river-gateway from monolithic agent loop to coordinator + peer tasks architecture. Extract reusable components, rewrite core.

## Current State

```
river-gateway/src/
├── loop/           # 1000+ lines, monolithic, one-agent assumption
├── memory/         # Direct embedding, not zettelkasten
├── tools/          # 40+ tools, reusable
├── db/             # Solid, keep
├── subagent/       # Parent-child pattern (not peer)
├── api/            # Keep patterns
└── ...             # Ancillary (metrics, logging, etc.)
```

## Target State

```
river-gateway/src/
├── coordinator/    # Event bus, peer task management
│   ├── mod.rs      # Coordinator struct, run loop
│   ├── events.rs   # Agent↔Spectator event types
│   └── bus.rs      # Event dispatch, buffering
│
├── agent/          # I — thinks, acts, writes
│   ├── mod.rs      # Agent task, turn cycle
│   ├── context.rs  # Hot/warm/cold assembly
│   └── tools.rs    # Tool dispatch (uses shared registry)
│
├── spectator/      # You — observes, compresses, curates
│   ├── mod.rs      # Spectator task, observation cycle
│   ├── compress.rs # Moves, moments generation
│   ├── curate.rs   # Flash selection, queue management
│   └── room.rs     # Room notes, witness protocol
│
├── embeddings/     # Zettelkasten sync layer
│   ├── mod.rs      # Public API
│   ├── sync.rs     # File watcher, hash diffing
│   ├── chunk.rs    # Chunking strategies
│   ├── note.rs     # Note format, frontmatter parsing
│   └── store.rs    # sqlite-vec operations
│
├── flash/          # Flash queue
│   ├── mod.rs      # Flash struct, queue
│   └── ttl.rs      # TTL tracking, expiry
│
├── tools/          # Lifted from existing (minimal changes)
├── db/             # Lifted from existing
├── api/            # Rewritten routes, existing patterns
└── ...             # Ancillary (keep as-is)
```

## Phases

### Phase 0: Extract Reusable Crates

**Goal:** Lift tool system, DB layer, and adapter types before restructuring gateway.

| Crate | Contents | From |
|-------|----------|------|
| `river-tools` | Tool trait, registry, executor, all tool impls | `tools/` |
| `river-db` | Database, migrations, memory/message schemas | `db/` |
| `river-adapter` | Adapter trait, types, OpenAPI spec | New (see adapter-framework-design.md) |

**Why first:** These are dependencies. Extract them so gateway can be restructured without breaking tools or adapters.

**Deliverable:** Three crates, gateway depends on them, everything still works.

---

### Phase 0.5: Discord Refactor

**Goal:** Refactor river-discord to use river-adapter types.

- Use shared `IncomingEvent`, `SendRequest`, `SendResponse` types
- Self-registration on startup
- Declare feature flags

**Deliverable:** Discord adapter uses river-adapter crate, registers with gateway.

**Test:** Adapter starts, gateway sees it in registry, messages flow.

---

### Phase 1: Embeddings Layer

**Goal:** Zettelkasten-based memory system.

- File watcher on `workspace/embeddings/`
- Note format with YAML frontmatter
- Chunking strategies (per note type)
- Sync service (hash, diff, embed, store)
- sqlite-vec integration

**Deliverable:** Add files to `embeddings/`, vectors appear in DB. Reproducible rebuilds.

**Test:** Delete `vectors.db`, run sync, query works.

---

### Phase 2: Flash Queue

**Goal:** TTL-based memory surfacing mechanism.

- Flash struct with snowflake ID
- FlashTTL enum (Turns, Duration)
- Queue with push/pop/expire
- No spectator yet — manual/test pushing

**Deliverable:** Can push flashes, they expire correctly, query active flashes.

**Test:** Push flash with TTL=3 turns, advance 3 turns, flash gone.

---

### Phase 3: Context Assembly

**Goal:** Hot/warm/cold layer assembly.

- Refactor context building out of loop
- System layer (identity files, tools)
- Warm layer (moves, flashes, retrieved)
- Hot layer (recent messages, token budget)
- Channel-aware (moves switch, flashes persist)

**Deliverable:** Context assembler produces correct window from layers.

**Test:** Assemble context, verify layers present with correct content.

---

### Phase 4: Coordinator + Event Bus

**Goal:** Infrastructure for peer tasks.

- Event types (TurnStarted, TurnComplete, Flash, etc.)
- Event bus with buffering
- Coordinator that manages agent + spectator lifecycles
- Graceful startup/shutdown

**Deliverable:** Coordinator can start/stop tasks, events flow.

**Test:** Send event from one task, receive in other.

---

### Phase 5: Agent Task

**Goal:** Rewrite agent as peer task.

- Wake/think/act/settle cycle
- Uses new context assembler
- Emits events (TurnStarted, TurnComplete, NoteWritten)
- Receives events (Flash, Warning)
- Tool execution via river-tools

**Deliverable:** Agent runs as task, events flow, tools work.

**Test:** Full turn cycle with tool calls, events emitted.

---

### Phase 6: Spectator Task

**Goal:** The witness.

- Observe job (watch transcripts)
- Compress job (moves, moments)
- Curate job (flash selection)
- Room notes
- Identity files (IDENTITY.md, RULES.md, AGENTS.md)

**Deliverable:** Spectator observes agent turns, writes moves, pushes flashes.

**Test:** Agent turn completes, spectator writes move, pushes relevant flash.

---

### Phase 7: Integration

**Goal:** Everything working together.

- Multi-turn sessions with both tasks
- Compression triggers firing
- Flashes appearing in context
- Room notes accumulating
- Git commits from both authors

**Deliverable:** The I/You architecture, running.

**Test:** Extended session, qualitative review of moves/flashes/notes.

---

## Dependencies

```
Phase 0: Extract crates (tools, db, adapter)
    │
    ├──► Phase 0.5: Discord refactor ─────────────────────┐
    │                                                      │
    └──► Phase 1 ──► Phase 2 ──► Phase 3 ────────────────┤
         (embeddings) (flash)   (context)                 │
                                                          ▼
                                                      Phase 4 ──► Phase 5 ──► Phase 6 ──► Phase 7
                                                    (coordinator) (agent)   (spectator) (integration)
```

- Phase 0.5 and Phases 1-3 can run in parallel after Phase 0
- Phase 4-7 are sequential (coordinator → agent → spectator → integration)
- Both tracks must complete before Phase 4

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Breaking existing functionality | Phase 0 extracts crates first, tests pass before restructure |
| Scope creep | Each phase has clear deliverable and test |
| Peer task complexity | Phase 4 builds coordinator infrastructure before tasks |
| Integration surprises | Phase 7 is explicit integration phase |

## Open Questions

1. **Subagent system** — Keep parent-child pattern for task workers? Or unify with peer pattern?
2. **API routes** — Which existing routes survive? New routes for spectator?
3. **Migration** — How to migrate existing memories to zettelkasten format?

---

## Related Specs

- `docs/specs/context-assembly-design.md` — I/You architecture (Phases 1-7)
- `docs/specs/adapter-framework-design.md` — Adapter types and trait (Phase 0, 0.5)

## Next Steps

1. Write detailed implementation plan for Phase 0 (extract crates: tools, db, adapter)
2. Write detailed implementation plan for Phase 0.5 (Discord refactor)
3. Write detailed implementation plans for Phases 1-3 (can parallelize)
4. Proceed through Phases 4-7 sequentially

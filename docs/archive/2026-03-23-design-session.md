# Design Session: 2026-03-23

> River Engine architecture and roadmap planning

## What We Built

### Specs Written

| Spec | Lines | Content |
|------|-------|---------|
| `context-assembly-design.md` | 772 | I/You architecture, embeddings, flash queue, spectator |
| `adapter-framework-design.md` | 411 | Adapter trait, types, OpenAPI, Discord reference |
| `gateway-restructure-meta-plan.md` | 250 | 8-phase implementation roadmap |

### Research

- `two-people-in-the-room.md` — Chinese Room argument vs I/You architecture

### Roadmap Reorganized

- **Core Prototype** vs **Fun Features** separation
- Next specs: Tmux, Web/Search, Resilience, Utterances
- Later: Skills, MCP

## Key Architectural Decisions

### Gateway Restructure

Two tracks after Phase 0 (extract crates):

```
├── Communication (Phase 0.5) — mouth and ear
│   └── Adapter crate, Discord refactor
│
└── Cognition (Phases 1-7) — the mind
    └── Embeddings → Flash → Context → Coordinator → Agent → Spectator → Integration
```

### I/You Architecture

- **Agent (I)** — thinks, acts, writes notes, decides
- **Spectator (You)** — observes, compresses, curates, whispers
- Peer tasks, event-driven coordination
- Spectator never uses "I", critical in philosophical sense

### Context Assembly

- **Hot** — recent messages (token budget)
- **Warm** — moves, flashes, retrieved notes
- **Cold** — searchable but not active

### Transformation Principle

Nothing deleted, only moved between layers. Spectator transforms, doesn't erase.

### Adapter Framework

- `river-adapter` crate with types + trait + OpenAPI
- Core: send/receive. Everything else is feature flags.
- Self-registration, health on demand
- Metadata stays native to platform

## New Features Added to Roadmap

| Feature | Concept |
|---------|---------|
| Utterances | Speech as deliberate act via `speak` tool |
| Silent work | Background processing, no user output |
| Heartbeat coalescing | Priority queue for scheduled wakes |
| Tmux integration | Persistent terminal sessions |
| Web search | SearXNG self-hosted stack |

## Philosophy Captured

- "No mind should be the sole author of its own memory"
- "The agent thinks, then utters"
- "Philosophy as code"
- "Forest resilience — one tree dies, others take over"
- Spectator: dry truth, lays bare contradictions

## Review Received

William Thomas Lessing reviewed adapter framework spec — "Ready to build 👍"

Addressed:
- `Identify(String)` documentation
- Gateway restart behavior
- `(adapter, channel)` keying
- `AdapterError` variants
- Auth intentionally deferred

## Next Session

Specs to write:
1. Tmux Integration
2. Web & Search
3. Resilience
4. Utterances

Then: Implementation plans for Phase 0.

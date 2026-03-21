# River Engine Roadmap

> Brainstormed 2026-03-20, compiled by William

## Features Overview

1. **Timezone support** — agent needs proper time awareness
2. **Shell profile loading** — not loading user's shell profile correctly
3. **Reduce Nix dependency / Docker support** — simplify deployment, keep composable
4. **Voice chat** — natural next step for communication
5. **Issue tracking** — internal issue tracking system
6. **Module support** — modular architecture
7. **Skill support** — skill system (like OpenClaw's)
8. **MCP support** — Model Context Protocol integration
9. **Embeddings** — embedding pipeline; in-progress with William/Thomas
10. **Agent message history access** — agent doesn't have access to its messages yet
11. **Adversarial mind / actor-spectator** — dialectical architecture, "I" and "You"
12. **Study OpenClaw source** — learn from their architecture
13. **Embedding strategy** — iterative approach, memory files as first test case

---

## Implementation Plan

### Phase 0: Quick Wins + Research

Low-risk, high-value improvements plus research to inform later decisions.

| # | Feature | Notes |
|---|---------|-------|
| 1 | Timezone support | Simple fix, improves agent awareness |
| 2 | Shell profile loading | Basic fix, needed for proper command execution |
| 10 | Agent message history access | Gives agent access to its own conversation history |
| 12 | Study OpenClaw source | **Parallel research** — informs skill/module/MCP design |

### Phase 1: Foundation

Complete in-progress work and improve deployment.

| # | Feature | Notes |
|---|---------|-------|
| 13 | Embedding strategy | **Declarative sync model** — see below |
| 9 | Embeddings pipeline | sqlite-vec + sync service + external embed server |
| 3 | Docker/Podman support | Parallel to above; keep Nix as option, add container support |

#### Embedding Architecture (NixOS-style)

**Core idea:** `embeddings/` folder is source of truth. Sync service maintains DB state.

```
embeddings/           →  Sync Service  →  sqlite-vec DB
├── memory.md                            (chunks + vectors)
├── notes/*.md
└── context/*.md
```

**Components:**
1. **sqlite-vec storage** — Vector search in SQLite, no external DB
2. **Sync service** — Scans folder, hashes files, diffs against DB, adds/removes
3. **Embed client** — Calls external server (Ollama, OpenAI, etc.) for vectors
4. **Chunker** — Splits files into ~400 token pieces with overlap

**Sync triggers:** Startup, file watcher, manual API, periodic

**Fallback:** If sqlite-vec unavailable, store vectors as JSON, compute similarity in Rust

**Future:** Abstract `VectorStore` trait for Qdrant/Milvus when scale demands

See: `docs/research/embedding-architecture.md`

### Phase 2: Architecture

Build extensibility infrastructure.

| # | Feature | Notes |
|---|---------|-------|
| 6 | Module support | Foundation for skills and extensibility |
| 7 | Skill support | Depends on modules; informed by OpenClaw research |
| 8 | MCP support | Can parallel with skills; standardized tool integration |

### Phase 3: Features

Build on the new architecture.

| # | Feature | Notes |
|---|---------|-------|
| 5 | Issue tracking | Can leverage modules/skills infrastructure |
| 4 | Voice chat | New adapter type for communication |

### Phase 4: Advanced

Major architectural evolution.

| # | Feature | Notes |
|---|---------|-------|
| 11 | Adversarial mind | Actor-spectator dialectical model; needs careful design |

---

## Dependencies

```
┌─────────────────┐
│ OpenClaw Study  │ ─────────────────────────────────────┐
└────────┬────────┘                                      │
         │ informs                                       │
         ▼                                               ▼
┌─────────────────┐     ┌─────────────────┐     ┌───────────────┐
│ Module Support  │ ──▶ │  Skill Support  │     │  MCP Support  │
└────────┬────────┘     └────────┬────────┘     └───────┬───────┘
         │                       │                      │
         └───────────┬───────────┴──────────────────────┘
                     ▼
              ┌──────────────┐
              │ Issue Track  │
              └──────────────┘

┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│ Embed Strategy  │ ──▶ │  sqlite-vec     │ ──▶ │  Sync Service   │
│ (declarative)   │     │  integration    │     │  + chunker      │
└─────────────────┘     └─────────────────┘     └─────────────────┘

┌─────────────────┐
│ Adversarial Mind│  (standalone, needs solid foundation)
└─────────────────┘
```

---

## Open Questions

1. ~~**OpenClaw research**~~ — ✅ Completed. See `docs/research/openclaw-*.md`
2. ~~**Embeddings**~~ — ✅ Strategy defined: declarative sync + sqlite-vec
3. **Adversarial mind** — Design upfront or evolve as we go?
4. **Nix vs Docker** — Both-and? What's the primary deployment target?

## Resolved

- **OpenClaw research** — Extensive research completed. Key takeaways: skill system (CLI + metadata), tool policy pipeline, sqlite-vec for vectors, subagent hierarchy, channel adapter interface.
- **Embedding strategy** — NixOS-style declarative sync. `embeddings/` folder = source of truth. Sync service maintains sqlite-vec state. External embed server for vector generation.

---

## Notes

- "It's a both-end situation, doesn't have to be one or the other" — on Nix vs Docker
- "We needed to figure out a strategy and we needed to fail first" — on embeddings
- "Reading OpenClaw source and sniping most of the features. Well, the good ones." — on research approach

# River Engine Roadmap

> Last updated: 2026-03-23

## Status Legend

- 🔴 **Not Started**
- 🟡 **In Progress**
- 🟢 **Complete**
- ⚪ **Deferred**

---

## Quick Wins

| Feature | Status | Notes |
|---------|--------|-------|
| Timezone support | 🟢 | `PREFERENCES.toml` with chrono-tz |
| Shell profile loading | 🟢 | Bash `-l` flag for login shell |
| Message history access | 🟢 | Bidirectional conversations, `sync_conversation` tool |

---

## Monitoring & Observability

| Feature | Status | Notes |
|---------|--------|-------|
| Health endpoint | 🟢 | Rich `/health` with metrics |
| Agent metrics | 🟢 | `AgentMetrics` struct, context tracking |
| Health policy | 🟢 | `HealthPolicy` with degraded/needs-attention states |
| Systemd watchdog | 🟢 | `sd_notify` integration |
| Structured logging | 🟢 | JSON logging support |

**Files:** `metrics.rs`, `policy.rs`, `watchdog.rs`, `logging.rs`

---

## Agent Core

| Feature | Status | Notes |
|---------|--------|-------|
| Agent loop | 🟢 | Wake/think/act/settle cycle |
| Context persistence | 🟢 | Save/restore conversation state |
| Context rotation | 🟢 | Auto-rotate when approaching limit |
| Subagent spawning | 🟢 | Task workers and long-running subagents |
| Tool system | 🟢 | Registry, executor, 20+ tools |
| Native Anthropic API | 🟢 | Direct Claude API with ephemeral caching |

**Files:** `loop/`, `subagent/`, `tools/`

---

## Embeddings

**Status:** 🟡 In Progress

**Architecture:** Declarative sync (NixOS-style)

```
workspace/embeddings/     Sync Service      sqlite-vec
├── memory.md        ──→  (hash, diff,  ──→  (vectors)
├── notes/*.md            chunk, embed)
└── context/*.md
```

| Component | Status | Description |
|-----------|--------|-------------|
| sqlite-vec integration | 🔴 | Load extension, create virtual tables |
| Chunker | 🔴 | Split markdown into ~400 token pieces |
| Sync service | 🔴 | Scan folder, hash files, diff against DB |
| Embed client | 🟢 | Exists: `EmbeddingClient` in river-gateway |
| Search API | 🔴 | Query vectors with `vec_distance_cosine()` |

**Design:** See `docs/research/embedding-architecture.md`

**Principle:** The `embeddings/` folder is the source of truth. The database is derived state.

---

## Resilience

| Feature | Status | Notes |
|---------|--------|-------|
| Model fallback chains | 🔴 | Graceful degradation when primary model fails |
| Cron with exponential backoff | 🔴 | Self-healing scheduling for failed tasks |
| ATTENTION.md escalation | 🔴 | Agent writes urgent issues, human reviews |
| Tool policy pipeline | 🔴 | 2-3 layer: agent default, per-agent override, runtime deny |
| Context pruning (TTL) | 🔴 | Expire old messages, not just rotate context |

**Principle:** Forest resilience — one tree dies, others take over.

---

## Deployment

| Feature | Status | Notes |
|---------|--------|-------|
| Nix/NixOS | 🟢 | Current deployment method |
| Docker/Podman | 🔴 | Reduce Nix dependency, broader compatibility |
| Environment sanitization | 🔴 | Sanitize env vars before shell execution |

**Goal:** Both options, composable. Nix for declarative systems, Docker for everything else.

---

## Architecture

| Feature | Status | Depends On | Notes |
|---------|--------|------------|-------|
| Module support | 🔴 | — | Foundation for extensibility |
| Skill support | 🔴 | Modules | CLI tools + metadata (OpenClaw-style) |
| MCP support | 🔴 | Modules | Model Context Protocol integration |

**Research:** See `docs/research/openclaw-*.md`

**Key insight from OpenClaw:** Skills are just CLI wrappers with `SKILL.md` metadata files. Simple and effective.

---

## Communication

| Feature | Status | Notes |
|---------|--------|-------|
| Discord adapter | 🟢 | Working |
| Bidirectional conversations | 🟢 | `conversations/` module, outgoing message tracking |
| Discord /read endpoint | 🟢 | Fetch channel history from Discord API |
| Typing indicators | 🔴 | Show typing while agent is thinking |
| Adapter trait | 🔴 | Clean trait interface for adapters (not plugin monstrosity) |
| Voice chat | 🔴 | New adapter type |
| Issue tracking | 🔴 | Internal issue system for agent |

---

## Advanced

| Feature | Status | Notes |
|---------|--------|-------|
| Dynamic thinking temperature | 🔴 | Continuous 0.0–1.0, system-controlled |
| Adversarial mind | ⚪ | Actor-spectator dialectical architecture |

**Concept:** "I" and "You" — a spectator that observes and critiques the actor's work. The spectator adjusts the dial, agent operates at whatever level is set. "No mind should be the sole author of its own cognition level."

---

## Research

| Topic | Status | Output |
|-------|--------|--------|
| OpenClaw architecture | 🟢 | `docs/research/openclaw-architecture.md` |
| OpenClaw features | 🟢 | `docs/research/openclaw-features.md` |
| OpenClaw detailed | 🟢 | `docs/research/openclaw-features-detailed.md` |
| OpenClaw feature analysis | 🟢 | `docs/research/openclaw-features-analysis.md` |
| Embedding architecture | 🟢 | `docs/research/embedding-architecture.md` |

---

## Dependencies

```
                    ┌─────────────┐
                    │   Modules   │
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │  Skills  │ │   MCP    │ │  Issues  │
        └──────────┘ └──────────┘ └──────────┘

        ┌──────────┐     ┌──────────┐     ┌──────────┐
        │ sqlite-  │ ──▶ │  Sync    │ ──▶ │  Search  │
        │   vec    │     │ Service  │     │   API    │
        └──────────┘     └──────────┘     └──────────┘
```

---

## Open Questions

1. **Adversarial mind** — Design upfront or evolve as we go?
2. **Nix vs Docker** — Both-and? Primary deployment target?

---

## Notes

- "It's a both-end situation, doesn't have to be one or the other" — on Nix vs Docker
- "We needed to figure out a strategy and we needed to fail first" — on embeddings
- "Reading OpenClaw source and sniping most of the features. Well, the good ones."

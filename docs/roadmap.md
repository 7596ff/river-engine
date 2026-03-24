# River Engine Roadmap

> Last updated: 2026-03-23

## Status Legend

- 🔴 **Not Started**
- 🟡 **In Progress**
- 🟢 **Complete**
- ⚪ **Deferred**

---

# Core Prototype

The essential architecture. What makes River *River*.

## Foundation (Complete)

| Feature | Status | Notes |
|---------|--------|-------|
| Agent loop | 🟢 | Wake/think/act/settle cycle |
| Context persistence | 🟢 | Save/restore conversation state |
| Context rotation | 🟢 | Auto-rotate when approaching limit |
| Subagent spawning | 🟢 | Task workers and long-running subagents |
| Tool system | 🟢 | Registry, executor, 40+ tools |
| Native Anthropic API | 🟢 | Direct Claude API with ephemeral caching |
| Discord adapter | 🟢 | Working, bidirectional |
| Monitoring | 🟢 | Health, metrics, watchdog, structured logging |
| Nix deployment | 🟢 | Current deployment method |

## Gateway Restructure

**Spec:** `docs/specs/gateway-restructure-meta-plan.md`

**Plans:**
- Phase 0: `docs/superpowers/plans/2026-03-23-plan-phase0-extract-crates.md`
- Phase 0.5: `docs/superpowers/plans/2026-03-23-plan-phase0.5-discord-refactor.md`
- Phase 1-7: `docs/superpowers/plans/2026-03-23-plan-phase*.md`

| Phase | Status | Deliverable |
|-------|--------|-------------|
| Phase 0: Extract crates | 🔴 | river-tools, river-db, river-adapter |
| Phase 0.5: Discord refactor | 🔴 | Discord uses river-adapter types |
| Phase 1: Embeddings layer | 🔴 | Zettelkasten sync to sqlite-vec |
| Phase 2: Flash queue | 🔴 | TTL-based memory surfacing |
| Phase 3: Context assembly | 🔴 | Hot/warm/cold layers |
| Phase 4: Coordinator | 🔴 | Event bus, peer task management |
| Phase 5: Agent task | 🔴 | Agent as peer task |
| Phase 6: Spectator task | 🔴 | Observer, compressor, curator |
| Phase 7: Integration | 🔴 | I/You architecture running |

## I/You Architecture

**Spec:** `docs/specs/context-assembly-design.md`

The mind has two perspectives:
- **Agent (I)** — thinks, acts, writes notes, decides
- **Spectator (You)** — observes, compresses, curates, whispers

> "No mind should be the sole author of its own memory."

## Adapter Framework

**Spec:** `docs/specs/adapter-framework-design.md`

| Component | Status | Notes |
|-----------|--------|-------|
| river-adapter crate | 🔴 | Types, trait, OpenAPI |
| Discord refactor | 🔴 | Use shared types, self-register |
| Feature flags | 🔴 | Adapters declare capabilities |

---

# Fun Features

The shiny stuff. Build after core works.

## Communication

| Feature | Status | Notes |
|---------|--------|-------|
| Utterances | 🔴 | Speech as deliberate act via `speak` tool |
| Silent work | 🔴 | Background processing, no user output |
| Typing indicators | 🔴 | Show typing while agent thinks |
| Hooks expansion | 🔴 | Message lifecycle phases (received → processed → sent) |

**Utterances:** The agent thinks (internal stream), then *utters* (deliberate speech). Messages arrive when the agent chooses to speak.

## Resilience

| Feature | Status | Notes |
|---------|--------|-------|
| Model fallback chains | 🔴 | Primary fails → try fallbacks with cooldowns |
| Heartbeat coalescing | 🔴 | Priority queue for scheduled wakes |
| Cron with exponential backoff | 🔴 | 30s → 1m → 5m → 15m → 60m |
| ATTENTION.md escalation | 🔴 | Agent writes urgent issues, human reviews |
| Tool policy pipeline | 🔴 | Multi-layer filtering, deny-wins |
| Environment sanitization | 🔴 | Block *_API_KEY, *_TOKEN, etc. |

**Principle:** Forest resilience — one tree dies, others take over.

## Tmux Integration

| Feature | Status | Notes |
|---------|--------|-------|
| Tmux tool | 🔴 | Create/attach sessions, run commands in panes |
| Session persistence | 🔴 | Agent can resume terminal sessions across restarts |

**Concept:** Agent gets persistent terminal sessions. Long-running processes, interactive debugging, process monitoring.

## Web & Search

| Feature | Status | Notes |
|---------|--------|-------|
| Web search | 🔴 | SearXNG self-hosted, scrape results |
| Web fetch | 🟢 | Exists |
| SSRF protection | 🔴 | URL validation, private IP blocking |
| Content caching | 🔴 | SQLite, 15min TTL |

**Stack:** SearXNG (meta-search) → fetch top N → scrape/extract → return to agent

## Media & Voice

| Feature | Status | Notes |
|---------|--------|-------|
| Whisper transcription | 🔴 | Local tool, techniques with Will |
| TTS | 🔴 | qwentts local, explore more |
| Canvas | 🔴 | Research Obsidian canvas spec |
| Image generation | ⚪ | Tabled |
| Image/video analysis | ⚪ | Tabled |
| Browser automation | ⚪ | Tabled |

## Extensibility

| Feature | Status | Notes |
|---------|--------|-------|
| Skills | 🔴 | Needs spec — CLI tools + SKILL.md metadata |
| MCP | 🔴 | Needs spec — Model Context Protocol |

## Deployment

| Feature | Status | Notes |
|---------|--------|-------|
| Docker/Podman | 🔴 | Reduce Nix dependency |

---

# Research

| Topic | Status | Output |
|-------|--------|--------|
| OpenClaw architecture | 🟢 | `docs/research/openclaw-architecture.md` |
| OpenClaw features | 🟢 | `docs/research/openclaw-features.md` |
| OpenClaw detailed | 🟢 | `docs/research/openclaw-features-detailed.md` |
| OpenClaw feature analysis | 🟢 | `docs/research/openclaw-features-analysis.md` |
| Embedding architecture | 🟢 | `docs/research/embedding-architecture.md` |
| Context management | 🟢 | `docs/research/context-management-brainstorm.md` |
| Two people in the room | 🟢 | `docs/research/two-people-in-the-room.md` |
| Obsidian canvas | 🔴 | For canvas feature design |

---

# Specs

| Spec | Status | Priority |
|------|--------|----------|
| Context Assembly & I/You | 🟢 `docs/specs/context-assembly-design.md` | — |
| Adapter Framework | 🟢 `docs/specs/adapter-framework-design.md` | — |
| Gateway Restructure | 🟢 `docs/specs/gateway-restructure-meta-plan.md` | — |
| Tmux Integration | 🔴 Needs spec | Next |
| Web & Search | 🔴 Needs spec | Next |
| Resilience | 🔴 Needs spec | Next |
| Utterances | 🔴 Needs spec | Next |
| Skills | 🔴 Needs spec | Later |
| MCP | 🔴 Needs spec | Later |

---

# Notes

- "Philosophy as code"
- "No mind should be the sole author of its own memory"
- "The agent thinks, then utters"
- "Forest resilience — one tree dies, others take over"

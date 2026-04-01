# River Engine Roadmap

> Last updated: 2026-03-28

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

## Gateway Restructure (Complete)

**Spec:** `docs/superpowers/specs/2026-03-23-iyou-architecture-design.md`

**Plans:**
- Phase 0: `docs/superpowers/plans/2026-03-23-plan-phase0-extract-crates.md`
- Phase 0.5: `docs/superpowers/plans/2026-03-23-plan-phase0.5-discord-refactor.md`
- Phase 1-7: `docs/superpowers/plans/2026-03-23-plan-phase*.md`

| Phase | Status | Deliverable |
|-------|--------|-------------|
| Phase 0: Extract crates | 🟢 | river-tools, river-db, river-adapter |
| Phase 0.5: Discord refactor | 🟢 | Discord uses river-adapter types |
| Phase 1: Embeddings layer | 🟢 | VectorStore, SyncService |
| Phase 2: Flash queue | 🟢 | Priority-based memory retrieval |
| Phase 3: Context assembly | 🟢 | Hot/warm/cold tiers |
| Phase 4: Coordinator | 🟢 | Event bus, task spawning |
| Phase 5: Agent task | 🟢 | I - acting self, turn cycle |
| Phase 6: Spectator task | 🟢 | You - observing self, compression, curation |
| Phase 7: Integration | 🟢 | I/You architecture running, git authorship |

## I/You Architecture (Complete)

**Spec:** `docs/superpowers/specs/2026-03-23-iyou-architecture-design.md`

The mind has two perspectives:
- **Agent (I)** — thinks, acts, writes notes, decides
- **Spectator (You)** — observes, compresses, curates, whispers

> "No mind should be the sole author of its own memory."

Architecture:
```
Coordinator
├── Agent Task (I - acting self)
│   ├── Turn cycle: wake → think → act → settle
│   ├── Context assembly (hot/warm/cold)
│   └── Emits: TurnStarted, TurnComplete, NoteWritten, ContextPressure
│
├── Spectator Task (You - observing self)
│   ├── Compressor: moves → moments
│   ├── Curator: semantic search → flashes
│   └── Emits: MovesUpdated, Warning
│
└── Event Bus (broadcast channel)
```

## Adapter Framework (Complete)

**Spec:** `docs/specs/adapter-framework-design.md`

| Component | Status | Notes |
|-----------|--------|-------|
| river-adapter crate | 🟢 | Types, trait, OpenAPI |
| Discord refactor | 🟢 | Uses shared types |
| Feature flags | 🔴 | Adapters declare capabilities |

## River Oneshot (In Progress)

**Spec:** `docs/superpowers/specs/2026-03-27-river-oneshot-design.md`

Turn-based dual-loop agent CLI. Complements gateway's always-on model.

| Phase | Status | Deliverable |
|-------|--------|-------------|
| Phase 1: Skeleton | 🟢 | Project setup, types, CLI |
| Phase 2: Single Loop | 🟢 | Claude provider, message assembly |
| Phase 3: Dual Loop | 🔴 | Skills, both loops completing |
| Phase 4: Memory | 🔴 | Vector store integration |
| Phase 5: Polish | 🔴 | Error handling, other providers |

---

# Fun Features

The shiny stuff. Build after core works.

## Communication

| Feature | Status | Notes |
|---------|--------|-------|
| Utterances | 🟢 | `speak` tool + `switch_channel` for channel-aware messaging |
| Silent work | 🔴 | Background processing, no user output |
| Typing indicators | 🟢 | `typing` tool shows typing while agent thinks |
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

## Deployment & Platform

| Feature | Status | Notes |
|---------|--------|-------|
| Docker/Podman | 🔴 | Reduce Nix dependency |
| macOS support | 🔴 | Native builds and testing |
| Windows support | ⚪ | Native builds and testing |
| Nix flake | 🔴 | Flake packaging (currently standalone modules) |

## Discord Enhancements

| Feature | Status | Notes |
|---------|--------|-------|
| Embed support | 🔴 | Rich message formatting with embeds |
| File/attachment handling | 🔴 | Upload and download files |
| Message edit/delete events | 🔴 | React to edits and deletions |
| Multiple guild support | 🔴 | Single adapter instance, multiple servers |
| Voice channel support | ⚪ | Join voice, TTS, audio processing |

## Orchestrator Enhancements

| Feature | Status | Notes |
|---------|--------|-------|
| Prometheus metrics | 🔴 | Export metrics for monitoring dashboards |
| Request queuing | 🔴 | Queue requests when resources busy |
| Priority preemption | 🔴 | Interactive requests evict batch jobs |
| Agent restart | 🔴 | Detect unhealthy agents, trigger restart |
| Persistence | 🔴 | SQLite for historical data, crash recovery |
| Model preloading | ⚪ | Predictive loading based on usage patterns |
| Multi-node distribution | ⚪ | Distribute models across machines |

## Additional Adapters

| Adapter | Status | Notes |
|---------|--------|-------|
| CLI | 🔴 | Interactive terminal adapter (for testing) |
| Slack | 🔴 | Slack workspace integration |
| Matrix | 🔴 | Matrix/Element chat integration |
| Telegram | 🔴 | Telegram bot integration |
| IRC | 🔴 | Internet Relay Chat |
| Email | ⚪ | IMAP/SMTP integration |
| Web UI | ⚪ | Browser-based chat interface |

## Security

| Feature | Status | Notes |
|---------|--------|-------|
| Audit logging | 🔴 | Detailed logs for security auditing |
| Authentication | 🔴 | API keys/tokens for service communication |
| TLS for internal comms | 🔴 | HTTPS between gateway, orchestrator, adapters |
| Encryption at rest | ⚪ | Encrypt SQLite databases and state files |

## Publishing & Integration

| Feature | Status | Notes |
|---------|--------|-------|
| Webhooks | 🔴 | Outbound webhooks for events |
| REST API clients | 🔴 | Generated client libraries |
| AT Protocol | ⚪ | Bluesky/AT Protocol integration |
| Tangled publishing | ⚪ | Publish to Tangled network |

## Advanced Agent Features

| Feature | Status | Notes |
|---------|--------|-------|
| Onboarding flow | 🔴 | Guided setup for new agents |
| Memory consolidation | ⚪ | Algorithms to compress/summarize old memories |
| Negotiated priority | ⚪ | Agents negotiate priority based on context |
| Co-processor architecture | ⚪ | Specialized sub-models for specific tasks |

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
| Context Assembly & I/You | 🟢 `docs/superpowers/specs/2026-03-23-iyou-architecture-design.md` | — |
| Adapter Framework | 🟢 `docs/specs/adapter-framework-design.md` | — |
| Gateway Restructure | 🟢 `docs/specs/gateway-restructure-meta-plan.md` | — |
| River Oneshot | 🟢 `docs/superpowers/specs/2026-03-27-river-oneshot-design.md` | — |
| Tmux Integration | 🔴 Needs spec | Next |
| Web & Search | 🔴 Needs spec | Next |
| Resilience | 🔴 Needs spec | Next |
| Utterances | 🟢 Complete | — |
| Skills | 🔴 Needs spec | Later |
| MCP | 🔴 Needs spec | Later |

---

# Open Questions

Architectural decisions that may need revisiting:

| Question | Context | Status |
|----------|---------|--------|
| Subagent unification | Keep parent-child pattern for task workers, or unify with coordinator peer pattern? | Needs review |
| Memory migration | How to migrate existing SQLite embeddings to zettelkasten format for existing agents? | Needs review |
| Redis fate | Redis ephemeral memory overlaps with flash queue. Keep both? | Needs review |

---

# Notes

- "Philosophy as code"
- "No mind should be the sole author of its own memory"
- "The agent thinks, then utters"
- "Forest resilience — one tree dies, others take over"

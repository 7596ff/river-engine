# River Engine Implementation Status

**Last Updated:** 2026-03-28

> **Feature status:** See [`docs/roadmap.md`](../roadmap.md) for the canonical feature roadmap.
> This document tracks implementation details, test counts, and technical notes.

---

## Completed

### Plan 1: Core Libraries ✅
- `river-core` crate with:
  - 128-bit Snowflake IDs (AgentBirth, SnowflakeType, Snowflake, SnowflakeGenerator)
  - Types (Priority, SubagentType, ContextStatus)
  - RiverError with 11 variants
  - Configuration types (AgentConfig, HeartbeatConfig, EmbeddingConfig, OrchestratorConfig)
- 87 tests passing

### Plan 2: Gateway Core ✅
- `river-gateway` crate with:
  - SQLite database layer with migrations
  - Message CRUD operations
  - Tool system (registry, executor, 6 core tools)
  - HTTP API (axum-based)
  - Server setup with CLI
- 30 tests passing
- Binary: `river-gateway --workspace <path> --data-dir <path>`

### Plan 3: Memory & Embeddings ✅
- Semantic memory (SQLite embeddings):
  - Memory CRUD operations with f32 vector storage
  - Embedding server client (OpenAI-compatible API)
  - Vector similarity search with cosine similarity
  - Memory tools (embed, memory_search, memory_delete, memory_delete_by_source)
- Redis ephemeral memory (4 domains):
  - Working memory (TTL in minutes)
  - Medium-term memory (TTL in hours)
  - Coordination (locks, counters)
  - Cache (optional TTL)
  - 10 Redis tools total
- Added Embedding and Redis error variants to river-core
- 131 tests passing (83 core, 44 gateway, 4 doc-tests)
- Binary: `river-gateway ... --embedding-url <url> --redis-url <url> --agent-name <name>`

### Plan 4: Minimal Orchestrator ✅
- `river-orchestrator` crate with:
  - Agent health monitoring via heartbeats
  - Agent status API (`/agents/status`)
  - Static model registry (`/models/available`)
  - Health endpoint (`/health`)
  - Heartbeat endpoint (`POST /heartbeat`)
  - CLI: `river-orchestrator --port --health-threshold --models-config`
- Gateway integration:
  - `--orchestrator-url` flag for heartbeat configuration
  - Background heartbeat task (30 second interval)
  - Graceful degradation (works without orchestrator)
- 151 tests passing (83 core, 46 gateway, 18 orchestrator, 4 doc-tests)
- Binary: `river-orchestrator --port 5000 --models-config <path>`

### Plan 5: Advanced Orchestrator ✅
- Model discovery via GGUF header parsing:
  - Parse GGUF magic number, version, metadata
  - Extract architecture, parameters, quantization type
  - Calculate VRAM requirements from file size + KV cache overhead
- GPU/VRAM and CPU memory tracking:
  - GPU discovery via nvidia-smi
  - System memory tracking from /proc/meminfo
  - Swap detection with warnings (proceeds but warns if swap would be used)
  - Device resource allocation with reserved memory
- llama-server process lifecycle management:
  - Automatic port allocation (default range 8080-8180)
  - Process spawning with GPU/CPU selection
  - Health monitoring loop (10s interval)
  - Idle model eviction (configurable timeout, default 15 minutes)
- LiteLLM integration for external models:
  - External models config file support
  - Direct endpoint routing for external models
- On-demand model loading:
  - GPU-first with CPU fallback
  - Automatic eviction of releasable models when resources needed
- New API endpoints:
  - `POST /model/request` - Request model, blocks until ready
  - `POST /model/release` - Mark model as releasable
  - `GET /resources` - Device and loaded model status
  - Enhanced `GET /models/available` - Local/external models with resources
- 181 tests passing (83 core, 46 gateway, 48 orchestrator, 4 doc-tests)
- Binary: `river-orchestrator --model-dirs /models --external-models config.json`

### Plan 6: Discord Adapter ✅
- Twilight-based Discord adapter
- Channel management via slash commands and admin API
- Message and reaction forwarding to gateway
- Outbound message sending from agent
- Dynamic channel add/remove at runtime
- State persistence to file
- Thread support (send to threads, create threads)
- 197 tests passing (83 core, 46 gateway, 47 orchestrator, 16 discord, 1 integration, 4 doc-tests)
- Binary: `river-discord --token-file /path --gateway-url http://localhost:3000 --guild-id 123`

### Plan 7: NixOS Module ✅
- NixOS module (`nix/nixos-module.nix`):
  - Orchestrator, embedding, Redis, agents with Discord
  - System services with DynamicUser
  - Dedicated users for agents
- Home-manager module (`nix/home-module.nix`):
  - Identical functionality as user services
  - Redis as direct user service
- Shared library (`nix/lib.nix`):
  - Option type definitions
  - Command builders for all services
- Package definitions (`nix/packages.nix`):
  - CUDA-optional llama-cpp
  - All River binaries

### I/You Architecture Restructure ✅

**Master Spec:** `docs/superpowers/specs/2026-03-23-iyou-architecture-design.md`

All 8 phases completed:

| Phase | Description | Status |
|-------|-------------|--------|
| Phase 0 | Extract Crates (`river-db`, `river-tools`, `river-migrate`, `river-adapter`) | ✅ |
| Phase 1 | Embeddings (VectorStore, SyncService) | ✅ |
| Phase 2 | Flash Queue (priority-based memory retrieval) | ✅ |
| Phase 3 | Context Assembly (hot/warm/cold tiers) | ✅ |
| Phase 4 | Coordinator (task spawning, event bus) | ✅ |
| Phase 5 | Agent Task (I - acting self, turn cycle) | ✅ |
| Phase 6 | Spectator Task (You - observing self, compression, curation) | ✅ |
| Phase 7 | Integration (coordinator default, git authorship, compression triggers) | ✅ |

Architecture:
```
Coordinator
├── Agent Task (I - acting self)
│   ├── Turn cycle: wake → think → act → settle
│   ├── Context assembly (hot/warm/cold)
│   ├── Tool execution with stats
│   └── Emits: TurnStarted, TurnComplete, NoteWritten, ContextPressure
│
├── Spectator Task (You - observing self)
│   ├── Observes agent events
│   ├── Compressor: moves → moments
│   ├── Curator: semantic search → flashes
│   ├── RoomWriter: session observations
│   └── Emits: MovesUpdated, Warning
│
└── Event Bus (broadcast channel)
```

- 544 tests passing (90 core, 308 gateway, 43 orchestrator, 23 discord, 43 tools, 14 db, 3 adapter, 15 integration, 4 doc-tests, 1 migrate)

## In Progress: River Oneshot

**Spec:** `docs/superpowers/specs/2026-03-27-river-oneshot-design.md`
**Plan:** `crates/river-oneshot/PLAN.md`

Turn-based dual-loop agent CLI. Complements gateway's always-on model.

### Architecture
- Two concurrent loops: reasoning (LLM) + execution (skills)
- Both complete every cycle, first ready wins, other cached
- Memory via river-db (SQLite + vector store)
- Native Rust skills

### Phases
1. [x] Skeleton - project setup, types, CLI
2. [x] Single Loop - Claude provider, message assembly
3. [ ] Dual Loop - skills, both loops completing
4. [ ] Memory & Embeddings - vector store integration
5. [ ] Polish - error handling, other providers

---

## Next Up: Qualitative Review & Production Testing

**Review Plan:** `docs/superpowers/plans/2026-03-25-plan-qualitative-review.md`

### Tasks

1. **Qualitative Review** - Run extended sessions, evaluate:
   - Moves capture and formatting
   - Moments compression quality
   - Room notes coherence
   - Flash relevance
   - Spectator voice consistency
   - System stability

2. **Production Testing** - Test with real adapters and conversations

3. **Tuning** - Adjust thresholds based on findings:
   - Compression interval (currently 10 turns)
   - Compression pressure threshold (currently 80%)
   - Moves threshold for moments (currently 15)
   - Flash similarity threshold (currently 0.6)

## Key Files

- **Spec:** `docs/superpowers/specs/2026-03-16-river-engine-design.md`
- **I/You Spec:** `docs/superpowers/specs/2026-03-23-iyou-architecture-design.md`
- **Oneshot Spec:** `docs/superpowers/specs/2026-03-27-river-oneshot-design.md`
- **Plans:** `docs/superpowers/plans/`
- **Core:** `crates/river-core/src/`
- **Gateway:** `crates/river-gateway/src/`
- **DB:** `crates/river-db/src/`
- **Tools:** `crates/river-tools/src/`
- **Migrate:** `crates/river-migrate/src/`
- **Adapter:** `crates/river-adapter/src/`
- **Orchestrator:** `crates/river-orchestrator/src/`
- **Discord:** `crates/river-discord/src/`
- **Oneshot:** `crates/river-oneshot/src/`
- **Nix Modules:** `nix/`

## Test Commands

```bash
cargo test                              # Run all tests
cargo build --release                   # Build all binaries
./target/release/river-gateway --help   # Show gateway CLI options
./target/release/river-orchestrator --help  # Show orchestrator CLI options
```

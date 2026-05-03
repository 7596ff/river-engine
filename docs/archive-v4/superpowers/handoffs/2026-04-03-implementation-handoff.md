# Implementation Handoff — 2026-04-03

> River Engine v4 implementation progress and next steps

## Session Summary

Implemented Stages 1-5 of the master spec (`docs/superpowers/specs/2026-04-02-master-spec.md`).

## Completed Crates

### river-snowflake (Stage 1)
**Path:** `crates/river-snowflake/`
**Type:** Library + binary
**Status:** Complete, 9 tests passing

128-bit unique ID generation with:
- `Snowflake` struct (high: timestamp micros, low: birth+type+sequence)
- `AgentBirth` (36-bit packed timestamp)
- `SnowflakeType` enum (Message, Embedding, Session, etc.)
- `SnowflakeGenerator` (thread-safe per-birth generator with `next()` method)
- `GeneratorCache` (multi-birth generator cache)
- `parse()` / `format()` for hex string conversion
- `timestamp_iso8601()` for timestamp extraction
- HTTP server (feature-gated behind `server` feature)

**API endpoints:** `GET /id/{type}?birth=`, `POST /ids`, `GET /health`

### river-adapter (Stage 1)
**Path:** `crates/river-adapter/`
**Type:** Library (types only)
**Status:** Complete, 1 doctest passing

Defines adapter ↔ worker interface:
- `FeatureId` enum (SendMessage, ReceiveMessage, EditMessage, etc.) with `TryFrom<u16>`
- `OutboundRequest` enum (typed payloads for each feature)
- `InboundEvent` / `EventMetadata` (enum with event-specific fields) / `EventType`
- `OutboundResponse` / `ResponseData` / `ResponseError` (response types)
- `Author`, `Channel`, `Attachment` (supporting types)
- `Baton` (Actor/Spectator), `Side` (Left/Right) with `Hash` impl, `Ground` (human operator)
- `Adapter` trait (async trait for adapter implementations)
- OpenAPI generation via utoipa

### river-embed (Stage 2)
**Path:** `crates/river-embed/`
**Type:** Binary
**Status:** Complete, 2 tests passing

Vector search service with:
- Markdown-aware chunking (`chunk.rs`)
- Embedding client for Ollama/OpenAI-compatible APIs (`embed.rs`)
- SQLite storage with cosine similarity search (`store.rs`)
- Cursor-based search iteration (`search.rs`)
- HTTP server (`http.rs`)

**Note:** Simplified implementation without sqlite-vec extension. Stores vectors as blobs and computes similarity in Rust. Works but not optimized for large datasets.

**API endpoints:** `POST /index`, `DELETE /source/{path}`, `POST /search`, `POST /next`, `GET /health`

### river-context (Stage 3)
**Path:** `crates/river-context/`
**Type:** Library
**Status:** Complete, 3 tests passing

Context assembly for workers:
- `OpenAIMessage` with helper methods (`system()`, `user()`, `assistant()`, `tool()`)
- `ToolCall`, `FunctionCall` (OpenAI-compatible tool calls)
- `Moment`, `Move`, `ChatMessage`, `Flash` (with `from` field), `Embedding` (workspace types)
- `ChannelContext`, `ContextRequest` (request types)
- `ContextResponse`, `ContextError` (response types)
- `build_context()` pure function (assembles workspace data into messages)
- Token estimation (~4 chars/token heuristic)
- TTL filtering for flashes and embeddings

**Assembly order:**
1. Other channels (moments + moves only)
2. Last channel (moments + moves + embeddings)
3. LLM history block
4. Current channel (moments + moves + messages + embeddings)
5. Flashes (interspersed, high priority)

### river-orchestrator (Stage 3)
**Path:** `crates/river-orchestrator/`
**Type:** Binary
**Status:** Complete, 4 tests passing

Process supervisor with:
- Config loading with `$ENV_VAR` substitution (`config.rs`)
- Registry state and push mechanism (`registry.rs`)
- Process spawning (workers, adapters, embed service) (`supervisor.rs`)
- Health checks (60s interval, 3 failures = dead)
- Worker registration (returns baton, model config, ground, workspace)
- Adapter registration (validates required features, returns config + worker endpoint)
- Embed service registration (returns model config)
- Model switching (`POST /model/switch`)
- Role switching protocol (`POST /switch_roles` with two-phase commit)
- Worker output handling (`POST /worker/output`)
- Respawn policy tracking (Done, ContextExhausted, Error) (`respawn.rs`)
- Graceful shutdown

**API endpoints:** `POST /register`, `POST /model/switch`, `POST /switch_roles`, `POST /worker/output`, `GET /registry`, `GET /health`

### river-worker (Stage 4)
**Path:** `crates/river-worker/`
**Type:** Binary
**Status:** Complete, 2 tests passing

Worker runtime with:
- CLI parsing and startup sequence (`main.rs`)
- Configuration from CLI args and registration (`config.rs`)
- Worker state management (`state.rs`)
- LLM client for OpenAI-compatible endpoints (`llm.rs`)
- Context persistence in JSONL format (`persistence.rs`)
- HTTP server for notifications and flashes (`http.rs`)
- Main think→act loop (`worker_loop.rs`)
- Initial context loading (role file, identity file, initial_message)
- Context pressure warnings at 80%
- 17 tool implementations (`tools.rs`):
  - File: read, write, delete
  - Bash: command execution with timeout
  - Communication: speak, adapter, switch_channel
  - Control: sleep, watch, summary
  - Memory: create_move, create_moment, create_flash
  - Model: request_model, switch_roles
  - Search: search_embeddings, next_embedding

**API endpoints:** `POST /notify`, `POST /flash`, `POST /registry`, `POST /prepare_switch`, `POST /commit_switch`, `GET /health`

### river-discord (Stage 5)
**Path:** `crates/river-discord/`
**Type:** Binary
**Status:** Complete, builds successfully

Discord adapter using twilight crates:
- Gateway connection via `twilight-gateway` (`discord.rs`)
- Event forwarding to worker via `/notify` endpoint
- HTTP API for adapter operations (`http.rs`)
- Supported features:
  - SendMessage, ReceiveMessage, EditMessage, DeleteMessage
  - ReadHistory, AddReaction, RemoveReaction, TypingIndicator
- Timestamp formatting via chrono (twilight Timestamp -> RFC3339)
- Emoji handling (unicode and custom Discord emoji)

**API endpoints:** `POST /start`, `POST /execute`, `GET /health`

**Startup sequence:**
1. Parse CLI args (--orchestrator, --dyad, --type, --port)
2. Bind HTTP server
3. Register with orchestrator (sends feature list)
4. Receive config (token, guild_id, intents) from orchestrator
5. Initialize Discord gateway connection
6. Start event forwarding loop
7. Serve HTTP requests

## Test Summary

**Total: 23 tests passing**
- river-snowflake: 9 tests
- river-context: 3 tests
- river-embed: 2 tests
- river-orchestrator: 4 tests
- river-worker: 2 tests
- doctests: 3 tests

## Remaining Work

### Stage 4: river-worker (Polish)
- [ ] Conversation file format (hybrid append-only with compaction)
- [ ] Malformed tool call retry with backoff
- [ ] More robust error handling

### Stage 6: Integration
- [ ] Workspace directory template
- [ ] actor.md and spectator.md role files
- [ ] Identity file templates
- [ ] Backchannel adapter
- [ ] End-to-end testing

## Key Design Decisions Made

1. **sqlite-vec simplified:** Removed sqlite-vec dependency from river-embed due to FFI complexity. Using blob storage + Rust-side cosine similarity. Works for small datasets. Consider revisiting for production.

2. **Error handling:** Using manual Error trait implementations instead of thiserror in some places due to type complexity.

3. **Thread safety:** river-embed uses `std::sync::Mutex` instead of `tokio::sync::RwLock` because rusqlite::Connection is not Send/Sync.

4. **Flash format:** Flash struct uses single `from` field (format: "dyad:side") instead of separate `from_dyad` and `from_side` fields.

5. **EventMetadata as enum:** EventMetadata is a data-carrying enum with per-event-type fields, not a flat struct.

6. **OpenAIMessage helpers:** Added `system()`, `user()`, `assistant()`, `tool()` helper methods to reduce boilerplate.

7. **Twilight Shard not Sync:** Discord client spawns a dedicated task for the gateway event loop, communicates via mpsc channel. This avoids issues with Shard not implementing Sync.

8. **Timestamp conversion:** Using chrono for timestamp formatting. Twilight's `Timestamp::as_micros()` -> chrono `DateTime::from_timestamp()` -> RFC3339 string.

## Files to Read First

1. `docs/superpowers/specs/2026-04-02-master-spec.md` — Overall architecture
2. `docs/superpowers/specs/2026-04-01-adapter-library-design.md` — Adapter interface spec
3. `crates/river-worker/src/tools.rs` — Tool implementations for reference
4. `crates/river-discord/src/discord.rs` — Discord gateway implementation

## Build & Test

```bash
# Build all crates
cargo build --workspace

# Run all tests
cargo test --workspace

# Build snowflake server
cargo build --package river-snowflake --features server

# Run snowflake server
cargo run --package river-snowflake --features server -- --port 4001

# Build orchestrator
cargo build --package river-orchestrator

# Build worker
cargo build --package river-worker

# Build discord adapter
cargo build --package river-discord

# Run discord adapter (requires orchestrator running)
cargo run --package river-discord -- --orchestrator http://localhost:4000 --dyad river --type discord
```

## Workspace Dependencies

Added to `Cargo.toml`:
- utoipa (OpenAPI generation)
- async-trait
- base64
- futures (for river-embed)
- sha2, rand (for river-embed)
- zerocopy (for vector byte conversion)
- chrono (for timestamps)
- urlencoding (for URL encoding in tools)
- regex (for env var substitution)
- tower-http (for HTTP tracing)
- twilight-gateway, twilight-http, twilight-model (for Discord)

All crates use workspace dependencies where possible.

## Architecture Notes

The I/You architecture creates a dyad (pair of workers) where:
- **Actor (I):** Handles external communication via adapters
- **Spectator (You):** Manages memory, reviews actor's work
- **Ground (Human):** Reality-checks via backchannel

Workers switch roles via orchestrator-mediated two-phase commit. This ensures atomic role swaps even if one worker crashes mid-switch.

### Adapter Flow

```
Discord -> river-discord -> POST /notify -> river-worker
river-worker -> POST /execute -> river-discord -> Discord
```

Adapters register with orchestrator on startup, receive secrets and worker endpoint. All configuration flows through orchestrator, not CLI args.

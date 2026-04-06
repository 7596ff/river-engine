# Architecture

**Analysis Date:** 2026-04-06

## Pattern Overview

**Overall:** Microservice-based multi-agent system with central orchestration.

**Key Characteristics:**
- **Distributed spawning:** Orchestrator launches worker and adapter processes, tracks registration
- **Worker-centric think→act loop:** Workers call LLM, execute tools, handle notifications asynchronously
- **Registry-based service discovery:** All processes register endpoints with orchestrator for inter-process communication
- **Dyad model:** Each conversation pair (left + right worker) forms a "dyad" with shared adapter connections
- **Zero-knowledge message model:** Workers don't inject full message content; model reads history on demand via tools

## Layers

**Orchestrator (`river-orchestrator`):**
- Purpose: Process supervisor, registry, HTTP endpoint coordination
- Location: `crates/river-orchestrator/src/`
- Contains: Process spawning, health checks, respawn logic, HTTP router, registry management
- Depends on: `river-adapter`, `river-protocol`, tokio, axum, reqwest
- Used by: External systems (CLI, config files); spawns all other processes

**Worker (`river-worker`):**
- Purpose: Agent runtime that implements think→act loop (LLM → tools → notifications)
- Location: `crates/river-worker/src/`
- Contains: Worker loop, LLM client, tool execution, workspace loading, conversation persistence, HTTP handlers
- Depends on: `river-adapter`, `river-context`, `river-protocol`, `river-snowflake`, tokio, axum, reqwest
- Used by: Orchestrator (spawns); receives requests from adapters and orchestrator

**Protocol (`river-protocol`):**
- Purpose: Foundational types for all processes (no inter-crate dependencies)
- Location: `crates/river-protocol/src/`
- Contains: `Side`, `Baton`, `Channel`, `Ground`, `Author`, `ModelConfig`, registration/response types, conversation file handling
- Depends on: serde, utoipa (for OpenAPI docs)
- Used by: All other crates

**Adapter Types (`river-adapter`):**
- Purpose: Types-only interface for worker ↔ adapter communication
- Location: `crates/river-adapter/src/`
- Contains: `FeatureId` enum (lightweight capability markers), `OutboundRequest` (typed requests from worker), `InboundEvent` (typed events from adapters), feature system, OpenAPI schema
- Depends on: `river-protocol`, serde, utoipa
- Used by: Workers (for tool execution); adapter binaries (for implementation); protocol definitions

**Context Assembly (`river-context`):**
- Purpose: Pure function to assemble workspace data into LLM-compatible messages
- Location: `crates/river-context/src/`
- Contains: `build_context()` function, message formatting, token estimation, workspace type definitions
- Depends on: `river-protocol`, serde
- Used by: Workers (before each LLM call)

**Snowflake ID Generation (`river-snowflake`):**
- Purpose: Distributed ID generation with encoded timestamps and metadata
- Location: `crates/river-snowflake/src/`
- Contains: Snowflake ID generator, `AgentBirth` metadata
- Depends on: chrono, serde
- Used by: Workers (for message IDs)

**Adapters (Examples):**
- `river-discord`: Discord gateway client using twilight, HTTP adapter interface
  - Location: `crates/river-discord/src/`
- `river-embed`: Vector embedding and semantic search service
  - Location: `crates/river-embed/src/`

**TUI (`river-tui`):**
- Purpose: Terminal UI for observing worker state and adapters
- Location: `crates/river-tui/src/`
- Contains: Ratatui-based UI, HTTP client for querying orchestrator/workers
- Depends on: ratatui, crossterm, reqwest

## Data Flow

**Startup Flow:**

1. Orchestrator loads `river.json` config (dyads, adapters, embed service)
2. Orchestrator starts HTTP server, creates shared registry/supervisor/respawn manager
3. For each configured dyad:
   - Orchestrator calls `spawn_worker()` twice (left + right side)
   - Each worker registers with orchestrator via `/register` endpoint
   - Orchestrator stores endpoint in registry, returns partner endpoint + model config
4. For each dyad's adapters:
   - Orchestrator calls `spawn_adapter()` for each adapter config
   - Adapter registers with orchestrator via `/register` endpoint
   - Orchestrator links adapter to dyad workers in registry
5. Orchestrator enters supervision loop: health checks every 60s, respawn failed processes

**Worker Think→Act Loop:**

1. Worker calls LLM with: system prompt + conversation history + current context tokens
2. LLM returns: either text response (no tools) or tool calls array
3. Tool execution:
   - Execute all tool calls in parallel (independent)
   - For each result: append to message history, persist to workspace JSONL
   - If `summary` tool called: stop loop, return output
   - If `sleep` tool called: pause loop, wait for notification wake
   - Otherwise: go to step 1
4. Text response (no tools):
   - Inject system-role status message: "You responded with text. Context: X / Y tokens. Current time: Z. New notifications: ..."
   - Go to step 1
5. Loop stops when: `summary` called, context exhausted (95%), or error

**Notification Flow:**

1. Adapter receives external event (e.g., Discord message)
2. Adapter posts `InboundEvent` to worker's `/notify` endpoint
3. Worker writes message to workspace conversation file
4. Worker injects notification string: "New message in #channel (X unread)"
5. If worker is sleeping and channel is in watch list: worker wakes, processes immediately
6. If worker is thinking: notification batches until next status injection

**Respawn Flow:**

1. Orchestrator health check finds dead process
2. Process removed from supervisor, registry
3. Respawn manager queues respawn with exponential backoff
4. After backoff duration expires, orchestrator wakes process
5. Process re-registers with orchestrator

## Key Abstractions

**WorkerState (`river-worker`):**
- Purpose: Mutable shared state for worker runtime
- Examples: `crates/river-worker/src/state.rs`
- Pattern: `Arc<RwLock<WorkerState>>` shared across HTTP server, worker loop, tool execution

**ProcessRegistry (`river-orchestrator`):**
- Purpose: Track all running workers, adapters, embed services by identity
- Examples: `crates/river-orchestrator/src/registry.rs`
- Pattern: HashMap keyed by `ProcessKey` (dyad name + side/adapter type)

**Supervisor (`river-orchestrator`):**
- Purpose: Spawn, track, and shut down child processes
- Examples: `crates/river-orchestrator/src/supervisor.rs`
- Pattern: Stores `ProcessHandle` (Child + endpoint + failure count) for each running process

**LlmClient (`river-worker`):**
- Purpose: OpenAI-compatible chat completions client
- Examples: `crates/river-worker/src/llm.rs`
- Pattern: Wraps reqwest client, deserializes tool calls, tracks token usage

**FeatureId + OutboundRequest (`river-adapter`):**
- Purpose: Bidirectional mapping between capability markers and typed operations
- Examples: `crates/river-adapter/src/feature.rs`
- Pattern: `FeatureId` enum (u16) → `OutboundRequest` variant via `request.feature_id()`, reverse via `OutboundRequest::feature_id()`

## Entry Points

**Orchestrator Entry:**
- Location: `crates/river-orchestrator/src/main.rs`
- Triggers: `river-orchestrator --config river.json [--port 3000]`
- Responsibilities: Load config, spawn all processes, run supervision loop, handle health checks and respawns

**Worker Entry:**
- Location: `crates/river-worker/src/main.rs`
- Triggers: `river-worker --orchestrator http://... --dyad dyad-name --side [left|right] --port 0`
- Responsibilities: Register with orchestrator, load workspace, run think→act loop, accept notifications

**Adapter Entry (Example - Discord):**
- Location: `crates/river-discord/src/main.rs`
- Triggers: Spawned by orchestrator via `Command::new("river-discord")`
- Responsibilities: Connect to Discord gateway, forward events to worker, execute outbound requests

## Error Handling

**Strategy:** Result-based error propagation with context-preserving error types.

**Patterns:**
- `Result<T, AdapterError>` in `river-adapter` for feature validation failures
- `Result<T, SupervisorError>` in `river-orchestrator` for spawn/signal failures
- `anyhow::Result<T>` for application-level errors (e.g., config loading, LLM client)
- `thiserror::Error` for custom error types with display/source chain
- Worker loop captures errors, returns `ExitStatus::Error { message }` instead of panicking

## Cross-Cutting Concerns

**Logging:** Tracing crate with structured logging:
- Orchestrator: `river_orchestrator=info` by default
- Worker: `river_worker=info` by default
- Subscribers use `fmt::layer()` for human-readable output, can switch to JSON with `EnvFilter`

**Validation:** Type-driven via serde (deserialization fails fast on invalid JSON):
- `Side::Left | Side::Right` enforced at serialization level
- `Baton::Actor | Baton::Spectator` enforced at serialization level
- `FeatureId` validated via `TryFrom<u16>` (returns `Err(u16)` if invalid)

**Authentication:** Via orchestrator registration:
- Workers and adapters register endpoint with orchestrator on startup
- Orchestrator stores endpoint in registry (currently no auth token; designed for private networks)
- Future: API key or JWT token could be added to registration response

**Concurrency:** Async/await with tokio:
- Orchestrator: supervision loop uses `tokio::select!` to multiplex health checks and respawn wakes
- Worker: think→act loop uses `tokio::spawn()` for concurrent tool execution
- All shared state protected with `Arc<RwLock<T>>` for thread-safe interior mutability

---

*Architecture analysis: 2026-04-06*

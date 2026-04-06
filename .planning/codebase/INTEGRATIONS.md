# External Integrations

**Analysis Date:** 2026-04-06

## APIs & External Services

**Language Model Services:**
- Anthropic Claude API - Primary LLM for agent reasoning
  - SDK/Client: Custom implementation via `reqwest` with OpenAI-compatible API format
  - Auth: Environment variable `$ANTHROPIC_API_KEY`
  - Endpoint: `https://api.anthropic.com/v1/messages` (configurable in `models` config)
  - Supported models: `claude-sonnet-4-20250514`, `claude-haiku-4-20250514`
  - Context: Handles context limits per model (e.g., 200000 tokens for Claude Sonnet)
  - Integration: `crates/river-worker/src/llm.rs` implements LlmClient for chat completions with tool calling

- OpenAI-Compatible LLM Endpoints - Generic support for any OpenAI API-compatible service
  - Configuration allows arbitrary endpoints for future compatibility

**Embedding Services:**
- Ollama (via OpenAI-compatible API) - Local embedding generation
  - Endpoint: `http://localhost:11434/v1/embeddings` (default in example config)
  - Model: `nomic-embed-text` (768 dimensions)
  - API Key: "ollama" (dummy value for local Ollama)
  - Integration: `crates/river-embed/src/embed.rs` calls embedding API
  - Fallback: If embedding service unavailable, requests are queued

## Data Storage

**Databases:**
- SQLite with sqlite-vec extension - Primary persistent storage
  - Connection: File-based database (default: `embed.db`, configurable via `--db` flag)
  - Client: Rusqlite 0.39 with `zerocopy` support for zero-copy reads
  - Usage: Vector embeddings, conversation history, agent context
  - Extension: sqlite-vec for KNN similarity search on embeddings
  - Implementation: `crates/river-embed/src/store.rs` manages SQLite connections and vector indexing

**File Storage:**
- Local filesystem only - No cloud storage integrations
  - Workspace files: Loaded from `workspace` path in dyad config
  - Context persistence: JSONL format in `.workspaces/{dyad}/context.jsonl`
  - Chat history: File-based storage via `crates/river-worker/src/persistence.rs`

**Caching:**
- None detected - No Redis or Memcached integration
- In-memory state via `tokio::sync::RwLock` for registered adapters/workers

## Authentication & Identity

**Discord Integration:**
- Provider: Discord OAuth/Token-based
  - Token: Environment variable `$DISCORD_TOKEN`
  - Guild ID: Optional guild restriction via `guild_id` config
  - DM Channel: Environment variable `$DISCORD_DM_CHANNEL_ID` for ground communication
  - Implementation: Twilight Gateway for event streaming, Twilight HTTP for API calls
  - Location: `crates/river-discord/src/main.rs` and `crates/river-discord/src/discord.rs`

**Model/Service Authentication:**
- API Keys: Environment variable references in JSON config (`$ANTHROPIC_API_KEY`, `$OLLAMA_API_KEY`, etc.)
- Configuration-driven: Auth credentials specified in model/service definitions
- No token refresh or OAuth flow implemented - static API keys expected

## Monitoring & Observability

**Error Tracking:**
- None detected - No Sentry, DataDog, or similar integration

**Logs:**
- Tracing-based structured logging via `tracing` crate (0.1)
- Subscriber: `tracing-subscriber` with environment filter and JSON output
- Format: JSON log format for structured analysis
- Environment control: `RUST_LOG` environment variable
- Default directives: "river_orchestrator=info", "river_discord=info", "river_worker=info"
- Location: Implemented in each binary's main.rs using `tracing_subscriber::registry()`

**Metrics:**
- None detected - No Prometheus or metrics collection

## CI/CD & Deployment

**Hosting:**
- Self-hosted on Linux (signal handling assumes Unix)
- NixOS-friendly with declarative module configuration
- Docker-unfriendly (no Dockerfile detected)

**CI Pipeline:**
- None detected - No GitHub Actions, GitLab CI, or similar

**Package Distribution:**
- Nix packages: Built via `nix/package.nix`
- NixOS modules: Service definitions in `nix/module.nix` and `nix/nixos-module.nix`
- Binary paths: All services installed to `$out/bin/` in Nix store

## Environment Configuration

**Required env vars:**
- `ANTHROPIC_API_KEY` - Critical for LLM operation
- `DISCORD_TOKEN` - Critical for Discord adapter
- `DISCORD_DM_CHANNEL_ID` - For ground communication in Discord dyad

**Optional env vars:**
- `RUST_LOG` - Control tracing verbosity

**Secrets location:**
- Environment file: Path specified in NixOS module `environmentFile` option
- Format: Sourced by systemd service (EnvironmentFile=)
- Examples: `.env`, `.env.local` (convention, not enforced)
- Note: Values are interpolated in JSON config at startup via environment variable substitution

**Configuration files:**
- `river.json` - Main orchestrator config (generated from NixOS settings or user-provided)
- Dyad workspaces: `{workspace}/` paths for agent state and conversations
- Example configs: `nix/river.example.json`, `nix/example-config.nix`

## Service Registration & Discovery

**Adapter Registration:**
- POST `/register` - Adapters register with orchestrator on startup
- Request: `AdapterRegistrationRequest` with endpoint and adapter details
- Response: `AdapterRegistrationResponse` with worker endpoint and configuration
- Implementation: `crates/river-discord/src/main.rs:99-122` (Discord example)
- Protocol: JSON over HTTP with 30-second timeout

**Worker Registration:**
- POST `/register` - Workers register with orchestrator on startup
- Request: `WorkerRegistrationRequest` with endpoint, dyad, and side
- Response: Worker configuration and adapter endpoints
- Location: `crates/river-worker/src/main.rs:88-105`
- Protocol: JSON over HTTP

**Embed Service Registration:**
- POST `/register` - Embedding service registers with orchestrator
- Request: `RegistrationRequest` with endpoint and service name
- Response: Model configuration (name, dimensions, endpoint, API key)
- Implementation: `crates/river-embed/src/main.rs:60-74`

## Webhooks & Callbacks

**Incoming Webhooks:**
- POST `/notify` - Worker receives events from adapters
  - Location: `crates/river-worker/src/http.rs` (router implementation)
  - Payload: Adapter-specific event JSON (e.g., Discord events from `crates/river-discord/src/discord.rs`)
  - Source: Adapters forward events to this endpoint via `{worker_endpoint}/notify`

- HTTP handlers in each service:
  - Orchestrator: `/register` (worker/adapter registration), health checks
  - Worker: `/notify` (adapter event forwarding), `/state`, `/conversation` endpoints
  - Discord Adapter: `/execute` (outbound message execution)
  - Embed Service: `/embed`, `/search`, `/chunk-search` endpoints

**Outgoing Webhooks:**
- POST `{orchestrator_url}/register` - Services register with orchestrator
  - Adapters and workers call this on startup
  - Implementation: All services use `reqwest::Client` for registration

- POST `{worker_endpoint}/notify` - Event forwarding from adapters to workers
  - Discord adapter forwards gateway events: `crates/river-discord/src/main.rs:158-174`
  - Payload: Serialized event JSON
  - Timeout: 5 seconds per forward

- POST `{embedding_service}/embed` - Worker calls embedding service
  - Converts text chunks to vectors for RAG

## System Communication Flow

**Startup Registration Chain:**
1. Orchestrator starts (HTTP server on configured port)
2. Embed service starts → registers with orchestrator
3. Workers start → register with orchestrator → receive adapter endpoints
4. Adapters start → register with orchestrator → receive worker endpoint

**Runtime Event Flow:**
1. External source (Discord, user request, etc.) → Adapter
2. Adapter → POST `/notify` to Worker
3. Worker processes event → calls LLM (Anthropic)
4. LLM response → Worker may call tools or adapters
5. Adapter executes action (send Discord message, etc.)
6. Optional: Embed text chunks with embedding service → store in SQLite

---

*Integration audit: 2026-04-06*

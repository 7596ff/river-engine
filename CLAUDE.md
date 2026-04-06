<!-- GSD:project-start source:PROJECT.md -->
## Project

**River Engine**

An agent orchestrator implementing dyadic architecture — two AI instances alternate between actor and spectator roles, witnessing each other. The spectator sees patterns the actor cannot see about themselves. Built in Rust with NixOS deployment, designed for a human operator (Ground) who has full access and final say.

**Core Value:** Two perspectives that can disagree. The gap between them is the point — it creates internal structure that a single rule-follower cannot have.

### Constraints

- **Stack**: Rust 2021, Tokio async runtime, Axum HTTP — established, not changing
- **Deployment**: NixOS modules exist, systemd integration — maintain compatibility
- **LLM Protocol**: OpenAI-compatible API — workers already implement this
- **Testing**: Must work with TUI mock adapter before Discord — reduces variables
<!-- GSD:project-end -->

<!-- GSD:stack-start source:codebase/STACK.md -->
## Technology Stack

## Languages
- Rust 2021 edition - Core system implementation across all crates
- Nix - Infrastructure and development environment configuration
## Runtime
- Tokio 1.0 - Async runtime for concurrent operations
- Cargo - Rust package management
- Lockfile: `Cargo.lock` (present)
## Frameworks
- Axum 0.8 - HTTP server framework for all services (workers, adapters, orchestrator)
- Tokio 1.0 - Async task spawning and signal handling
- Utoipa 5.0 - OpenAPI schema generation for API documentation
- Serde 1.0 with derive - Core serialization framework
- Serde JSON 1.0 - JSON serialization for protocol messages
- Serde YAML 0.9 - YAML configuration parsing
- Twilight Gateway 0.16 - Discord gateway connection and event streaming
- Twilight HTTP 0.16 - Discord HTTP API client
- Twilight Model 0.16 - Discord data structures
- Twilight Util 0.16 - Discord utility functions
- Tempfile 3.10 - Temporary file management for tests
- Clap 4.0 - CLI argument parsing (all binaries)
- Nix flakes - Declarative development environment and NixOS modules
## Key Dependencies
- Tokio 1.0 (full features) - Async runtime, multithread, process spawning, signals
- Axum 0.8 - HTTP routing and middleware
- Reqwest 0.12 - HTTP client with JSON and Rustls TLS
- Serde/Serde JSON - Protocol serialization (all components)
- Tower 0.5 - Middleware composition for Axum
- Tower HTTP 0.6 - CORS and tracing middleware
- Tracing 0.1 - Structured logging framework
- Tracing Subscriber 0.3 - Logging subscriber with environment filter and JSON output
- Chrono 0.4 - Timestamp handling with serde support
- Rusqlite 0.39 - SQLite embedded database for conversation history and embeddings
- sqlite-vec 0.1 - Vector similarity search extension for SQLite
- sqlite-vec 0.1 - KNN vector search using SQLite
- Snowflake ID generator (custom crate `river-snowflake`) - 128-bit distributed IDs
- Thiserror 2.0 - Error type derivation
- Anyhow 1.0 - Error handling context
- Base64 0.22 - Base64 encoding/decoding
- SHA2 0.10 - SHA256 hashing for embeddings
- Rand 0.9 - Random number generation
- Regex 1.10 - Pattern matching for configuration interpolation
- Walkdir 2 - Recursive directory traversal for workspace loading
- Glob 0.3 - File pattern matching
- Async Trait 0.1 - Async trait support
- Chrono 0.4 - Timestamps
- URL Encoding 2.1 - URL encoding for parameters
- Ratatui 0.29 - Terminal UI rendering for debugger
- Crossterm 0.28 - Cross-platform terminal control
- Nix 0.29 - Unix signal handling for orchestrator
## Crates Structure
- `river-orchestrator` - Process supervisor (main.rs)
- `river-worker` - Agent runtime with LLM integration (main.rs)
- `river-discord` - Discord adapter (main.rs)
- `river-embed` - Vector search service (main.rs)
- `river-snowflake` - ID generation service (lib.rs + optional server binary)
- `river-tui` - Terminal debugger (main.rs)
- `river-protocol` - Shared protocol types (lib.rs)
- `river-adapter` - Adapter communication types (lib.rs)
- `river-context` - Worker context assembly (lib.rs)
## Configuration
- Configuration via JSON files: `river.json` (main config) and adapter-specific JSON
- Environment variable interpolation in JSON using `$VAR_NAME` syntax
- Environment file support through NixOS module: `environmentFile` option for secrets
- `Cargo.toml` - Workspace root with shared dependencies
- `Cargo.lock` - Lock file for reproducible builds
- `flake.nix` - Nix flake for declarative development environment
- `shell.nix` - Legacy Nix shell (fallback)
- `.nvmrc` / `.python-version` - Not present (Rust project)
## Platform Requirements
- Rust toolchain (1.70+, 2021 edition)
- Cargo package manager
- GCC or Clang for native compilation
- SQLite development libraries (`pkg-config`, `libsqlite3-dev`)
- OpenSSL development libraries (for TLS in reqwest)
- Nix (optional, for flake development)
- Node.js 24 (in shell.nix, purpose unclear - may be unused)
- Linux runtime (signal handling via `nix` crate uses Unix signals)
- SQLite3 runtime
- OpenSSL runtime libraries
- Port access for HTTP services (default: 4337 for orchestrator, dynamic for workers/adapters)
- Network access to:
- Services defined in `nix/module.nix` (main NixOS module)
- Service files for orchestrator, workers, and adapters
- User/group management
- Systemd integration for process management
<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->
## Conventions

## Naming Patterns
- Rust module files use `snake_case` (e.g., `llm.rs`, `worker_loop.rs`, `workspace_loader.rs`)
- Test files follow the pattern: `{module_name}.rs` for unit tests in `#[cfg(test)]` blocks, or `tests/integration.rs` for integration tests
- Example files: `examples/detailed_channels.rs`
- All functions use `snake_case` (e.g., `parse_message_line`, `build_context`, `format_message`)
- Private helper functions are prefixed with underscore when needed
- Async functions use `async fn` keyword without special naming suffix
- Local variables and bindings use `snake_case` (e.g., `current_message`, `response_message`, `dyad_name`)
- Loop variables use short descriptive names (e.g., `line`, `entry`, `feature`)
- Unused variables are explicitly marked with underscore prefix to suppress warnings: `#[allow(dead_code)]` or `let _var`
- Struct and enum names use `PascalCase` (e.g., `Author`, `ChatMessage`, `OutboundRequest`, `FeatureId`)
- Generic type parameters use single uppercase letters (e.g., `T`, `E`) or descriptive names
- Private types are declared in modules and re-exported via `pub use` in lib.rs
- Enum variants use `PascalCase` (e.g., `SendMessage`, `MessageCreate`, `Left`, `Right`)
- Snake_case serde renaming applied via `#[serde(rename_all = "snake_case")]` for JSON serialization compatibility
## Code Style
- Standard Rust formatting via implicit rustfmt (2021 edition)
- 4-space indentation (Rust standard)
- Brace style: Allman-style opening braces on same line for functions/blocks
- Line length: Generally < 100 characters, but no strict enforced limit
- Default cargo clippy checks (no explicit .clippy.toml configuration found)
- Common patterns like `#[allow(dead_code)]` used when intentional
- Explicit derives used: `Clone`, `Debug`, `PartialEq`, `Serialize`, `Deserialize`, `ToSchema`
- Standard derives often combined in one line: `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]`
## Import Organization
- No path aliases used. All imports are absolute from crate root or external crates.
## Error Handling
- `thiserror` crate used for error enums with derive macros
- Error enums use `#[derive(Debug, thiserror::Error)]`
- Each error variant has a descriptive `#[error("...")]` message
- Errors are propagated using `?` operator, not `unwrap()`
#[derive(Debug, thiserror::Error)]
- `ParseError` defined as struct wrapper: `pub struct ParseError(pub String)` with manual `Error` trait implementation
- Errors implement `std::error::Error` trait
- IO errors wrapped and context added: `.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.0))`
- Result types use explicit error type: `Result<T, E>` or `Result<T, io::Error>`
- Main functions return `Result<(), Box<dyn std::error::Error>>` for flexible error handling
- Early returns with `?` on registration failures in `main.rs`
## Logging
- Info level for important startup events: `tracing::info!("...")`
- Error level for failures: `tracing::error!("...")`
- Warning level for recoverable issues: `tracing::warn!(...)`
- Structured logging with named fields: `tracing::warn!(error = %e, "message")`
- Debug output uses `{:?}` for Debug formatting
## Comments
- File-level documentation on crate modules: `//! Module description` at top of lib.rs
- Complex parsing logic documented with inline comments
- Intentional design decisions explained (e.g., "Reaction line" vs "Message line")
- Skip empty section comments unless explaining non-obvious control flow
- Rust doc comments use `///` for public items
- Module docs use `//!` for module-level documentation
- Example from `river-adapter/src/lib.rs` includes usage examples in doc comments
- Doc examples use triple backticks with `rust` language tag
## Function Design
- Functions generally 50-150 lines for complex operations (e.g., `format_line` in format.rs is ~100 lines)
- Larger functions (500+ lines) exist for complex parsing (e.g., `tools.rs` is 1318 lines for tool execution)
- No strict line limit, but readability prioritized with helper functions
- Use strong typing: specific enums/structs rather than strings where possible
- Optional parameters use `Option<T>` rather than default arguments
- References used for borrowed data: `&str`, `&Path`
- Owned data when mutation needed or to maintain ownership
- Use Result for fallible operations: `Result<T, E>`
- Void operations return `()` or `Result<(), E>` if they can fail
- Tuple returns used for multiple related values: `(meta, body)`
- Named fields in return enums for clarity: `SendMessage { channel, content, reply_to }`
## Module Design
- Public modules declared in `lib.rs` with `pub mod name;`
- Selective re-exports via `pub use` to expose public API:
- Private implementation details hidden in modules
- Each crate has lib.rs serving as the main barrel file
- Sub-modules organized in src/ directory matching module structure
- Example: `river-adapter` has `lib.rs` with `mod event`, `mod feature`, `mod error`, etc.
## Serialization Conventions
- All public types derive `Serialize` and `Deserialize`
- OpenAPI schema support via `utoipa::ToSchema` derive
- `#[serde(rename_all = "snake_case")]` on enums for JSON compatibility
- `#[serde(default, skip_serializing_if = "Option::is_none")]` for optional fields
- Enums serialize as lowercase snake_case variants: `SendMessage` → `"send_message"`
- Struct fields use default naming (snake_case via rename_all)
- Type field for tagged enums: `#[serde(rename = "type")]` for adapter_type
## Testing Conventions
- Tests in same file as code: `#[cfg(test)] mod tests { #[test] fn test_...() { } }`
- Tests grouped at end of file or module
- Async tests use `#[tokio::test]` attribute
- Ignored expensive tests: `#[tokio::test] #[ignore]` with comment explaining why
<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->
## Architecture

## Pattern Overview
- **Distributed spawning:** Orchestrator launches worker and adapter processes, tracks registration
- **Worker-centric think→act loop:** Workers call LLM, execute tools, handle notifications asynchronously
- **Registry-based service discovery:** All processes register endpoints with orchestrator for inter-process communication
- **Dyad model:** Each conversation pair (left + right worker) forms a "dyad" with shared adapter connections
- **Zero-knowledge message model:** Workers don't inject full message content; model reads history on demand via tools
## Layers
- Purpose: Process supervisor, registry, HTTP endpoint coordination
- Location: `crates/river-orchestrator/src/`
- Contains: Process spawning, health checks, respawn logic, HTTP router, registry management
- Depends on: `river-adapter`, `river-protocol`, tokio, axum, reqwest
- Used by: External systems (CLI, config files); spawns all other processes
- Purpose: Agent runtime that implements think→act loop (LLM → tools → notifications)
- Location: `crates/river-worker/src/`
- Contains: Worker loop, LLM client, tool execution, workspace loading, conversation persistence, HTTP handlers
- Depends on: `river-adapter`, `river-context`, `river-protocol`, `river-snowflake`, tokio, axum, reqwest
- Used by: Orchestrator (spawns); receives requests from adapters and orchestrator
- Purpose: Foundational types for all processes (no inter-crate dependencies)
- Location: `crates/river-protocol/src/`
- Contains: `Side`, `Baton`, `Channel`, `Ground`, `Author`, `ModelConfig`, registration/response types, conversation file handling
- Depends on: serde, utoipa (for OpenAPI docs)
- Used by: All other crates
- Purpose: Types-only interface for worker ↔ adapter communication
- Location: `crates/river-adapter/src/`
- Contains: `FeatureId` enum (lightweight capability markers), `OutboundRequest` (typed requests from worker), `InboundEvent` (typed events from adapters), feature system, OpenAPI schema
- Depends on: `river-protocol`, serde, utoipa
- Used by: Workers (for tool execution); adapter binaries (for implementation); protocol definitions
- Purpose: Pure function to assemble workspace data into LLM-compatible messages
- Location: `crates/river-context/src/`
- Contains: `build_context()` function, message formatting, token estimation, workspace type definitions
- Depends on: `river-protocol`, serde
- Used by: Workers (before each LLM call)
- Purpose: Distributed ID generation with encoded timestamps and metadata
- Location: `crates/river-snowflake/src/`
- Contains: Snowflake ID generator, `AgentBirth` metadata
- Depends on: chrono, serde
- Used by: Workers (for message IDs)
- `river-discord`: Discord gateway client using twilight, HTTP adapter interface
- `river-embed`: Vector embedding and semantic search service
- Purpose: Terminal UI for observing worker state and adapters
- Location: `crates/river-tui/src/`
- Contains: Ratatui-based UI, HTTP client for querying orchestrator/workers
- Depends on: ratatui, crossterm, reqwest
## Data Flow
## Key Abstractions
- Purpose: Mutable shared state for worker runtime
- Examples: `crates/river-worker/src/state.rs`
- Pattern: `Arc<RwLock<WorkerState>>` shared across HTTP server, worker loop, tool execution
- Purpose: Track all running workers, adapters, embed services by identity
- Examples: `crates/river-orchestrator/src/registry.rs`
- Pattern: HashMap keyed by `ProcessKey` (dyad name + side/adapter type)
- Purpose: Spawn, track, and shut down child processes
- Examples: `crates/river-orchestrator/src/supervisor.rs`
- Pattern: Stores `ProcessHandle` (Child + endpoint + failure count) for each running process
- Purpose: OpenAI-compatible chat completions client
- Examples: `crates/river-worker/src/llm.rs`
- Pattern: Wraps reqwest client, deserializes tool calls, tracks token usage
- Purpose: Bidirectional mapping between capability markers and typed operations
- Examples: `crates/river-adapter/src/feature.rs`
- Pattern: `FeatureId` enum (u16) → `OutboundRequest` variant via `request.feature_id()`, reverse via `OutboundRequest::feature_id()`
## Entry Points
- Location: `crates/river-orchestrator/src/main.rs`
- Triggers: `river-orchestrator --config river.json [--port 3000]`
- Responsibilities: Load config, spawn all processes, run supervision loop, handle health checks and respawns
- Location: `crates/river-worker/src/main.rs`
- Triggers: `river-worker --orchestrator http://... --dyad dyad-name --side [left|right] --port 0`
- Responsibilities: Register with orchestrator, load workspace, run think→act loop, accept notifications
- Location: `crates/river-discord/src/main.rs`
- Triggers: Spawned by orchestrator via `Command::new("river-discord")`
- Responsibilities: Connect to Discord gateway, forward events to worker, execute outbound requests
## Error Handling
- `Result<T, AdapterError>` in `river-adapter` for feature validation failures
- `Result<T, SupervisorError>` in `river-orchestrator` for spawn/signal failures
- `anyhow::Result<T>` for application-level errors (e.g., config loading, LLM client)
- `thiserror::Error` for custom error types with display/source chain
- Worker loop captures errors, returns `ExitStatus::Error { message }` instead of panicking
## Cross-Cutting Concerns
- Orchestrator: `river_orchestrator=info` by default
- Worker: `river_worker=info` by default
- Subscribers use `fmt::layer()` for human-readable output, can switch to JSON with `EnvFilter`
- `Side::Left | Side::Right` enforced at serialization level
- `Baton::Actor | Baton::Spectator` enforced at serialization level
- `FeatureId` validated via `TryFrom<u16>` (returns `Err(u16)` if invalid)
- Workers and adapters register endpoint with orchestrator on startup
- Orchestrator stores endpoint in registry (currently no auth token; designed for private networks)
- Future: API key or JWT token could be added to registration response
- Orchestrator: supervision loop uses `tokio::select!` to multiplex health checks and respawn wakes
- Worker: think→act loop uses `tokio::spawn()` for concurrent tool execution
- All shared state protected with `Arc<RwLock<T>>` for thread-safe interior mutability
<!-- GSD:architecture-end -->

<!-- GSD:skills-start source:skills/ -->
## Project Skills

No project skills found. Add skills to any of: `.claude/skills/`, `.agents/skills/`, `.cursor/skills/`, or `.github/skills/` with a `SKILL.md` index file.
<!-- GSD:skills-end -->

<!-- GSD:workflow-start source:GSD defaults -->
## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:
- `/gsd-quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd-debug` for investigation and bug fixing
- `/gsd-execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->



<!-- GSD:profile-start -->
## Developer Profile

> Profile not yet configured. Run `/gsd-profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->

# Technology Stack

**Analysis Date:** 2026-04-06

## Languages

**Primary:**
- Rust 2021 edition - Core system implementation across all crates

**Secondary:**
- Nix - Infrastructure and development environment configuration

## Runtime

**Environment:**
- Tokio 1.0 - Async runtime for concurrent operations

**Package Manager:**
- Cargo - Rust package management
- Lockfile: `Cargo.lock` (present)

## Frameworks

**Core:**
- Axum 0.8 - HTTP server framework for all services (workers, adapters, orchestrator)
- Tokio 1.0 - Async task spawning and signal handling

**API & Protocol:**
- Utoipa 5.0 - OpenAPI schema generation for API documentation

**Data Serialization:**
- Serde 1.0 with derive - Core serialization framework
- Serde JSON 1.0 - JSON serialization for protocol messages
- Serde YAML 0.9 - YAML configuration parsing

**Discord Integration:**
- Twilight Gateway 0.16 - Discord gateway connection and event streaming
- Twilight HTTP 0.16 - Discord HTTP API client
- Twilight Model 0.16 - Discord data structures
- Twilight Util 0.16 - Discord utility functions

**Testing:**
- Tempfile 3.10 - Temporary file management for tests

**Build/Dev:**
- Clap 4.0 - CLI argument parsing (all binaries)
- Nix flakes - Declarative development environment and NixOS modules

## Key Dependencies

**Critical:**
- Tokio 1.0 (full features) - Async runtime, multithread, process spawning, signals
- Axum 0.8 - HTTP routing and middleware
- Reqwest 0.12 - HTTP client with JSON and Rustls TLS
- Serde/Serde JSON - Protocol serialization (all components)

**Infrastructure:**
- Tower 0.5 - Middleware composition for Axum
- Tower HTTP 0.6 - CORS and tracing middleware
- Tracing 0.1 - Structured logging framework
- Tracing Subscriber 0.3 - Logging subscriber with environment filter and JSON output

**Date/Time:**
- Chrono 0.4 - Timestamp handling with serde support

**Database:**
- Rusqlite 0.39 - SQLite embedded database for conversation history and embeddings
- sqlite-vec 0.1 - Vector similarity search extension for SQLite

**Vector Search:**
- sqlite-vec 0.1 - KNN vector search using SQLite

**ID Generation:**
- Snowflake ID generator (custom crate `river-snowflake`) - 128-bit distributed IDs

**Utilities:**
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

**TUI:**
- Ratatui 0.29 - Terminal UI rendering for debugger
- Crossterm 0.28 - Cross-platform terminal control

**System Integration:**
- Nix 0.29 - Unix signal handling for orchestrator

## Crates Structure

**Workspace Members:**
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

**Environment:**
- Configuration via JSON files: `river.json` (main config) and adapter-specific JSON
- Environment variable interpolation in JSON using `$VAR_NAME` syntax
- Environment file support through NixOS module: `environmentFile` option for secrets

**Build:**
- `Cargo.toml` - Workspace root with shared dependencies
- `Cargo.lock` - Lock file for reproducible builds
- `flake.nix` - Nix flake for declarative development environment
- `shell.nix` - Legacy Nix shell (fallback)
- `.nvmrc` / `.python-version` - Not present (Rust project)

## Platform Requirements

**Development:**
- Rust toolchain (1.70+, 2021 edition)
- Cargo package manager
- GCC or Clang for native compilation
- SQLite development libraries (`pkg-config`, `libsqlite3-dev`)
- OpenSSL development libraries (for TLS in reqwest)
- Nix (optional, for flake development)
- Node.js 24 (in shell.nix, purpose unclear - may be unused)

**Production:**
- Linux runtime (signal handling via `nix` crate uses Unix signals)
- SQLite3 runtime
- OpenSSL runtime libraries
- Port access for HTTP services (default: 4337 for orchestrator, dynamic for workers/adapters)
- Network access to:
  - LLM endpoints (Anthropic API)
  - Embedding service endpoints
  - Discord Gateway (if using Discord adapter)

**NixOS Integration:**
- Services defined in `nix/module.nix` (main NixOS module)
- Service files for orchestrator, workers, and adapters
- User/group management
- Systemd integration for process management

---

*Stack analysis: 2026-04-06*

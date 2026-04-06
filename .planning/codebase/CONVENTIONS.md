# Coding Conventions

**Analysis Date:** 2026-04-06

## Naming Patterns

**Files:**
- Rust module files use `snake_case` (e.g., `llm.rs`, `worker_loop.rs`, `workspace_loader.rs`)
- Test files follow the pattern: `{module_name}.rs` for unit tests in `#[cfg(test)]` blocks, or `tests/integration.rs` for integration tests
- Example files: `examples/detailed_channels.rs`

**Functions:**
- All functions use `snake_case` (e.g., `parse_message_line`, `build_context`, `format_message`)
- Private helper functions are prefixed with underscore when needed
- Async functions use `async fn` keyword without special naming suffix

**Variables:**
- Local variables and bindings use `snake_case` (e.g., `current_message`, `response_message`, `dyad_name`)
- Loop variables use short descriptive names (e.g., `line`, `entry`, `feature`)
- Unused variables are explicitly marked with underscore prefix to suppress warnings: `#[allow(dead_code)]` or `let _var`

**Types:**
- Struct and enum names use `PascalCase` (e.g., `Author`, `ChatMessage`, `OutboundRequest`, `FeatureId`)
- Generic type parameters use single uppercase letters (e.g., `T`, `E`) or descriptive names
- Private types are declared in modules and re-exported via `pub use` in lib.rs

**Enum variants:**
- Enum variants use `PascalCase` (e.g., `SendMessage`, `MessageCreate`, `Left`, `Right`)
- Snake_case serde renaming applied via `#[serde(rename_all = "snake_case")]` for JSON serialization compatibility

## Code Style

**Formatting:**
- Standard Rust formatting via implicit rustfmt (2021 edition)
- 4-space indentation (Rust standard)
- Brace style: Allman-style opening braces on same line for functions/blocks
- Line length: Generally < 100 characters, but no strict enforced limit

**Linting:**
- Default cargo clippy checks (no explicit .clippy.toml configuration found)
- Common patterns like `#[allow(dead_code)]` used when intentional
- Explicit derives used: `Clone`, `Debug`, `PartialEq`, `Serialize`, `Deserialize`, `ToSchema`
- Standard derives often combined in one line: `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]`

## Import Organization

**Order:**
1. Standard library imports (`use std::...`)
2. External crate imports (tokio, serde, axum, etc.)
3. Internal crate imports (`use crate::...`)
4. Module re-exports (`pub use ...`)

**Pattern from `river-worker/src/main.rs`:**
```rust
use clap::Parser;
use config::WorkerConfig;
use river_protocol::{WorkerRegistration, WorkerRegistrationRequest};
use http::router;
use river_adapter::Side;
use state::new_shared_state;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use worker_loop::run_loop;
```

**Path Aliases:**
- No path aliases used. All imports are absolute from crate root or external crates.

## Error Handling

**Pattern:**
- `thiserror` crate used for error enums with derive macros
- Error enums use `#[derive(Debug, thiserror::Error)]`
- Each error variant has a descriptive `#[error("...")]` message
- Errors are propagated using `?` operator, not `unwrap()`

**Example from `river-adapter/src/error.rs`:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("connection failed: {0}")]
    Connection(String),

    #[error("request timeout")]
    Timeout,

    #[error("feature not supported: {0:?}")]
    Unsupported(FeatureId),

    #[error("rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
}
```

**Custom error types:**
- `ParseError` defined as struct wrapper: `pub struct ParseError(pub String)` with manual `Error` trait implementation
- Errors implement `std::error::Error` trait
- IO errors wrapped and context added: `.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.0))`

**Result patterns:**
- Result types use explicit error type: `Result<T, E>` or `Result<T, io::Error>`
- Main functions return `Result<(), Box<dyn std::error::Error>>` for flexible error handling
- Early returns with `?` on registration failures in `main.rs`

## Logging

**Framework:** `tracing` crate with `tracing-subscriber`

**Initialization pattern from `river-worker/src/main.rs`:**
```rust
tracing_subscriber::registry()
    .with(tracing_subscriber::fmt::layer())
    .with(
        tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("river_worker=info".parse()?),
    )
    .init();
```

**Logging conventions:**
- Info level for important startup events: `tracing::info!("...")`
- Error level for failures: `tracing::error!("...")`
- Warning level for recoverable issues: `tracing::warn!(...)`
- Structured logging with named fields: `tracing::warn!(error = %e, "message")`
- Debug output uses `{:?}` for Debug formatting

## Comments

**When to Comment:**
- File-level documentation on crate modules: `//! Module description` at top of lib.rs
- Complex parsing logic documented with inline comments
- Intentional design decisions explained (e.g., "Reaction line" vs "Message line")
- Skip empty section comments unless explaining non-obvious control flow

**JSDoc/TSDoc:**
- Rust doc comments use `///` for public items
- Module docs use `//!` for module-level documentation
- Example from `river-adapter/src/lib.rs` includes usage examples in doc comments
- Doc examples use triple backticks with `rust` language tag

**Example pattern from `river-adapter/src/lib.rs`:**
```rust
//! River Adapter - Types-only library for adapter ↔ worker communication.
//!
//! This crate defines the interface between Workers and adapter binaries.
//! It exports types, traits, and enums — no HTTP infrastructure.
//!
//! # Feature System
//!
//! Two enums work together:
//! - [`FeatureId`]: Lightweight enum for registration and capability checks
//! - [`OutboundRequest`]: Data-carrying enum with typed payloads
```

## Function Design

**Size:**
- Functions generally 50-150 lines for complex operations (e.g., `format_line` in format.rs is ~100 lines)
- Larger functions (500+ lines) exist for complex parsing (e.g., `tools.rs` is 1318 lines for tool execution)
- No strict line limit, but readability prioritized with helper functions

**Parameters:**
- Use strong typing: specific enums/structs rather than strings where possible
- Optional parameters use `Option<T>` rather than default arguments
- References used for borrowed data: `&str`, `&Path`
- Owned data when mutation needed or to maintain ownership

**Return Values:**
- Use Result for fallible operations: `Result<T, E>`
- Void operations return `()` or `Result<(), E>` if they can fail
- Tuple returns used for multiple related values: `(meta, body)`
- Named fields in return enums for clarity: `SendMessage { channel, content, reply_to }`

## Module Design

**Exports:**
- Public modules declared in `lib.rs` with `pub mod name;`
- Selective re-exports via `pub use` to expose public API:
  ```rust
  pub use error::AdapterError;
  pub use event::{EventMetadata, EventType, InboundEvent};
  pub use feature::{FeatureId, OutboundRequest};
  ```
- Private implementation details hidden in modules

**Barrel Files:**
- Each crate has lib.rs serving as the main barrel file
- Sub-modules organized in src/ directory matching module structure
- Example: `river-adapter` has `lib.rs` with `mod event`, `mod feature`, `mod error`, etc.

**Module organization pattern:**
```rust
// lib.rs
mod error;
mod event;
mod feature;
mod response;
mod traits;

pub use error::AdapterError;
pub use event::{EventMetadata, EventType, InboundEvent};
// ... etc
```

## Serialization Conventions

**Serde usage:**
- All public types derive `Serialize` and `Deserialize`
- OpenAPI schema support via `utoipa::ToSchema` derive
- `#[serde(rename_all = "snake_case")]` on enums for JSON compatibility
- `#[serde(default, skip_serializing_if = "Option::is_none")]` for optional fields

**JSON naming:**
- Enums serialize as lowercase snake_case variants: `SendMessage` → `"send_message"`
- Struct fields use default naming (snake_case via rename_all)
- Type field for tagged enums: `#[serde(rename = "type")]` for adapter_type

## Testing Conventions

**Unit test placement:**
- Tests in same file as code: `#[cfg(test)] mod tests { #[test] fn test_...() { } }`
- Tests grouped at end of file or module

**Async test decoration:**
- Async tests use `#[tokio::test]` attribute
- Ignored expensive tests: `#[tokio::test] #[ignore]` with comment explaining why

---

*Convention analysis: 2026-04-06*

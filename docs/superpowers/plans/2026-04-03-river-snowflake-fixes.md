# river-snowflake Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix critical race condition in ID generation, eliminate blocking sleep, add graceful shutdown, and improve code quality with proper validation and deduplication.

**Architecture:** The `river-snowflake` crate provides 128-bit unique ID generation. The core `SnowflakeGenerator` uses atomics for thread-safe ID generation but has a race condition between timestamp load and store. The server wraps this in an HTTP API using axum.

**Tech Stack:** Rust, axum (HTTP server), tokio (async runtime), serde (serialization), thiserror (error handling)

---

## File Structure

| File | Responsibility | Changes |
|------|----------------|---------|
| `crates/river-snowflake/src/snowflake/generator.rs` | Thread-safe ID generation | Fix race condition with CAS loop, return error on overflow instead of blocking |
| `crates/river-snowflake/src/snowflake/birth.rs` | Agent birth timestamp | Add `try_from_u64` validation, fix bit-packing comment, remove duplicate `is_leap_year` |
| `crates/river-snowflake/src/snowflake/types.rs` | Snowflake type enum | Implement `FromStr` trait |
| `crates/river-snowflake/src/snowflake/mod.rs` | Module exports | Export new `is_leap_year` utility |
| `crates/river-snowflake/src/extract.rs` | Timestamp extraction | Use shared `is_leap_year` from birth module |
| `crates/river-snowflake/src/parse.rs` | Parsing/formatting | Implement `FromStr` for `Snowflake` |
| `crates/river-snowflake/src/lib.rs` | Crate root | Add `SequenceOverflow` error variant |
| `crates/river-snowflake/src/main.rs` | Server binary | Add graceful shutdown with signal handling |
| `crates/river-snowflake/src/server.rs` | HTTP endpoints | Add integration tests |
| `crates/river-snowflake/Cargo.toml` | Dependencies | Feature-gate clap behind `server` |

---

## Task 1: Fix Race Condition in SnowflakeGenerator (Critical)

**Files:**
- Modify: `crates/river-snowflake/src/snowflake/generator.rs:29-56`

- [ ] **Step 1: Add SequenceOverflow error to lib.rs**

In `crates/river-snowflake/src/lib.rs`, add new error variant:

```rust
/// Errors that can occur in snowflake operations.
#[derive(Debug, thiserror::Error)]
pub enum SnowflakeError {
    #[error("invalid format: {0}")]
    InvalidFormat(String),

    #[error("invalid birth: {0}")]
    InvalidBirth(String),

    #[error("invalid type: {0}")]
    InvalidType(String),

    #[error("sequence overflow: too many IDs generated in same microsecond")]
    SequenceOverflow,
}
```

- [ ] **Step 2: Rewrite next() method with CAS loop**

Replace the entire `next` method in `crates/river-snowflake/src/snowflake/generator.rs`:

```rust
    /// Generate a new snowflake ID.
    ///
    /// Returns `Err(SnowflakeError::SequenceOverflow)` if more than 0xFFFFF (1,048,575)
    /// IDs are generated in the same microsecond.
    pub fn next(&self, snowflake_type: SnowflakeType) -> Result<Snowflake, crate::SnowflakeError> {
        let now_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_micros() as u64;

        let relative_micros = now_micros.saturating_sub(self.birth_unix_micros);

        loop {
            let last = self.last_timestamp.load(Ordering::Acquire);

            if relative_micros > last {
                // New timestamp - try to claim it with CAS
                match self.last_timestamp.compare_exchange(
                    last,
                    relative_micros,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        // We won the race, reset sequence
                        self.sequence.store(0, Ordering::Release);
                        return Ok(Snowflake::new(relative_micros, self.birth, snowflake_type, 0));
                    }
                    Err(_) => {
                        // Lost race, retry with updated timestamp
                        continue;
                    }
                }
            } else {
                // Same or older timestamp, increment sequence
                let seq = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
                if seq > 0xFFFFF {
                    // Sequence overflow - return error instead of blocking
                    return Err(crate::SnowflakeError::SequenceOverflow);
                }
                return Ok(Snowflake::new(last, self.birth, snowflake_type, seq));
            }
        }
    }
```

- [ ] **Step 3: Update GeneratorCache to handle Result**

In `crates/river-snowflake/src/cache.rs`, update the methods:

```rust
    /// Generate single ID (creates generator for birth if needed).
    pub fn next_id(&self, birth: AgentBirth, snowflake_type: SnowflakeType) -> Result<Snowflake, crate::SnowflakeError> {
        let gen = self.get_or_create(birth);
        gen.next(snowflake_type)
    }

    /// Generate multiple IDs.
    pub fn next_ids(
        &self,
        birth: AgentBirth,
        snowflake_type: SnowflakeType,
        count: usize,
    ) -> Result<Vec<Snowflake>, crate::SnowflakeError> {
        let gen = self.get_or_create(birth);
        (0..count).map(|_| gen.next(snowflake_type)).collect()
    }
```

- [ ] **Step 4: Update cache tests**

In `crates/river-snowflake/src/cache.rs`, update tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_reuses_generators() {
        let cache = GeneratorCache::new();
        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();

        let _id1 = cache.next_id(birth, SnowflakeType::Message).unwrap();
        let _id2 = cache.next_id(birth, SnowflakeType::Message).unwrap();

        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_different_births() {
        let cache = GeneratorCache::new();
        let birth1 = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let birth2 = AgentBirth::new(2026, 4, 2, 12, 0, 0).unwrap();

        let _id1 = cache.next_id(birth1, SnowflakeType::Message).unwrap();
        let _id2 = cache.next_id(birth2, SnowflakeType::Message).unwrap();

        assert_eq!(cache.len(), 2);
    }
}
```

- [ ] **Step 5: Update server to handle Result**

In `crates/river-snowflake/src/server.rs`, update `get_id`:

```rust
/// GET /id/{type}?birth={birth}
async fn get_id(
    State(state): State<Arc<AppState>>,
    Path(type_str): Path<String>,
    Query(query): Query<IdQuery>,
) -> impl IntoResponse {
    let Some(snowflake_type) = SnowflakeType::from_str(&type_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid type: {}", type_str),
            }),
        )
            .into_response();
    };

    let birth = AgentBirth::from_u64(query.birth);
    match state.cache.next_id(birth, snowflake_type) {
        Ok(id) => (StatusCode::OK, Json(IdResponse { id: id.to_string() })).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

- [ ] **Step 6: Update server post_ids to handle Result**

In `crates/river-snowflake/src/server.rs`, update `post_ids`:

```rust
/// POST /ids
async fn post_ids(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchRequest>,
) -> impl IntoResponse {
    let Some(snowflake_type) = SnowflakeType::from_str(&req.snowflake_type) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid type: {}", req.snowflake_type),
            }),
        )
            .into_response();
    };

    if req.count > 10000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "count must be <= 10000".into(),
            }),
        )
            .into_response();
    }

    let birth = AgentBirth::from_u64(req.birth);
    match state.cache.next_ids(birth, snowflake_type, req.count) {
        Ok(ids) => {
            let ids = ids.into_iter().map(|id| id.to_string()).collect();
            (StatusCode::OK, Json(BatchResponse { ids })).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

- [ ] **Step 7: Update generator tests**

In `crates/river-snowflake/src/snowflake/generator.rs`, update existing tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generates_unique_ids() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let id1 = gen.next(SnowflakeType::Message).unwrap();
        let id2 = gen.next(SnowflakeType::Message).unwrap();

        assert_ne!(id1, id2);
    }

    #[test]
    fn test_extracts_birth() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let id = gen.next(SnowflakeType::Message).unwrap();
        let extracted = id.birth();

        assert_eq!(extracted.year(), 2026);
        assert_eq!(extracted.month(), 4);
        assert_eq!(extracted.day(), 1);
    }
}
```

- [ ] **Step 8: Add concurrent generation test**

In `crates/river-snowflake/src/snowflake/generator.rs`, add new test:

```rust
    #[test]
    fn test_concurrent_generation() {
        use std::collections::HashSet;
        use std::sync::Arc;
        use std::thread;

        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let gen = Arc::new(SnowflakeGenerator::new(birth));
        let num_threads = 8;
        let ids_per_thread = 1000;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let gen = Arc::clone(&gen);
                thread::spawn(move || {
                    let mut ids = Vec::with_capacity(ids_per_thread);
                    for _ in 0..ids_per_thread {
                        if let Ok(id) = gen.next(SnowflakeType::Message) {
                            ids.push(id);
                        }
                    }
                    ids
                })
            })
            .collect();

        let mut all_ids = HashSet::new();
        for handle in handles {
            for id in handle.join().unwrap() {
                let key = (id.high(), id.low());
                assert!(
                    all_ids.insert(key),
                    "Duplicate ID detected: {:016x}-{:016x}",
                    id.high(),
                    id.low()
                );
            }
        }
    }
```

- [ ] **Step 9: Run tests**

Run: `cargo test -p river-snowflake`

Expected: All tests pass, including new concurrent generation test

- [ ] **Step 10: Commit**

```bash
git add crates/river-snowflake/src/lib.rs crates/river-snowflake/src/snowflake/generator.rs crates/river-snowflake/src/cache.rs crates/river-snowflake/src/server.rs && git commit -m "fix(river-snowflake): resolve race condition with CAS loop and return error on sequence overflow

- Replace load/store with compare_exchange loop to prevent duplicate IDs
- Return SequenceOverflow error instead of blocking thread::sleep
- Fix off-by-one in sequence overflow check (>= to >)
- Add concurrent generation test with 8 threads
- Update GeneratorCache and server to handle Result types"
```

---

## Task 2: Add Graceful Shutdown

**Files:**
- Modify: `crates/river-snowflake/src/main.rs:22-40`

- [ ] **Step 1: Add tracing dependency**

First check if tracing is already available. If not, add to `crates/river-snowflake/Cargo.toml`:

```toml
tracing = { workspace = true, optional = true }
```

And update server feature:

```toml
server = ["dep:axum", "dep:tokio", "dep:tracing"]
```

- [ ] **Step 2: Implement graceful shutdown**

Replace the main function in `crates/river-snowflake/src/main.rs`:

```rust
//! River Snowflake server binary.

use std::sync::Arc;

use clap::Parser;
use river_snowflake::{server, GeneratorCache};
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(name = "river-snowflake")]
#[command(about = "Snowflake ID generation server")]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "4001")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let state = Arc::new(server::AppState {
        cache: GeneratorCache::new(),
    });

    let app = server::router(state);

    let addr = format!("{}:{}", args.host, args.port);
    let listener = TcpListener::bind(&addr).await?;

    eprintln!("Snowflake server listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    eprintln!("Snowflake server shut down gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            eprintln!("\nReceived Ctrl+C, shutting down...");
        }
        _ = terminate => {
            eprintln!("\nReceived SIGTERM, shutting down...");
        }
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p river-snowflake --features server`

Expected: Build succeeds

- [ ] **Step 4: Commit**

```bash
git add crates/river-snowflake/src/main.rs crates/river-snowflake/Cargo.toml && git commit -m "feat(river-snowflake): add graceful shutdown on SIGINT/SIGTERM

- Handle Ctrl+C and SIGTERM signals
- Use axum's with_graceful_shutdown for clean connection draining
- Log shutdown events to stderr"
```

---

## Task 3: Add AgentBirth Validation

**Files:**
- Modify: `crates/river-snowflake/src/snowflake/birth.rs:117-119`

- [ ] **Step 1: Add try_from_u64 with validation**

In `crates/river-snowflake/src/snowflake/birth.rs`, add after `from_u64`:

```rust
    /// Create from raw packed value with validation.
    ///
    /// Returns an error if the packed value represents an invalid date/time.
    pub fn try_from_u64(value: u64) -> Result<Self, crate::SnowflakeError> {
        let birth = Self(value);

        // Extract and validate components
        let year = birth.year();
        let month = birth.month();
        let day = birth.day();
        let hour = birth.hour();
        let minute = birth.minute();
        let second = birth.second();

        if year < 2000 || year > 3023 {
            return Err(crate::SnowflakeError::InvalidBirth(format!(
                "year {} out of range (2000-3023)",
                year
            )));
        }
        if month < 1 || month > 12 {
            return Err(crate::SnowflakeError::InvalidBirth(format!(
                "month {} out of range (1-12)",
                month
            )));
        }
        if day < 1 || day > 31 {
            return Err(crate::SnowflakeError::InvalidBirth(format!(
                "day {} out of range (1-31)",
                day
            )));
        }
        if hour > 23 {
            return Err(crate::SnowflakeError::InvalidBirth(format!(
                "hour {} out of range (0-23)",
                hour
            )));
        }
        if minute > 59 {
            return Err(crate::SnowflakeError::InvalidBirth(format!(
                "minute {} out of range (0-59)",
                minute
            )));
        }
        if second > 59 {
            return Err(crate::SnowflakeError::InvalidBirth(format!(
                "second {} out of range (0-59)",
                second
            )));
        }

        Ok(birth)
    }

    /// Create from raw packed value without validation.
    ///
    /// # Safety
    /// The caller must ensure the packed value represents a valid date/time.
    /// Use `try_from_u64` for validated parsing.
    #[inline]
    pub fn from_u64_unchecked(value: u64) -> Self {
        Self(value)
    }
```

- [ ] **Step 2: Deprecate from_u64**

In `crates/river-snowflake/src/snowflake/birth.rs`, update `from_u64`:

```rust
    /// Create from raw packed value.
    ///
    /// **Deprecated:** Use `try_from_u64` for validation or `from_u64_unchecked` if you know the value is valid.
    #[deprecated(since = "0.2.0", note = "Use try_from_u64 for validation or from_u64_unchecked")]
    pub fn from_u64(value: u64) -> Self {
        Self(value)
    }
```

- [ ] **Step 3: Add validation tests**

In `crates/river-snowflake/src/snowflake/birth.rs`, add tests:

```rust
    #[test]
    fn test_try_from_u64_valid() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 30, 45).unwrap();
        let packed = birth.as_u64();
        let restored = AgentBirth::try_from_u64(packed).unwrap();
        assert_eq!(birth, restored);
    }

    #[test]
    fn test_try_from_u64_invalid_month() {
        // Pack a value with month = 0 (invalid)
        let invalid = 0u64; // year=2000, month=0, day=0, etc.
        let result = AgentBirth::try_from_u64(invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_from_u64_garbage() {
        // Maximum u64 would have year = 2000 + 1023 = 3023, month = 15 (invalid)
        let garbage = u64::MAX;
        let result = AgentBirth::try_from_u64(garbage);
        assert!(result.is_err());
    }
```

- [ ] **Step 4: Update server to use try_from_u64**

In `crates/river-snowflake/src/server.rs`, update `get_id`:

```rust
/// GET /id/{type}?birth={birth}
async fn get_id(
    State(state): State<Arc<AppState>>,
    Path(type_str): Path<String>,
    Query(query): Query<IdQuery>,
) -> impl IntoResponse {
    let Some(snowflake_type) = SnowflakeType::from_str(&type_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid type: {}", type_str),
            }),
        )
            .into_response();
    };

    let birth = match AgentBirth::try_from_u64(query.birth) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    match state.cache.next_id(birth, snowflake_type) {
        Ok(id) => (StatusCode::OK, Json(IdResponse { id: id.to_string() })).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

- [ ] **Step 5: Update post_ids to use try_from_u64**

In `crates/river-snowflake/src/server.rs`, update `post_ids`:

```rust
/// POST /ids
async fn post_ids(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchRequest>,
) -> impl IntoResponse {
    let Some(snowflake_type) = SnowflakeType::from_str(&req.snowflake_type) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid type: {}", req.snowflake_type),
            }),
        )
            .into_response();
    };

    if req.count > 10000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "count must be <= 10000".into(),
            }),
        )
            .into_response();
    }

    let birth = match AgentBirth::try_from_u64(req.birth) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    match state.cache.next_ids(birth, snowflake_type, req.count) {
        Ok(ids) => {
            let ids = ids.into_iter().map(|id| id.to_string()).collect();
            (StatusCode::OK, Json(BatchResponse { ids })).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p river-snowflake`

Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/river-snowflake/src/snowflake/birth.rs crates/river-snowflake/src/server.rs && git commit -m "feat(river-snowflake): add AgentBirth validation with try_from_u64

- Add try_from_u64 that validates all date/time components
- Add from_u64_unchecked for performance-critical paths
- Deprecate from_u64 in favor of explicit methods
- Update server endpoints to validate birth parameter
- Add tests for invalid birth values"
```

---

## Task 4: Deduplicate is_leap_year

**Files:**
- Modify: `crates/river-snowflake/src/snowflake/birth.rs:196-198`
- Modify: `crates/river-snowflake/src/snowflake/mod.rs`
- Modify: `crates/river-snowflake/src/extract.rs:60-62`

- [ ] **Step 1: Make is_leap_year public in birth.rs**

In `crates/river-snowflake/src/snowflake/birth.rs`, change:

```rust
pub(crate) fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}
```

- [ ] **Step 2: Export is_leap_year from mod.rs**

In `crates/river-snowflake/src/snowflake/mod.rs`, add:

```rust
//! Core snowflake types.

mod birth;
mod generator;
mod id;
mod types;

pub use birth::AgentBirth;
pub(crate) use birth::is_leap_year;
pub use generator::SnowflakeGenerator;
pub use id::Snowflake;
pub use types::SnowflakeType;
```

- [ ] **Step 3: Use shared is_leap_year in extract.rs**

In `crates/river-snowflake/src/extract.rs`, remove the local function and add import:

```rust
//! Timestamp extraction from snowflakes.

use crate::snowflake::is_leap_year;
use crate::Snowflake;
```

Then delete lines 60-62 (the duplicate `is_leap_year` function).

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-snowflake`

Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-snowflake/src/snowflake/birth.rs crates/river-snowflake/src/snowflake/mod.rs crates/river-snowflake/src/extract.rs && git commit -m "refactor(river-snowflake): deduplicate is_leap_year function

- Move is_leap_year to birth.rs as pub(crate)
- Export from snowflake/mod.rs
- Remove duplicate from extract.rs"
```

---

## Task 5: Fix Bit-Packing Comment

**Files:**
- Modify: `crates/river-snowflake/src/snowflake/birth.rs:43-48`

- [ ] **Step 1: Update the misleading comment**

In `crates/river-snowflake/src/snowflake/birth.rs`, replace the comment block:

```rust
        // Pack: [year_offset:10][month:4][day:5][hour:5][minute:6][second:6] = 36 bits
        // year_offset = year - 2000, supports years 2000-3023
        let year_offset = (year - 2000) as u64;
```

- [ ] **Step 2: Commit**

```bash
git add crates/river-snowflake/src/snowflake/birth.rs && git commit -m "docs(river-snowflake): fix bit-packing comment to match implementation

The comment incorrectly said 38 bits, but implementation uses 36 bits:
- year_offset: 10 bits (0-1023, representing 2000-3023)
- month: 4 bits
- day: 5 bits
- hour: 5 bits
- minute: 6 bits
- second: 6 bits
Total: 36 bits"
```

---

## Task 6: Implement FromStr Traits

**Files:**
- Modify: `crates/river-snowflake/src/snowflake/types.rs`
- Modify: `crates/river-snowflake/src/parse.rs`

- [ ] **Step 1: Implement FromStr for SnowflakeType**

In `crates/river-snowflake/src/snowflake/types.rs`, add:

```rust
//! Snowflake type identifier enum.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::SnowflakeError;

/// 8-bit type identifier for snowflake IDs.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnowflakeType {
    Message = 0x01,
    Embedding = 0x02,
    Session = 0x03,
    Subagent = 0x04,
    ToolCall = 0x05,
    Context = 0x06,
    Flash = 0x07,
    Move = 0x08,
    Moment = 0x09,
}

impl SnowflakeType {
    /// Parse from string (lowercase).
    #[deprecated(since = "0.2.0", note = "Use FromStr trait instead")]
    pub fn from_str(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    /// Convert to lowercase string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Embedding => "embedding",
            Self::Session => "session",
            Self::Subagent => "subagent",
            Self::ToolCall => "tool_call",
            Self::Context => "context",
            Self::Flash => "flash",
            Self::Move => "move",
            Self::Moment => "moment",
        }
    }
}

impl FromStr for SnowflakeType {
    type Err = SnowflakeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "message" => Ok(Self::Message),
            "embedding" => Ok(Self::Embedding),
            "session" => Ok(Self::Session),
            "subagent" => Ok(Self::Subagent),
            "tool_call" => Ok(Self::ToolCall),
            "context" => Ok(Self::Context),
            "flash" => Ok(Self::Flash),
            "move" => Ok(Self::Move),
            "moment" => Ok(Self::Moment),
            _ => Err(SnowflakeError::InvalidType(s.to_string())),
        }
    }
}

impl fmt::Display for SnowflakeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fromstr_roundtrip() {
        for variant in [
            SnowflakeType::Message,
            SnowflakeType::Embedding,
            SnowflakeType::Session,
            SnowflakeType::Subagent,
            SnowflakeType::ToolCall,
            SnowflakeType::Context,
            SnowflakeType::Flash,
            SnowflakeType::Move,
            SnowflakeType::Moment,
        ] {
            let s = variant.to_string();
            let parsed: SnowflakeType = s.parse().unwrap();
            assert_eq!(variant, parsed);
        }
    }

    #[test]
    fn test_fromstr_invalid() {
        let result: Result<SnowflakeType, _> = "invalid".parse();
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Implement FromStr for Snowflake**

In `crates/river-snowflake/src/parse.rs`, add:

```rust
//! Parsing and formatting snowflake IDs.

use std::str::FromStr;

use crate::{Snowflake, SnowflakeError};

/// Parse a hex string "high-low" to Snowflake.
#[deprecated(since = "0.2.0", note = "Use FromStr trait instead")]
pub fn parse(s: &str) -> Result<Snowflake, SnowflakeError> {
    s.parse()
}

/// Format Snowflake as hex string "high-low".
pub fn format(id: &Snowflake) -> String {
    id.to_string()
}

impl FromStr for Snowflake {
    type Err = SnowflakeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 2 {
            return Err(SnowflakeError::InvalidFormat(
                "expected format: high-low".into(),
            ));
        }

        let high = u64::from_str_radix(parts[0], 16)
            .map_err(|_| SnowflakeError::InvalidFormat("invalid high component".into()))?;
        let low = u64::from_str_radix(parts[1], 16)
            .map_err(|_| SnowflakeError::InvalidFormat("invalid low component".into()))?;

        Ok(Snowflake { high, low })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fromstr_roundtrip() {
        let original = "0000000000123456-1a2b3c4d5e6f7890";
        let parsed: Snowflake = original.parse().unwrap();
        let formatted = parsed.to_string();
        assert_eq!(formatted, original);
    }

    #[test]
    fn test_fromstr_invalid() {
        assert!("invalid".parse::<Snowflake>().is_err());
        assert!("abc-def-ghi".parse::<Snowflake>().is_err());
        assert!("zzzz-0000".parse::<Snowflake>().is_err());
    }

    #[test]
    #[allow(deprecated)]
    fn test_parse_format_roundtrip() {
        let original = "0000000000123456-1a2b3c4d5e6f7890";
        let parsed = parse(original).unwrap();
        let formatted = format(&parsed);
        assert_eq!(formatted, original);
    }

    #[test]
    #[allow(deprecated)]
    fn test_parse_invalid() {
        assert!(parse("invalid").is_err());
        assert!(parse("abc-def-ghi").is_err());
        assert!(parse("zzzz-0000").is_err());
    }
}
```

- [ ] **Step 3: Update server to use FromStr**

In `crates/river-snowflake/src/server.rs`, update imports and handlers:

```rust
//! HTTP server for snowflake generation.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{AgentBirth, GeneratorCache, SnowflakeType};
```

Update `get_id` to use `parse()`:

```rust
/// GET /id/{type}?birth={birth}
async fn get_id(
    State(state): State<Arc<AppState>>,
    Path(type_str): Path<String>,
    Query(query): Query<IdQuery>,
) -> impl IntoResponse {
    let snowflake_type: SnowflakeType = match type_str.parse() {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    let birth = match AgentBirth::try_from_u64(query.birth) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    match state.cache.next_id(birth, snowflake_type) {
        Ok(id) => (StatusCode::OK, Json(IdResponse { id: id.to_string() })).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

Update `post_ids` similarly:

```rust
/// POST /ids
async fn post_ids(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchRequest>,
) -> impl IntoResponse {
    let snowflake_type: SnowflakeType = match req.snowflake_type.parse() {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    if req.count > 10000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "count must be <= 10000".into(),
            }),
        )
            .into_response();
    }

    let birth = match AgentBirth::try_from_u64(req.birth) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    match state.cache.next_ids(birth, snowflake_type, req.count) {
        Ok(ids) => {
            let ids = ids.into_iter().map(|id| id.to_string()).collect();
            (StatusCode::OK, Json(BatchResponse { ids })).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-snowflake`

Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-snowflake/src/snowflake/types.rs crates/river-snowflake/src/parse.rs crates/river-snowflake/src/server.rs && git commit -m "feat(river-snowflake): implement FromStr and Display traits

- Implement FromStr for SnowflakeType with proper error handling
- Implement FromStr for Snowflake
- Add Display for SnowflakeType
- Deprecate manual from_str/parse functions
- Update server to use standard traits"
```

---

## Task 7: Feature-Gate clap

**Files:**
- Modify: `crates/river-snowflake/Cargo.toml:20`

- [ ] **Step 1: Make clap optional**

In `crates/river-snowflake/Cargo.toml`, update:

```toml
[package]
name = "river-snowflake"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "128-bit snowflake ID generation for River Engine"

[features]
default = []
server = ["dep:axum", "dep:tokio", "dep:clap"]

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }

axum = { workspace = true, optional = true }
tokio = { workspace = true, optional = true }
clap = { workspace = true, optional = true }

[[bin]]
name = "river-snowflake"
required-features = ["server"]
```

- [ ] **Step 2: Verify library builds without server feature**

Run: `cargo build -p river-snowflake`

Expected: Build succeeds without clap

- [ ] **Step 3: Verify server builds with feature**

Run: `cargo build -p river-snowflake --features server`

Expected: Build succeeds with clap

- [ ] **Step 4: Commit**

```bash
git add crates/river-snowflake/Cargo.toml && git commit -m "build(river-snowflake): feature-gate clap behind server feature

clap is only needed for the binary, not the library. This reduces
compile time and binary size for library consumers."
```

---

## Task 8: Add Server Integration Tests

**Files:**
- Create: `crates/river-snowflake/tests/server_tests.rs`

- [ ] **Step 1: Create integration test file**

Create `crates/river-snowflake/tests/server_tests.rs`:

```rust
//! Integration tests for river-snowflake HTTP server.

#![cfg(feature = "server")]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use river_snowflake::{server, AgentBirth, GeneratorCache};
use tower::ServiceExt;

fn create_app() -> axum::Router {
    let state = Arc::new(server::AppState {
        cache: GeneratorCache::new(),
    });
    server::router(state)
}

fn valid_birth() -> u64 {
    AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap().as_u64()
}

#[tokio::test]
async fn test_get_id_success() {
    let app = create_app();
    let birth = valid_birth();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/id/message?birth={}", birth))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(json.get("id").is_some());
    let id = json["id"].as_str().unwrap();
    assert!(id.contains('-'), "ID should be in high-low format");
}

#[tokio::test]
async fn test_get_id_invalid_type() {
    let app = create_app();
    let birth = valid_birth();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/id/invalid_type?birth={}", birth))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_get_id_invalid_birth() {
    let app = create_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/id/message?birth=18446744073709551615") // u64::MAX
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_post_ids_success() {
    let app = create_app();
    let birth = valid_birth();

    let body = serde_json::json!({
        "birth": birth,
        "type": "session",
        "count": 5
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ids")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let ids = json["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 5);
}

#[tokio::test]
async fn test_post_ids_count_too_large() {
    let app = create_app();
    let birth = valid_birth();

    let body = serde_json::json!({
        "birth": birth,
        "type": "session",
        "count": 10001
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ids")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_health_endpoint() {
    let app = create_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok");
    assert!(json.get("generators").is_some());
}

#[tokio::test]
async fn test_all_snowflake_types() {
    let app = create_app();
    let birth = valid_birth();

    let types = [
        "message",
        "embedding",
        "session",
        "subagent",
        "tool_call",
        "context",
        "flash",
        "move",
        "moment",
    ];

    for snowflake_type in types {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/id/{}?birth={}", snowflake_type, birth))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Failed for type: {}",
            snowflake_type
        );
    }
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test -p river-snowflake --features server`

Expected: All integration tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-snowflake/tests/server_tests.rs && git commit -m "test(river-snowflake): add HTTP endpoint integration tests

- Test GET /id/{type} success and error cases
- Test POST /ids success and validation
- Test GET /health endpoint
- Test all snowflake types
- Tests require server feature"
```

---

## Task 9: Final Verification

- [ ] **Step 1: Run all tests**

Run: `cargo test -p river-snowflake --features server`

Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p river-snowflake --features server -- -D warnings`

Expected: No warnings

- [ ] **Step 3: Check formatting**

Run: `cargo fmt -p river-snowflake -- --check`

Expected: No formatting issues

- [ ] **Step 4: Build release**

Run: `cargo build -p river-snowflake --features server --release`

Expected: Build succeeds

- [ ] **Step 5: Final commit if any fixes needed**

If any fixes were needed:
```bash
git add -A && git commit -m "chore(river-snowflake): address clippy and formatting issues"
```

---

## Summary

| Task | Priority | Estimated Time |
|------|----------|----------------|
| 1. Fix Race Condition | Critical | 30 min |
| 2. Add Graceful Shutdown | Critical | 10 min |
| 3. Add AgentBirth Validation | Important | 15 min |
| 4. Deduplicate is_leap_year | Important | 5 min |
| 5. Fix Bit-Packing Comment | Minor | 2 min |
| 6. Implement FromStr Traits | Minor | 15 min |
| 7. Feature-Gate clap | Important | 5 min |
| 8. Add Server Tests | Important | 20 min |
| 9. Final Verification | Required | 10 min |

**Total Estimated Time:** ~2 hours

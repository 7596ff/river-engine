# Snowflake Server — Design Spec

> river-snowflake: Library + binary for snowflake ID generation
>
> Authors: Cass, Claude
> Date: 2026-04-01

## Overview

The snowflake server provides unique 128-bit IDs to all components in the River system. It exposes an HTTP API for ID generation and a library for parsing, formatting, and constructing snowflakes.

**Key characteristics:**
- Self-contained crate (includes all snowflake types)
- Library for parsing/formatting without server overhead
- HTTP server feature-gated behind `server` feature
- Generators cached by birth, stateless across restarts

## Snowflake Format

128-bit ID with the following layout:
- **64 bits (high):** Timestamp in microseconds since agent birth
- **36 bits:** Agent birth (packed yyyymmddhhmmss)
- **8 bits:** Type identifier
- **20 bits:** Sequence number

**Hex string format:** `{high:016x}-{low:016x}` (33 characters)

Example: `0000000000123456-1a2b3c4d5e6f7890`

## Library API

### Core Types

```rust
/// 128-bit unique identifier
pub struct Snowflake {
    high: u64,  // timestamp micros since birth
    low: u64,   // [birth:36][type:8][sequence:20]
}

/// 36-bit packed birth timestamp
pub struct AgentBirth(u64);

/// 8-bit type identifier
pub enum SnowflakeType {
    Message = 0x01,
    Embedding = 0x02,
    Session = 0x03,
    Subagent = 0x04,
    ToolCall = 0x05,
    Context = 0x06,
}

/// Thread-safe generator for a single birth
pub struct SnowflakeGenerator { ... }
```

### Parsing and Formatting

```rust
/// Parse hex string "high-low" to Snowflake
pub fn parse(s: &str) -> Result<Snowflake, ParseError>;

/// Format Snowflake as hex string
pub fn format(id: &Snowflake) -> String;
```

### Extraction

```rust
/// Extract ISO8601 timestamp from Snowflake
/// Combines birth + relative timestamp to produce absolute time
pub fn timestamp_iso8601(id: &Snowflake) -> String;
```

### Generator Cache

```rust
/// Cache of generators keyed by AgentBirth
pub struct GeneratorCache { ... }

impl GeneratorCache {
    /// Create empty cache
    pub fn new() -> Self;

    /// Generate single ID (creates generator for birth if needed)
    pub fn next_id(&self, birth: AgentBirth, snowflake_type: SnowflakeType) -> Snowflake;

    /// Generate multiple IDs
    pub fn next_ids(
        &self,
        birth: AgentBirth,
        snowflake_type: SnowflakeType,
        count: usize,
    ) -> Vec<Snowflake>;
}
```

The cache maintains a `HashMap<AgentBirth, SnowflakeGenerator>` internally. Generators are created on first request for a given birth and reused thereafter.

## HTTP API

### Single ID

```
GET /id/{type}?birth={birth}
```

**Path parameters:**
- `type` — One of: `message`, `embedding`, `session`, `subagent`, `tool_call`, `context`

**Query parameters:**
- `birth` — AgentBirth as u64 (36-bit packed value)

**Response 200:**
```json
{
  "id": "0000000000123456-1a2b3c4d5e6f7890"
}
```

**Response 400:**
```json
{
  "error": "invalid type: foo"
}
```

### Batch IDs

```
POST /ids
```

**Request body:**
```json
{
  "birth": 123456789,
  "type": "message",
  "count": 10
}
```

**Response 200:**
```json
{
  "ids": [
    "0000000000123456-1a2b3c4d5e6f7890",
    "0000000000123456-1a2b3c4d5e6f7891",
    "..."
  ]
}
```

### Health

```
GET /health
```

**Response 200:**
```json
{
  "status": "ok",
  "generators": 3
}
```

The `generators` field indicates how many distinct births have been seen.

## Crate Structure

```
river-snowflake/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Re-exports, public API
│   ├── parse.rs            # parse(), format()
│   ├── extract.rs          # timestamp_iso8601()
│   ├── cache.rs            # GeneratorCache
│   │
│   ├── snowflake/          # Core types
│   │   ├── mod.rs
│   │   ├── id.rs           # Snowflake struct
│   │   ├── birth.rs        # AgentBirth
│   │   ├── types.rs        # SnowflakeType enum
│   │   └── generator.rs    # SnowflakeGenerator
│   │
│   ├── server.rs           # HTTP handlers (feature-gated)
│   └── main.rs             # CLI binary
```

## Dependencies

**Library (always):**
- `serde` + `serde_json` — serialization
- `thiserror` — error types

**Server (feature-gated):**
- `axum` — HTTP framework
- `tokio` — async runtime

**Binary:**
- `clap` — CLI argument parsing

## Cargo.toml Features

```toml
[features]
default = []
server = ["dep:axum", "dep:tokio"]

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
axum = { workspace = true, optional = true }
tokio = { workspace = true, optional = true }

[dependencies.clap]
version = "4.0"
features = ["derive"]
```

## CLI

```
river-snowflake [OPTIONS]

Options:
  -p, --port <PORT>  Port to listen on [default: 4001]
      --host <HOST>  Host to bind to [default: 127.0.0.1]
  -h, --help         Print help
```

**Startup:**
1. Parse CLI arguments
2. Create empty `GeneratorCache`
3. Bind HTTP server to `{host}:{port}`
4. Log: `"Snowflake server listening on {host}:{port}"`

**Shutdown:**
- Graceful on SIGINT/SIGTERM
- No persistence (generators are ephemeral)

## Restart Behavior

On server restart, cached generators are lost. This is acceptable because:
1. Timestamps are microsecond-precision
2. Restart takes longer than one microsecond
3. New IDs will have later timestamps, guaranteeing uniqueness

No persistence layer is needed.

## Usage Examples

### Library (parsing)

```rust
use river_snowflake::{parse, format, timestamp_iso8601};

let id = parse("0000000000123456-1a2b3c4d5e6f7890")?;
let timestamp = timestamp_iso8601(&id);  // "2026-04-01T12:34:56.789012Z"
let hex = format(&id);  // "0000000000123456-1a2b3c4d5e6f7890"
```

### Library (embedded generation)

```rust
use river_snowflake::{GeneratorCache, AgentBirth, SnowflakeType};

let cache = GeneratorCache::new();
let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0)?;
let id = cache.next_id(birth, SnowflakeType::Message);
```

### HTTP Client

```bash
# Single ID
curl "http://localhost:4001/id/message?birth=123456789"

# Batch IDs
curl -X POST http://localhost:4001/ids \
  -H "Content-Type: application/json" \
  -d '{"birth": 123456789, "type": "message", "count": 10}'
```

## Related Documents

- `docs/archive/river-core/src/snowflake/` — Original snowflake implementation
- `docs/superpowers/specs/2026-04-01-context-management-design.md` — Uses snowflake IDs

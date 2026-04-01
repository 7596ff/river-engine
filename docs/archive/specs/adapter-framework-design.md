# Adapter Framework Design

> Design spec for River Engine's communication adapter system
>
> Brainstorm session: 2026-03-23
> Authors: Cass, Claude

## Overview

A generic adapter framework for connecting River to communication platforms. Discord is the reference implementation.

**Philosophy:** Rust types as source of truth. OpenAPI generated from types. Adapter-specific features stay close to their native APIs — no forced normalization.

**Core principle:** Every adapter can send and receive. Everything else is a feature flag.

---

## Library Structure

New crate: `river-adapter`

```
crates/river-adapter/
├── Cargo.toml
├── src/
│   ├── lib.rs           # Exports, OpenAPI doc generation
│   ├── types.rs         # Core message types
│   ├── capabilities.rs  # Feature flags
│   ├── trait.rs         # Adapter trait for gateway
│   ├── registration.rs  # Self-registration types
│   └── error.rs         # Shared error types
└── openapi.json         # Generated, committed
```

**Dependencies:**
- `serde` — serialization
- `utoipa` — OpenAPI generation
- `thiserror` — error types

**Consumers:**
- `river-gateway` — uses trait + types
- `river-discord` — uses types for request/response

---

## Core Types

### Events (Adapter → Gateway)

```rust
/// Incoming event from adapter to gateway
#[derive(Serialize, Deserialize, ToSchema)]
pub struct IncomingEvent {
    pub adapter: String,
    pub event_type: EventType,
    pub channel: String,
    pub channel_name: Option<String>,
    pub author: Author,
    pub content: String,
    pub message_id: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: serde_json::Value,  // Native platform structure
}

#[derive(Serialize, Deserialize, ToSchema)]
pub enum EventType {
    MessageCreate,
    MessageUpdate,
    MessageDelete,
    ReactionAdd,
    ReactionRemove,
    Identify(String),  // User identity events (name changes, profile updates)
    Custom(String),
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub is_bot: bool,
}
```

### Messages (Gateway → Adapter)

```rust
/// Outgoing message from gateway to adapter
#[derive(Serialize, Deserialize, ToSchema)]
pub struct SendRequest {
    pub channel: String,
    pub content: String,
    pub options: SendOptions,
}

#[derive(Serialize, Deserialize, ToSchema, Default)]
pub struct SendOptions {
    pub reply_to: Option<String>,
    pub thread_id: Option<String>,
    pub metadata: serde_json::Value,  // Adapter-specific options
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SendResponse {
    pub success: bool,
    pub message_id: Option<String>,
    pub error: Option<String>,
}
```

### Metadata Stays Native

Adapter-specific data preserves platform structure:

```json
// Discord
{
    "adapter": "discord",
    "event_type": "MessageCreate",
    "metadata": {
        "guild_id": "789",
        "attachments": [...],
        "embeds": [...]
    }
}

// Slack
{
    "adapter": "slack",
    "event_type": "MessageCreate",
    "metadata": {
        "workspace": "myteam",
        "blocks": [...],
        "thread_ts": "..."
    }
}
```

Gateway treats metadata as opaque. Agent sees native structure if needed.

**Note:** Metadata validation happens at runtime, not compile time. If this becomes a pain point, consider adding `validate_metadata()` to the trait.

---

## Feature Flags

Send and receive are fundamental — not features, just what adapters *are*. Everything else is optional.

```rust
#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Eq, Hash)]
pub enum Feature {
    ReadHistory,
    Reactions,
    Threads,
    Attachments,
    Embeds,
    TypingIndicator,
    EditMessage,
    DeleteMessage,
    Custom(String),
}
```

Adapters declare features on registration. Gateway checks before attempting operations:

```rust
if adapter.supports(&Feature::Reactions) {
    // safe to send reaction
} else {
    // return "adapter doesn't support reactions"
}
```

---

## Registration & Discovery

### Self-Registration

Adapters register with gateway on startup:

```rust
#[derive(Serialize, Deserialize, ToSchema)]
pub struct AdapterInfo {
    pub name: String,
    pub version: String,
    pub url: String,
    pub features: HashSet<Feature>,
    pub metadata: serde_json::Value,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct RegisterRequest {
    pub adapter: AdapterInfo,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct RegisterResponse {
    pub accepted: bool,
    pub error: Option<String>,
}
```

### Flow

```
Adapter starts
    │
    ▼
POST gateway/adapters/register
    │
    ▼
Gateway stores in AdapterRegistry
    │
    ▼
Adapter ready to receive events
```

If adapter restarts, it re-registers. Gateway updates the entry.

**Gateway restart:** For now, gateway restart means adapter restart too. Adapters don't know to re-register unprompted. This is simple and sufficient for the current deployment model.

### Health On Demand

No heartbeat. Gateway checks `/health` when needed, handles failures gracefully.

### Channel Identity

The `channel` field is adapter-defined. Two adapters might use similar ID formats. Gateway always keys on `(adapter, channel)` pairs, never just `channel`.

---

## Gateway-Side Trait

The trait gateway uses internally. Implemented by `HttpAdapter` (real) and `MockAdapter` (testing).

```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    /// Adapter name
    fn name(&self) -> &str;

    /// Check if feature is supported
    fn supports(&self, feature: &Feature) -> bool;

    /// Send a message
    async fn send(&self, request: SendRequest) -> Result<SendResponse, AdapterError>;

    /// Read channel history (if supported)
    async fn read_history(
        &self,
        channel: &str,
        limit: usize,
    ) -> Result<Vec<IncomingEvent>, AdapterError>;

    /// Health check
    async fn health(&self) -> Result<bool, AdapterError>;
}
```

### Implementations

```rust
/// Calls external adapter process via HTTP
pub struct HttpAdapter {
    pub info: AdapterInfo,
    pub client: reqwest::Client,
}

/// In-process mock for testing
pub struct MockAdapter {
    pub name: String,
    pub features: HashSet<Feature>,
    pub messages: Arc<Mutex<Vec<SendRequest>>>,
}
```

Gateway's `AdapterRegistry` holds `Box<dyn Adapter>`. Real adapters are `HttpAdapter`, tests use `MockAdapter`.

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("connection failed: {0}")]
    Connection(String),

    #[error("request timeout")]
    Timeout,

    #[error("feature not supported: {0:?}")]
    Unsupported(Feature),

    #[error("rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("adapter error: {0}")]
    Platform(String),  // Adapter-specific errors

    #[error("invalid request: {0}")]
    InvalidRequest(String),
}
```

---

## HTTP API

Generated from Rust types via `utoipa`.

### Gateway Endpoints (adapter calls these)

| Method | Path | Request | Response |
|--------|------|---------|----------|
| POST | `/adapters/register` | `RegisterRequest` | `RegisterResponse` |
| POST | `/incoming` | `IncomingEvent` | `{ "ok": true }` |

### Adapter Endpoints (gateway calls these)

| Method | Path | Request | Response |
|--------|------|---------|----------|
| GET | `/capabilities` | — | `AdapterInfo` |
| GET | `/health` | — | `{ "healthy": true }` |
| POST | `/send` | `SendRequest` | `SendResponse` |
| GET | `/history/{channel}?limit=N` | — | `Vec<IncomingEvent>` |

### OpenAPI Generation

```rust
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(/* routes */),
    components(schemas(
        IncomingEvent,
        SendRequest,
        SendResponse,
        AdapterInfo,
        Feature,
        EventType,
        Author,
    ))
)]
pub struct AdapterApiDoc;

pub fn openapi_json() -> String {
    AdapterApiDoc::openapi().to_json().unwrap()
}
```

Spec committed to repo, regenerated on type changes.

### Security

**Auth is intentionally deferred.** Adapter ↔ gateway communication is localhost only. No authentication mechanism for now. Add token-based auth if/when adapters run on separate hosts.

---

## Discord Reference Implementation

Refactor `river-discord` to use `river-adapter` types.

### Changes

| Current | After |
|---------|-------|
| Hardcoded `"discord"` string | Uses `AdapterInfo.name` |
| Custom `IncomingEvent` struct | Uses `river_adapter::IncomingEvent` |
| Custom `SendRequest` struct | Uses `river_adapter::SendRequest` |
| Manual registration | Calls `POST /adapters/register` on startup |

### Feature Declaration

```rust
let discord_info = AdapterInfo {
    name: "discord".into(),
    version: env!("CARGO_PKG_VERSION").into(),
    url: format!("http://localhost:{}", config.port),
    features: HashSet::from([
        Feature::ReadHistory,
        Feature::Reactions,
        Feature::Threads,
        Feature::Attachments,
        Feature::Embeds,
        Feature::EditMessage,
        Feature::DeleteMessage,
        Feature::TypingIndicator,
    ]),
    metadata: json!({
        "bot_id": bot_user.id,
        "bot_name": bot_user.name,
    }),
};

gateway_client.register(discord_info).await?;
```

### Event Mapping

```rust
fn discord_message_to_event(msg: &Message) -> IncomingEvent {
    IncomingEvent {
        adapter: "discord".into(),
        event_type: EventType::MessageCreate,
        channel: msg.channel_id.to_string(),
        content: msg.content.clone(),
        metadata: json!({
            "guild_id": msg.guild_id,
            "attachments": msg.attachments,
            "embeds": msg.embeds,
        }),
        // ...
    }
}
```

Discord-specific data stays in `metadata`, native format preserved.

---

## Success Criteria

| Criterion | Test |
|-----------|------|
| Types compile | `river-adapter` builds, exports types |
| OpenAPI generates | `openapi.json` produced from types |
| Discord uses shared types | `river-discord` depends on `river-adapter`, compiles |
| Registration works | Adapter starts → gateway knows about it |
| Feature checks work | Request unsupported feature → clean error |
| MockAdapter works | Gateway tests pass with in-process mock |
| Health on demand | Gateway calls `/health`, gets response |

---

## Non-Goals (YAGNI)

- Dynamic adapter loading (plugins)
- Multiple adapters of same type
- Adapter-to-adapter communication
- Custom protocols (HTTP only for now)

---

## Related Documents

- `docs/specs/context-assembly-design.md` — I/You architecture
- `docs/specs/gateway-restructure-meta-plan.md` — Gateway rewrite plan
- `docs/roadmap.md` — Project roadmap

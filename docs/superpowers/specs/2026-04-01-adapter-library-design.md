# Adapter Library — Design Spec

> river-adapter: Types-only library for adapter ↔ worker communication
>
> Authors: Cass, Claude
> Date: 2026-04-01

## Overview

The adapter library (`river-adapter`) defines the interface between Workers and adapter binaries. It exports types, traits, and enums — no HTTP infrastructure. Adapter binaries implement the trait and handle their own servers.

**Key characteristics:**
- Types-only library (no HTTP server code)
- OpenAPI spec generated via `utoipa`
- Two required features: SendMessage, ReceiveMessage
- All other features optional
- Typed payloads for both inbound and outbound

## Crate Structure

```
river-adapter/
├── Cargo.toml
├── src/
│   ├── lib.rs           # re-exports, OpenAPI doc generation
│   ├── trait.rs         # Adapter trait
│   ├── feature.rs       # FeatureId enum, OutboundRequest enum
│   ├── event.rs         # InboundEvent, EventMetadata, EventType
│   ├── response.rs      # OutboundResponse, ResponseData, ResponseError
│   ├── author.rs        # Author, Attachment structs
│   └── error.rs         # AdapterError
└── openapi.json         # generated, committed
```

## Feature System

Two enums work together:

### FeatureId

Lightweight enum for registration and capability checks. Uses `repr(u16)` for wire efficiency.

```rust
#[repr(u16)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub enum FeatureId {
    // === Core messaging (0-9) ===
    SendMessage         = 0,    // required
    ReceiveMessage      = 1,    // required

    // === Message operations (10-19) ===
    EditMessage         = 10,
    DeleteMessage       = 11,
    ReadHistory         = 12,
    PinMessage          = 13,
    UnpinMessage        = 14,
    BulkDeleteMessages  = 15,

    // === Reactions (20-29) ===
    AddReaction         = 20,
    RemoveReaction      = 21,
    RemoveAllReactions  = 22,

    // === Attachments (30-39) ===
    Attachments         = 30,

    // === Typing (40-49) ===
    TypingIndicator     = 40,

    // === Threads (50-59) ===
    CreateThread        = 50,
    ThreadEvents        = 51,

    // === Polls (60-69) ===
    CreatePoll          = 60,
    PollVote            = 61,
    PollEvents          = 62,

    // === Situational awareness (100-109) ===
    VoiceStateEvents    = 100,
    PresenceEvents      = 101,
    MemberEvents        = 102,
    ScheduledEvents     = 103,

    // === Server admin (200-209) ===
    ChannelEvents       = 200,

    // === Connection (900-909) ===
    ConnectionEvents    = 900,
}

impl FeatureId {
    pub fn is_required(&self) -> bool {
        matches!(self, Self::SendMessage | Self::ReceiveMessage)
    }
}
```

**Note on required features:**
- `SendMessage` — adapter must support outbound messages via `execute(OutboundRequest::SendMessage { .. })`
- `ReceiveMessage` — adapter must forward inbound events to the Worker via `start()`. There is no `OutboundRequest::ReceiveMessage` variant since receiving is handled by the `start()` method, not `execute()`.

**Action features vs event features:**

Some features represent actions (have `OutboundRequest` variants):
- `SendMessage`, `EditMessage`, `DeleteMessage`, `ReadHistory`, `AddReaction`, `TypingIndicator`, `CreateThread`, `CreatePoll`, etc.

Some features represent event subscriptions (no `OutboundRequest` variants):
- `ReceiveMessage`, `ThreadEvents`, `PollEvents`, `VoiceStateEvents`, `PresenceEvents`, `MemberEvents`, `ScheduledEvents`, `ChannelEvents`, `ConnectionEvents`

Event features indicate that the adapter will forward those event types to the Worker via `InboundEvent`. The Worker uses the adapter's reported features to know what events to expect.

### OutboundRequest

Data-carrying enum with typed payloads. Links to `FeatureId` via `feature_id()` method.

```rust
#[derive(Serialize, Deserialize, ToSchema)]
pub enum OutboundRequest {
    SendMessage {
        channel: String,
        content: String,
        reply_to: Option<String>,
    },
    EditMessage {
        channel: String,
        message_id: String,
        content: String,
    },
    DeleteMessage {
        channel: String,
        message_id: String,
    },
    ReadHistory {
        channel: String,
        limit: Option<u32>,
        before: Option<String>,
    },
    PinMessage {
        channel: String,
        message_id: String,
    },
    UnpinMessage {
        channel: String,
        message_id: String,
    },
    BulkDeleteMessages {
        channel: String,
        message_ids: Vec<String>,
    },
    AddReaction {
        channel: String,
        message_id: String,
        emoji: String,
    },
    RemoveReaction {
        channel: String,
        message_id: String,
        emoji: String,
    },
    RemoveAllReactions {
        channel: String,
        message_id: String,
    },
    SendAttachment {
        channel: String,
        filename: String,
        data: Vec<u8>,
        content_type: Option<String>,
    },
    TypingIndicator {
        channel: String,
    },
    CreateThread {
        channel: String,
        message_id: String,
        name: String,
    },
    CreatePoll {
        channel: String,
        question: String,
        options: Vec<String>,
        duration_hours: Option<u32>,
    },
    PollVote {
        channel: String,
        poll_id: String,
        option_index: u32,
    },
}

impl OutboundRequest {
    pub fn feature_id(&self) -> FeatureId {
        match self {
            Self::SendMessage { .. } => FeatureId::SendMessage,
            Self::EditMessage { .. } => FeatureId::EditMessage,
            Self::DeleteMessage { .. } => FeatureId::DeleteMessage,
            Self::ReadHistory { .. } => FeatureId::ReadHistory,
            Self::PinMessage { .. } => FeatureId::PinMessage,
            Self::UnpinMessage { .. } => FeatureId::UnpinMessage,
            Self::BulkDeleteMessages { .. } => FeatureId::BulkDeleteMessages,
            Self::AddReaction { .. } => FeatureId::AddReaction,
            Self::RemoveReaction { .. } => FeatureId::RemoveReaction,
            Self::RemoveAllReactions { .. } => FeatureId::RemoveAllReactions,
            Self::SendAttachment { .. } => FeatureId::Attachments,
            Self::TypingIndicator { .. } => FeatureId::TypingIndicator,
            Self::CreateThread { .. } => FeatureId::CreateThread,
            Self::CreatePoll { .. } => FeatureId::CreatePoll,
            Self::PollVote { .. } => FeatureId::PollVote,
        }
    }
}
```

## Inbound Events

### InboundEvent

Minimal top-level struct. All event-specific data lives in `EventMetadata`.

```rust
#[derive(Serialize, Deserialize, ToSchema)]
pub struct InboundEvent {
    pub adapter: String,
    pub metadata: EventMetadata,
}
```

### EventType

Lightweight enum for event type identification. Uses serde `rename_all` for snake_case serialization.

```rust
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    MessageCreate,
    MessageUpdate,
    MessageDelete,
    ReactionAdd,
    ReactionRemove,
    TypingStart,
    MemberJoin,
    MemberLeave,
    PresenceUpdate,
    VoiceStateUpdate,
    ChannelCreate,
    ChannelUpdate,
    ChannelDelete,
    ThreadCreate,
    ThreadUpdate,
    ThreadDelete,
    PinUpdate,
    PollVote,
    ScheduledEvent,
    ConnectionLost,
    ConnectionRestored,
    Unknown,
}
```

### EventMetadata

Data-carrying enum with per-event-type fields. Links to `EventType` via `event_type()` method.

```rust
#[derive(Serialize, Deserialize, ToSchema)]
pub enum EventMetadata {
    MessageCreate {
        channel: String,
        author: Author,
        content: String,
        message_id: String,
        timestamp: String,
        reply_to: Option<String>,
        attachments: Vec<Attachment>,
    },
    MessageUpdate {
        channel: String,
        message_id: String,
        content: String,
        timestamp: String,
    },
    MessageDelete {
        channel: String,
        message_id: String,
    },
    ReactionAdd {
        channel: String,
        message_id: String,
        user_id: String,
        emoji: String,
    },
    ReactionRemove {
        channel: String,
        message_id: String,
        user_id: String,
        emoji: String,
    },
    TypingStart {
        channel: String,
        user_id: String,
    },
    MemberJoin {
        user_id: String,
        username: String,
    },
    MemberLeave {
        user_id: String,
    },
    PresenceUpdate {
        user_id: String,
        status: String,
    },
    VoiceStateUpdate {
        user_id: String,
        channel: Option<String>,
    },
    ChannelCreate {
        channel: String,
        name: String,
    },
    ChannelUpdate {
        channel: String,
        name: String,
    },
    ChannelDelete {
        channel: String,
    },
    ThreadCreate {
        channel: String,
        parent_channel: String,
        name: String,
    },
    ThreadUpdate {
        channel: String,
        name: String,
    },
    ThreadDelete {
        channel: String,
    },
    PinUpdate {
        channel: String,
        message_id: String,
        pinned: bool,
    },
    PollVote {
        channel: String,
        poll_id: String,
        user_id: String,
        option_index: u32,
        added: bool,
    },
    ScheduledEvent {
        event_id: String,
        name: String,
        start_time: String,
    },
    ConnectionLost {
        reason: String,
        reconnecting: bool,
    },
    ConnectionRestored {
        downtime_seconds: u64,
    },
    Unknown(serde_json::Value),
}

impl EventMetadata {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::MessageCreate { .. } => EventType::MessageCreate,
            Self::MessageUpdate { .. } => EventType::MessageUpdate,
            Self::MessageDelete { .. } => EventType::MessageDelete,
            Self::ReactionAdd { .. } => EventType::ReactionAdd,
            Self::ReactionRemove { .. } => EventType::ReactionRemove,
            Self::TypingStart { .. } => EventType::TypingStart,
            Self::MemberJoin { .. } => EventType::MemberJoin,
            Self::MemberLeave { .. } => EventType::MemberLeave,
            Self::PresenceUpdate { .. } => EventType::PresenceUpdate,
            Self::VoiceStateUpdate { .. } => EventType::VoiceStateUpdate,
            Self::ChannelCreate { .. } => EventType::ChannelCreate,
            Self::ChannelUpdate { .. } => EventType::ChannelUpdate,
            Self::ChannelDelete { .. } => EventType::ChannelDelete,
            Self::ThreadCreate { .. } => EventType::ThreadCreate,
            Self::ThreadUpdate { .. } => EventType::ThreadUpdate,
            Self::ThreadDelete { .. } => EventType::ThreadDelete,
            Self::PinUpdate { .. } => EventType::PinUpdate,
            Self::PollVote { .. } => EventType::PollVote,
            Self::ScheduledEvent { .. } => EventType::ScheduledEvent,
            Self::ConnectionLost { .. } => EventType::ConnectionLost,
            Self::ConnectionRestored { .. } => EventType::ConnectionRestored,
            Self::Unknown(_) => EventType::Unknown,
        }
    }
}
```

## Supporting Types

```rust
#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub bot: bool,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct Channel {
    pub adapter: String,
    pub id: String,
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub url: String,
    pub size: u64,
    pub content_type: Option<String>,
}

// --- Shared types for orchestrator/worker ---

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Baton {
    Actor,
    Spectator,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct Ground {
    pub name: String,      // human operator name
    pub id: String,        // human operator platform ID
    pub adapter: String,   // adapter type (discord, slack, etc.)
    pub channel: String,   // channel ID for reaching human
}
```

## Adapter Trait

```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    /// Adapter type name (e.g. "discord", "slack")
    fn adapter_type(&self) -> &str;

    /// Which features this adapter supports
    fn features(&self) -> Vec<FeatureId>;

    /// Check if a specific feature is supported
    fn supports(&self, feature: FeatureId) -> bool {
        self.features().contains(&feature)
    }

    /// Start receiving events, forward to bound Worker
    async fn start(&self, worker_endpoint: String) -> Result<(), AdapterError>;

    /// Execute an outbound request
    async fn execute(&self, request: OutboundRequest) -> Result<OutboundResponse, AdapterError>;

    /// Health check
    async fn health(&self) -> Result<(), AdapterError>;
}
```

## HTTP API

Adapter binaries must expose these HTTP endpoints. The trait methods map directly to endpoints.

| Method | Endpoint | Request Body | Response |
|--------|----------|--------------|----------|
| POST | `/start` | `{ "worker_endpoint": "http://..." }` | `{ "ok": true }` |
| POST | `/execute` | `OutboundRequest` | `OutboundResponse` |
| GET | `/health` | — | `{ "status": "ok" }` |

### POST /start

Bind adapter to a worker. Adapter begins forwarding inbound events to `{worker_endpoint}/notify`.

```json
// Request
{ "worker_endpoint": "http://localhost:52341" }

// Response 200
{ "ok": true }

// Response 400 (already started)
{ "ok": false, "error": "already bound to worker" }
```

### POST /execute

Execute an outbound request. Body is an `OutboundRequest` variant.

```json
// Request
{
  "SendMessage": {
    "channel": "123456",
    "content": "Hello!",
    "reply_to": null
  }
}

// Response 200
{
  "ok": true,
  "data": { "MessageSent": { "message_id": "789" } }
}

// Response 400 (unsupported)
{
  "ok": false,
  "error": { "code": "unsupported_feature", "message": "EditMessage not supported" }
}
```

### GET /health

Health check for orchestrator supervision.

```json
// Response 200
{ "status": "ok" }

// Response 503 (disconnected from platform)
{ "status": "error", "message": "websocket disconnected" }
```

## Response Types

```rust
#[derive(Serialize, Deserialize, ToSchema)]
pub struct OutboundResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ResponseData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub enum ResponseData {
    MessageSent { message_id: String },
    MessageEdited { message_id: String },
    MessageDeleted,
    MessagesPinned,
    MessagesUnpinned,
    MessagesDeleted { count: usize },
    ReactionAdded,
    ReactionRemoved,
    ReactionsCleared,
    AttachmentSent { message_id: String },
    TypingStarted,
    History { messages: Vec<HistoryMessage> },
    ThreadCreated { thread_id: String },
    PollCreated { poll_id: String },
    PollVoted,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct HistoryMessage {
    pub message_id: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    pub timestamp: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ResponseError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnsupportedFeature,
    InvalidPayload,
    PlatformError,
    RateLimited,
    NotFound,
    Unauthorized,
}
```

## Error Types

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

    #[error("platform error: {0}")]
    Platform(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),
}
```

## OpenAPI Generation

```rust
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    components(schemas(
        // Feature system
        FeatureId,
        OutboundRequest,

        // Inbound events
        InboundEvent,
        EventMetadata,
        EventType,

        // Responses
        OutboundResponse,
        ResponseData,
        ResponseError,
        ErrorCode,
        HistoryMessage,

        // Supporting
        Author,
        Channel,
        Attachment,
        Baton,
        Side,
        Ground,
    ))
)]
pub struct AdapterApiDoc;

pub fn openapi_json() -> String {
    AdapterApiDoc::openapi().to_pretty_json().unwrap()
}
```

The `openapi.json` file is generated and committed to the repo. Regenerated when types change.

## Dependencies

```toml
[package]
name = "river-adapter"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
utoipa = { workspace = true }
async-trait = { workspace = true }
```

## Usage Examples

### Adapter Registration

```rust
use river_adapter::{FeatureId, Adapter};

impl MyDiscordAdapter {
    fn features(&self) -> Vec<FeatureId> {
        vec![
            FeatureId::SendMessage,
            FeatureId::ReceiveMessage,
            FeatureId::EditMessage,
            FeatureId::DeleteMessage,
            FeatureId::ReadHistory,
            FeatureId::AddReaction,
            FeatureId::TypingIndicator,
        ]
    }
}
```

### Handling Outbound Requests

```rust
use river_adapter::{OutboundRequest, OutboundResponse, ResponseData};

async fn execute(&self, request: OutboundRequest) -> Result<OutboundResponse, AdapterError> {
    match request {
        OutboundRequest::SendMessage { channel, content, reply_to } => {
            let msg_id = self.discord.send(&channel, &content, reply_to).await?;
            Ok(OutboundResponse {
                ok: true,
                data: Some(ResponseData::MessageSent { message_id: msg_id }),
                error: None,
            })
        }
        OutboundRequest::AddReaction { channel, message_id, emoji } => {
            self.discord.add_reaction(&channel, &message_id, &emoji).await?;
            Ok(OutboundResponse {
                ok: true,
                data: Some(ResponseData::ReactionAdded),
                error: None,
            })
        }
        _ => Err(AdapterError::Unsupported(request.feature_id())),
    }
}
```

### Sending Inbound Events

```rust
use river_adapter::{InboundEvent, EventMetadata, Author};

let event = InboundEvent {
    adapter: "discord".into(),
    metadata: EventMetadata::MessageCreate {
        channel: "general".into(),
        author: Author {
            id: "12345".into(),
            name: "alice".into(),
            bot: false,
        },
        content: "hey, can you check the logs?".into(),
        message_id: "msg-789".into(),
        timestamp: "2026-04-01T14:30:00Z".into(),
        reply_to: None,
        attachments: vec![],
    },
};

// POST to worker's /notify endpoint
client.post(&format!("{}/notify", worker_endpoint))
    .json(&event)
    .send()
    .await?;
```

## Related Documents

- `docs/ADAPTER-DESIGN.md` — High-level adapter design
- `docs/WORKER-DESIGN.md` — Worker architecture (receives inbound events)
- `docs/ORCHESTRATOR-DESIGN.md` — Orchestrator (spawns adapters)

# River Adapter — Design (WIP)

## Philosophy

An adapter is a dumb pipe. It connects to an external service, receives all events, and forwards them to its bound Worker. Outbound, it takes payloads from the Worker and sends them to the external service. It does not filter, prioritize, or understand messages.

## What An Adapter Is

A library crate (`river-adapter`) that exports a trait. Specific adapters (Discord, Slack, etc.) are separate binary crates that implement the trait. The trait requires two capabilities: sending and receiving. Everything else is a feature flag.

```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    /// Adapter type name (e.g. "discord", "slack")
    fn adapter_type(&self) -> &str;

    /// Start receiving events, forward to bound Worker
    async fn start(&self, worker_endpoint: String) -> Result<()>;

    /// Execute a feature action. The Worker specifies which feature
    /// it wants and provides a typed payload matching that feature's schema.
    async fn execute(&self, feature: AdapterFeature, payload: Value) -> Result<Value>;

    /// What this adapter supports
    fn features(&self) -> Vec<AdapterFeature>;
}
```

### Feature System

Each feature maps to an integer and has a schema method that returns the expected payload shape. The Worker sends `{feature: <int>, payload: {...}}` and the adapter validates the payload against the schema before executing.

```rust
#[repr(u16)]
pub enum AdapterFeature {
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

    // === Attachments & embeds (30-39) ===
    Attachments         = 30,

    // === Typing (40-49) ===
    TypingIndicator     = 40,

    // === Threads (50-59) ===
    CreateThread        = 50,
    ThreadEvents        = 51,   // receive thread create/update/delete

    // === Polls (60-69) ===
    CreatePoll          = 60,
    PollVote            = 61,
    PollEvents          = 62,   // receive vote add/remove

    // === Situational awareness (100-109) ===
    VoiceStateEvents    = 100,  // who's in voice channels
    PresenceEvents      = 101,  // online/offline/idle/dnd
    MemberEvents        = 102,  // join, leave
    ScheduledEvents     = 103,  // upcoming server events

    // === Server admin (200-209) ===
    ChannelEvents       = 200,  // channel create/update/delete

    // === Connection (900-909) ===
    ConnectionEvents    = 900,  // connection_lost, connection_restored
}

impl AdapterFeature {
    /// Returns the JSON schema for the payload this feature expects
    pub fn schema(&self) -> Value {
        match self {
            Self::SendMessage => json!({
                "channel": "string (required)",
                "content": "string (required)",
                "reply_to": "string (optional)"
            }),
            Self::EditMessage => json!({
                "channel": "string (required)",
                "message_id": "string (required)",
                "content": "string (required)"
            }),
            Self::AddReaction => json!({
                "channel": "string (required)",
                "message_id": "string (required)",
                "emoji": "string (required)"
            }),
            Self::ReadHistory => json!({
                "channel": "string (required)",
                "limit": "number (optional, default 50)",
                "before": "string (optional, message_id)"
            }),
            // ... etc
            _ => json!({})
        }
    }

    pub fn as_u16(&self) -> u16 {
        *self as u16
    }

    pub fn from_u16(n: u16) -> Option<Self> {
        // ...
    }
}
```

### Outbound Request Format

The Worker sends a typed request to the adapter:

```json
{
  "feature": 0,
  "payload": {
    "channel": "general",
    "content": "The postgres service was down, I've restarted it.",
    "reply_to": "msg-789"
  }
}
```

The adapter:
1. Checks if it supports feature 0 (`SendMessage`)
2. Validates the payload against `SendMessage.schema()`
3. Translates to the platform-specific API call
4. Returns the result or error

## Relationship To Workers

- Each adapter instance is bound to exactly one Worker
- Two Workers on Discord = two adapter processes with **different bot tokens**
- Adapters are defined in CLI config, spawned by the orchestrator at startup
- The adapter knows its bound Worker's endpoint via the process registry
- The Worker talks directly to the adapter (no orchestrator in the middle for message routing)

## Adapter Binary Discovery

The orchestrator maps adapter types to binaries via CLI options:

```bash
--adapter-binary discord=river-discord
--adapter-binary slack=river-slack
```

The orchestrator looks up the binary name and spawns it as a child process, passing the orchestrator endpoint and adapter config as arguments. The binary must be on `$PATH` or specified as an absolute path.

This means:
- Adapters are separate binaries, not plugins loaded into the orchestrator
- New adapters are installed by putting a binary on `$PATH` and adding it to config
- The orchestrator doesn't need to know anything about the adapter's internals

## Lifecycle

```
Orchestrator                          Adapter
    │                                   │
    ├──spawn binary───────────────────▶│
    │   (river-discord --orchestrator   │
    │    http://localhost:PORT           │
    │    --config '{"token_file":...}') │
    │                                   │
    │                          (binds port 0, connects to external service)
    │                                   │
    │◀──POST /register────────────────│
    │   {endpoint:"...",                │
    │    adapter:{type:"discord",       │
    │             worker_name:"river"}} │
    │                                   │
    ├──POST /registry────────────────▶│
    │   (here's who's alive, including  │
    │    your bound Worker's endpoint)  │
    │                                   │
    │     ... adapter runs ...          │
    │                                   │
```

## Inbound Flow

```
External Service (e.g. Discord)
         │
         ▼
┌─────────────────────┐
│ Adapter receives    │  (websocket event, webhook, poll, etc.)
│ event from service  │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ Build inbound       │  Normalize to common format
│ message             │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ POST to Worker      │  Worker's notification endpoint
│ /notify             │
└─────────────────────┘
```

**Inbound message format** (what the adapter sends to the Worker):

```json
{
  "adapter": "discord",
  "event_type": "message_create",
  "payload": {
    "channel": "general",
    "author": {"id": "12345", "name": "alice", "bot": false},
    "content": "hey, can you check the logs?",
    "id": "msg-789",
    "timestamp": "2026-03-28T14:30:00Z",
    "attachments": [],
    "reply_to": null
  }
}
```

The adapter normalizes platform-specific events into this common format. Everything goes in `payload` — channel, author, content, metadata. The only top-level fields are `adapter` (which adapter sent this) and `event_type` (what kind of event).

### Standard Event Types

The `river-adapter` crate defines standard event type strings. Adapters map their platform events to these:

| Event Type | Description |
|------------|-------------|
| `message_create` | New message |
| `message_update` | Message edited |
| `message_delete` | Message deleted |
| `reaction_add` | Reaction added to a message |
| `reaction_remove` | Reaction removed from a message |
| `typing_start` | User started typing |
| `member_join` | User joined the server |
| `member_leave` | User left the server |
| `presence_update` | User online/offline/idle/dnd status changed |
| `voice_state_update` | User joined/left/moved voice channel |
| `channel_create` | New channel created |
| `channel_update` | Channel settings changed |
| `channel_delete` | Channel deleted |
| `thread_create` | New thread created |
| `thread_update` | Thread settings changed |
| `thread_delete` | Thread deleted |
| `pin_update` | Message pinned or unpinned |
| `poll_vote` | Vote added or removed on a poll |
| `scheduled_event` | Scheduled event created/updated/deleted |
| `connection_lost` | Adapter lost connection to external service |
| `connection_restored` | Adapter reconnected |

Adapters can also send platform-specific event types not in this list. The Worker handles unknown types gracefully (logs and ignores).

### Connection Events

The adapter also forwards connection state changes to the Worker as events:

```json
{
  "adapter": "discord",
  "event_type": "connection_lost",
  "payload": {
    "reason": "websocket closed",
    "reconnecting": true
  }
}
```

```json
{
  "adapter": "discord",
  "event_type": "connection_restored",
  "payload": {
    "downtime_seconds": 12
  }
}
```

This lets the model know when an adapter is degraded and adjust its behavior (e.g. stop trying to send messages until it reconnects, notify users on another channel).

## Outbound Flow

The Worker sends a typed feature request to the adapter. The adapter validates and executes.

```
Worker calls speak/send_message
         │
         ▼
┌─────────────────────┐
│ POST to Adapter     │  Adapter's /send endpoint
│ /send               │  {feature: <int>, payload: {...}}
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ Adapter validates   │  Check feature support, validate payload schema
│ and executes        │  Translate to platform-specific API call(s)
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ Return result       │  Response payload or error
└─────────────────────┘
```

**Examples:**

Send a message (feature 0):
```json
{"feature": 0, "payload": {"channel": "general", "content": "hello!", "reply_to": "msg-789"}}
```

Add a reaction (feature 20):
```json
{"feature": 20, "payload": {"channel": "general", "message_id": "msg-789", "emoji": "👍"}}
```

Fetch history (feature 12):
```json
{"feature": 12, "payload": {"channel": "general", "limit": 50}}
```

Typing indicator (feature 40):
```json
{"feature": 40, "payload": {"channel": "general"}}
```

Pin a message (feature 13):
```json
{"feature": 13, "payload": {"channel": "general", "message_id": "msg-789"}}
```

**Standard response format**:

Success:
```json
{"ok": true, "data": {"message_id": "msg-790"}}
```

Error (unsupported feature):
```json
{"ok": false, "error": {"code": "unsupported_feature", "feature": 20, "message": "AddReaction not supported"}}
```

Error (platform error):
```json
{"ok": false, "error": {"code": "platform_error", "message": "rate limited, retry after 5s"}}
```

Success with data (history):
```json
{"ok": true, "data": {"messages": [...]}}
```

All responses have `ok: bool`. On success, `data` contains the result. On failure, `error` contains `code` and `message`.

## Worker Tool Changes

The Worker's `speak` and `send_message` tools pass typed feature requests through to the adapter.

**speak** — sends to the current adapter + channel:
```json
{
  "feature": 0,
  "payload": {}
}
```
The Worker runtime injects the current channel into the payload before forwarding. The feature int tells the adapter exactly what to do.

**send_message** — sends to any adapter:
```json
{
  "adapter": "discord",
  "feature": 0,
  "payload": {}
}
```
The payload must include `channel` and whatever else the feature schema requires.

## Adapter HTTP API

Every adapter exposes the same minimal interface:

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/send` | Handle any outbound payload |
| GET | `/health` | Health status |
| POST | `/registry` | Receive updated process registry |

One endpoint for everything outbound. The payload determines what happens.

## Reconnection

The adapter handles its own reconnection. If a Discord adapter loses its websocket, it reconnects. It does not ask the orchestrator for help. The orchestrator only restarts the adapter if the adapter process itself crashes.

Connection state changes (lost, restored) are forwarded to the Worker as events so the model can adjust its behavior.

## Rate Limiting

Two layers:

- **Internal**: the adapter handles rate limits the way platform libraries already do (e.g. discord.rs queues and retries automatically). This is invisible to the Worker.
- **Passthrough**: if a rate limit can't be resolved internally (e.g. hard daily limit, extended throttle), the error is returned to the Worker as a tool result. The model decides what to do.

## What An Adapter Does NOT Do

- Filter events by channel (the Worker decides what matters)
- Understand message content or commands semantically
- Route between Workers (each adapter is bound to one Worker)
- Store messages (the Worker writes to conversation files)

## Implementing A New Adapter

1. Create a new binary crate (e.g. `river-slack`)
2. Depend on `river-adapter` for the trait, `AdapterFeature` enum, and common types
3. Implement the `Adapter` trait:
   - `adapter_type()` → `"slack"`
   - `start()` → connect to Slack, forward events as inbound messages
   - `execute()` → match on the feature, validate payload against `feature.schema()`, call platform API
   - `features()` → list which `AdapterFeature` variants you support
4. Handle: port 0 binding, HTTP server, orchestrator registration, reconnection, rate limiting
5. Add the binary name to the orchestrator's `adapter_binaries` config

## Resolved Decisions

1. **Architecture** — adapter is a library crate exporting a trait. Specific adapters are separate binaries implementing the trait.
2. **Required capabilities** — `SendMessage` (feature 0) and `ReceiveMessage` (feature 1). Everything else is opt-in.
3. **Feature system** — `AdapterFeature` enum with `repr(u16)` int mapping. Each variant has a `schema()` method returning the expected payload shape. `from_u16()` enables deserialization from wire format. The Worker sends `{feature: <int>, payload: {...}}`. The adapter validates and executes.
4. **Feature negotiation** — adapters report supported features (as int list) during registration. The orchestrator includes feature names and ints in the Worker's system prompt. The model knows what each adapter can do from the start.
5. **Standard event types** — the `river-adapter` crate defines standard event type strings (`message_create`, `message_update`, etc.). Adapters map platform events to these. Unknown types are logged and ignored by the Worker.
6. **Standard response format** — all responses have `{ok: bool, data?: {...}, error?: {code, message}}`.
7. **Rate limiting** — adapters handle internally. Unresolvable limits passed back as standard errors.
8. **Reconnection** — the adapter handles it. Connection state changes forwarded to Worker as events.
9. **Binary discovery** — orchestrator CLI maps adapter types to binary names on `$PATH`.
10. **Payload-only interface** — channel, author, content all live in the payload. Top-level inbound is `{adapter, event_type, payload}`. Top-level outbound is `{feature, payload}`.
11. **Multiple Workers, same platform** — use different bot tokens per adapter process. One token per gateway connection.

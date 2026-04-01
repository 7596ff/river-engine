# Discord Adapter Design Specification

**Date:** 2026-03-16
**Status:** Draft
**Crate:** `river-discord`

## Overview

A standalone Discord adapter for River Engine using the Twilight library. Routes messages between Discord and a river-gateway instance. One adapter instance per agent.

## Architecture

### Crate Structure

```
crates/river-discord/
├── src/
│   ├── main.rs          # CLI entry point
│   ├── lib.rs           # Public exports
│   ├── client.rs        # Twilight Discord client wrapper
│   ├── gateway.rs       # HTTP client for river-gateway
│   ├── handler.rs       # Discord event handling
│   ├── commands.rs      # Slash commands (/listen, /unlisten, /channels)
│   ├── outbound.rs      # HTTP server (send_message callbacks + admin API)
│   ├── channels.rs      # Channel state management (thread-safe)
│   └── config.rs        # Configuration types
├── Cargo.toml
```

### Dependencies

- `twilight-gateway` - Discord websocket connection
- `twilight-model` - Discord type definitions
- `twilight-http` - Discord REST API client
- `axum` - HTTP server for outbound callbacks and admin API
- `reqwest` - HTTP client for gateway communication
- `tokio` - Async runtime
- `clap` - CLI argument parsing
- `tracing` - Structured logging
- `serde` / `serde_json` - Serialization

## CLI Interface

```bash
river-discord \
  --token-file <path>        # Discord bot token file (required)
  --gateway-url <url>        # River gateway URL (required)
  --listen-port <port>       # Adapter HTTP server port (default: 3002)
  --channels <id,id,...>     # Initial channel IDs (optional)
  --state-file <path>        # Persist channel state across restarts (optional)
  --guild-id <id>            # Guild for slash command registration (required)
```

## Message Flow

### Inbound: Discord → Agent

1. Twilight websocket receives Discord message event
2. Check if channel is in listen set
   - No: ignore message
   - Yes: continue
3. Format as gateway IncomingEvent:
   ```json
   {
     "adapter": "discord",
     "event_type": "message",
     "channel": "<channel_id>",
     "author": {
       "id": "<user_id>",
       "name": "<username>"
     },
     "content": "<message_text>",
     "message_id": "<discord_message_id>",
     "metadata": {
       "guild_id": "<guild_id>",
       "thread_id": "<thread_id>",
       "reply_to": "<referenced_message_id>"
     }
   }
   ```
4. POST to gateway `/incoming` endpoint
5. Gateway queues message for agent processing

### Outbound: Agent → Discord

1. Agent calls `send_message` tool during execution
2. Gateway POSTs to adapter `http://localhost:<port>/send`:
   ```json
   {
     "channel": "<channel_id>",
     "content": "<message_text>",
     "reply_to": "<message_id>",
     "thread_id": "<thread_id>",
     "create_thread": "<thread_title>",
     "reaction": "<emoji>"
   }
   ```
3. Adapter calls Discord API via Twilight HTTP client
4. Returns result to gateway:
   ```json
   {
     "success": true,
     "message_id": "<new_message_id>"
   }
   ```
   Or on error:
   ```json
   {
     "success": false,
     "error": "<error_description>"
   }
   ```

### Reaction Events

Inbound reaction events formatted as:
```json
{
  "adapter": "discord",
  "event_type": "reaction_add",
  "channel": "<channel_id>",
  "message_id": "<message_id>",
  "author": {
    "id": "<user_id>",
    "name": "<username>"
  },
  "content": "<emoji>",
  "metadata": {}
}
```

Agent adds reactions by including `"reaction": "<emoji>"` in outbound message (mutually exclusive with `content`).

## HTTP API Specification

All endpoints are unauthenticated (adapter runs on localhost only).

### POST /send (Outbound Messages)

Gateway calls this to send messages to Discord.

**Request:**
```json
{
  "channel": "<channel_id>",
  "content": "<message_text>",
  "reply_to": "<message_id>",
  "thread_id": "<thread_id>",
  "create_thread": "<thread_title>",
  "reaction": "<emoji>"
}
```

**Field validation:**
- `channel` is required
- `content` and `reaction` are mutually exclusive (error if both provided)
- `reply_to` and `create_thread` are mutually exclusive (error if both provided)
- At least one of `content` or `reaction` must be provided

**Response (200 OK):**
```json
{ "success": true, "message_id": "<new_message_id>" }
```

**Response (400 Bad Request):**
```json
{ "success": false, "error": "validation error: content and reaction are mutually exclusive" }
```

**Response (502 Bad Gateway):**
```json
{ "success": false, "error": "discord api error: channel not accessible" }
```

Timeout: 30 seconds. No retry on failure - error returns to agent.

### GET /channels

**Response (200 OK):**
```json
{ "channels": ["123", "456", "789"] }
```

### POST /channels

**Request:**
```json
{ "channel_id": "123456789" }
```

**Response (200 OK):**
```json
{ "success": true }
```

**Response (400 Bad Request):**
```json
{ "success": false, "error": "invalid channel id" }
```

### DELETE /channels/{id}

**Response (200 OK):**
```json
{ "success": true }
```

**Response (404 Not Found):**
```json
{ "success": false, "error": "channel not in listen set" }
```

### GET /health

**Response (200 OK):**
```json
{
  "status": "ok",
  "discord": "connected",
  "gateway": "reachable",
  "channel_count": 3
}
```

## Channel Management

### Runtime State

- Stored in `RwLock<HashSet<ChannelId>>`
- Thread-safe for concurrent access

### State Persistence

When `--state-file` is provided:

**File format:**
```json
{
  "version": 1,
  "channels": [123456789, 987654321]
}
```

**Write timing:**
- Written after every add/remove operation
- Atomic write (write to temp file, then rename)

**Startup behavior:**
- If file exists and is valid: load channels
- If file exists but corrupted: log warning, start with empty set
- If file doesn't exist: start with `--channels` from CLI

### Slash Commands

Admin-only commands (require Manage Channels permission):

- `/listen #channel` - Add channel to listen set
- `/unlisten #channel` - Remove channel from listen set
- `/channels` - List currently monitored channels

No public slash commands. Agent interaction is through messages in listened channels only.

## Thread Support

- Messages in threads include `thread_id` in metadata
- Agent replies to thread messages go to the same thread automatically
- Agent can create new threads by including `"create_thread": "Thread Title"` in outbound message
- Thread becomes a listened "channel" automatically when created by agent

## Error Handling

### Gateway Unreachable
- Log warning (no details)
- Retry with exponential backoff
- Do not crash
- If persistent, errors flow back through Discord callback responses

### Discord Disconnect
- Twilight handles automatic reconnection
- Log connection status changes

### Invalid Operations
- `/listen` on inaccessible channel: Return error message to Discord user
- send_message to unknown channel: Return `{ "success": false, "error": "channel not accessible" }` to gateway
- Agent receives error and can communicate issue to user

### Rate Limiting
- Twilight handles Discord rate limits automatically
- No additional handling needed

## Logging

### Privacy Requirements

Logs must not contain any identifying information:

**Allowed:**
- Connection status: `"Connected to Discord"`, `"Gateway reachable"`
- Event counts: `"Forwarded message to gateway"`
- Error types: `"Gateway request failed: connection refused"`
- Operational status: `"Discord reconnecting"`, `"Channel added"`

**Forbidden:**
- Message content
- Message IDs
- Channel IDs
- User IDs
- Usernames
- Guild IDs
- Any data that could identify conversations or users

### Error Visibility

Errors flow to the agent through gateway responses, not logs. Agent can then ask the user for help (e.g., "I couldn't send that message, can you check my permissions in that channel?").

## Configuration Types

```rust
pub struct DiscordConfig {
    pub token_file: PathBuf,
    pub gateway_url: String,
    pub listen_port: u16,
    pub initial_channels: Vec<u64>,
    pub state_file: Option<PathBuf>,
    pub guild_id: u64,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            token_file: PathBuf::from("/run/secrets/discord-token"),
            gateway_url: "http://localhost:3000".to_string(),
            listen_port: 3002,
            initial_channels: Vec::new(),
            state_file: None,
            guild_id: 0,
        }
    }
}
```

## Testing Strategy

### Unit Tests
- Channel state management (add, remove, persist, load)
- Message formatting (Discord event → gateway JSON)
- Outbound parsing (gateway request → Discord API parameters)
- Config parsing and validation

### Integration Tests
- Mock Twilight gateway for event handling
- Mock river-gateway for verifying inbound POST format
- HTTP server tests for admin API endpoints
- HTTP server tests for send_message endpoint

### Manual Testing
- Live Discord integration tested manually (not in CI)
- Slash command registration and permission checks
- Thread creation and reply flow
- Reaction send/receive

## Twilight Integration

### Required Discord Intents

```rust
Intents::GUILDS
    | Intents::GUILD_MESSAGES
    | Intents::GUILD_MESSAGE_REACTIONS
    | Intents::MESSAGE_CONTENT  // Privileged intent - must enable in Discord Developer Portal
    | Intents::DIRECT_MESSAGES
```

### Token Format

- Token file contains raw bot token (no "Bot " prefix)
- Single line, no trailing newline
- File permissions should be 0600

### Connection Lifecycle

- Single shard (sufficient for one guild per adapter)
- Twilight handles reconnection automatically
- On startup: connect, register slash commands, load channel state
- On shutdown: graceful disconnect (no special handling needed)

### Slash Command Registration

- Commands registered on startup for the configured guild
- Uses `twilight-http` to register application commands
- Re-registers on every startup (idempotent)

## Future Considerations

Not in scope for initial implementation:

- Embed support for rich messages (can add later)
- File/attachment handling
- Voice channel support
- Multiple guild support per adapter
- Message edit/delete events

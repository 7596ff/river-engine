# TUI Adapter

## Goal

A terminal-based chat adapter for the river engine. Runs as a separate process like the Discord adapter, speaks the same HTTP protocol, but the "platform" is the local terminal.

## Architecture

Same adapter pattern as Discord: separate process, HTTP on both sides. The orchestrator spawns it like any other adapter.

```
User types in terminal
    │
    ▼
river-tui POSTs to gateway /incoming
    │
    ▼
Gateway processes, agent responds
    │
    ▼
Gateway POSTs to river-tui /send
    │
    ▼
river-tui renders response in terminal
```

Two async tasks inside the process:

1. **TUI task** — crossterm event loop, ratatui rendering. Reads user input, displays messages.
2. **HTTP server task** — axum on a configured port. Receives `/send` from gateway, pushes messages into the TUI's display buffer. Also serves `/health`.

Shared state between the two tasks: a message buffer (append-only vec behind a lock) and a notify channel so the TUI task knows to re-render when a new message arrives from the gateway.

## Crate

`crates/river-tui` in the Cargo workspace. Produces the `river-tui` binary.

Dependencies from workspace: `axum`, `tokio`, `reqwest`, `serde`, `serde_json`, `clap`, `ratatui`, `crossterm`, `chrono`, `tracing`, `tracing-subscriber`, `river-adapter`.

## CLI

```
river-tui \
  --gateway-url http://127.0.0.1:3000 \
  --listen-port 8082 \
  --name cassie \
  --channel terminal \
  --auth-token-file /path/to/token
```

| Flag | Default | Description |
|------|---------|-------------|
| `--gateway-url` | `http://127.0.0.1:3000` | Gateway HTTP URL |
| `--listen-port` | `0` (OS-assigned) | Port for the TUI's HTTP server |
| `--name` | System username | User display name in chat |
| `--channel` | `terminal` | Channel ID for messages |
| `--auth-token-file` | None | Path to file containing gateway auth token |

## TUI Layout

```
┌──────────────────────────────────┐
│ message log (scrollable)         │
│                                  │
│ 00:03 cassie: hello              │
│ 00:03 viola: hi! how can i help? │
│                                  │
│                                  │
├──────────────────────────────────┤
│ [tui ● gateway: connected]       │
├──────────────────────────────────┤
│ > user input here_               │
└──────────────────────────────────┘
```

Three regions:

- **Message log** — scrollable, shows `HH:MM name: content` for each message. Timestamps in local time. Scrolls to bottom on new messages. User can scroll up to read history.
- **Status bar** — adapter name, gateway connection state (connected/disconnected).
- **Input line** — single-line text input with a prompt. Enter sends. Ctrl-C quits.

## HTTP Endpoints

### POST /send

Receives `SendRequest` from gateway (the agent is responding). Pushes the content into the message log as an agent message. Returns `SendResponse` with `success: true`.

```json
// Request (from gateway)
{ "channel": "terminal", "content": "Hello!" }

// Response
{ "success": true, "message_id": "<generated>" }
```

### GET /health

Returns health status.

```json
{ "healthy": true }
```

No `/read`, `/history`, or `/channels`. The TUI does not store message history or manage multiple channels. Feature set is empty.

## Message Flow: User → Gateway

When the user presses Enter:

1. Append the message to the local display buffer (so it appears immediately)
2. POST to `{gateway_url}/incoming`:

```json
{
  "adapter": "tui",
  "event_type": "MessageCreate",
  "channel": "<channel>",
  "author": { "id": "local-user", "name": "<name>", "is_bot": false },
  "content": "<user input>",
  "message_id": "<generated uuid>"
}
```

3. If the POST fails, show an error in the status bar

The auth token (if configured) is sent as `Authorization: Bearer <token>` on the `/incoming` POST.

## Message Flow: Gateway → TUI

When the HTTP server receives `POST /send`:

1. Push the message content into the shared display buffer with the agent's name
2. Notify the TUI task to re-render
3. Return `SendResponse { success: true, message_id: Some("<generated>") }`

## Gateway Registration

On startup, POST to `{gateway_url}/adapters/register`:

```json
{
  "name": "tui",
  "version": "0.1.0",
  "url": "http://127.0.0.1:<listen_port>",
  "features": [],
  "metadata": {}
}
```

Retries up to 3 times with 5-second backoff (same pattern as Discord adapter).

## Gateway Health Check

Background task polls `{gateway_url}/health` every 30 seconds. Updates the status bar connection indicator. Same pattern as Discord adapter.

## Out of Scope

- Orchestrator integration (the TUI is run manually by the user, not spawned by the orchestrator)
- Message persistence / history (messages are lost when the TUI exits)
- Multiple channels
- Markdown rendering
- Mouse support
- Streaming / partial responses

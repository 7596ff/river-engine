# TUI Redesign — Home Channel Viewer

## Goal

Replace the TUI adapter with a home channel viewer. The TUI reads JSONL from stdin, formats it as a chat window, and posts user input to the gateway's bystander endpoint. It is not an adapter. It has no HTTP server. It knows nothing about files or channels.

## Usage

```
tail -f channels/home/iris.jsonl | river-tui --agent iris
tail -f channels/home/iris.jsonl | river-tui --agent iris --gateway-url http://athena:3000
ssh athena tail -f /path/to/channels/home/iris.jsonl | river-tui --agent iris --gateway-url http://athena:3000
```

## CLI

| Flag | Default | Required |
|---|---|---|
| `--agent` | none | yes |
| `--gateway-url` | `http://127.0.0.1:3000` | no |
| `--auth-token-file` | none | no |

## Architecture

Two async tasks:

1. **Stdin reader** — reads lines from stdin, deserializes each as `HomeChannelEntry`, formats via `Display`, pushes to display buffer, notifies TUI to re-render.
2. **TUI task** — crossterm event loop, ratatui rendering. Handles keyboard input. On Enter, POSTs to the bystander endpoint.

Shared state: a display buffer (append-only vec of formatted strings behind a lock) and a tokio Notify for re-render signals.

```
stdin (JSONL) ──► deserialize ──► Display::fmt ──► display buffer ──► ratatui
                                                                        │
keyboard ──► Enter ──► POST /home/{agent}/message ──────────────────────┘
```

## Entry Formatting

Entry types (`HomeChannelEntry`, `MessageEntry`, `ToolEntry`, `HeartbeatEntry`, `CursorEntry`) move from `river-gateway/src/channels/entry.rs` to `river-core`. Each entry type implements `Display`. Timestamps are extracted from the snowflake ID on each entry.

| Entry type | Format |
|---|---|
| message/agent | `2026-05-14 14:03:22 [agent] content` |
| message/user | `2026-05-14 14:03:22 [user:discord] cassie: content` |
| message/bystander | `2026-05-14 14:03:22 [bystander] content` |
| message/system | `2026-05-14 14:03:22 [system] content` |
| tool/tool_call | `2026-05-14 14:03:22 🔧 tool_name(args_summary)` |
| tool/tool_result | appended to call line: `→ N lines` or `→ ok` |
| heartbeat | `2026-05-14 14:03:22 ♡` |
| cursor | `2026-05-14 14:03:22 ┄ read cursor` |

Tool calls and results are paired by `tool_call_id`. The formatter holds pending tool calls in a small map. When a tool result arrives, it renders the combined one-liner: `2026-05-14 14:03:22 🔧 read_file(src/main.rs) → 245 lines`. If a tool call has no result yet, it renders without the arrow. If a tool result arrives without a matching call (e.g., TUI started mid-session), it renders standalone: `2026-05-14 14:03:22 🔧 tool_name → N lines`.

The `Display` impl on individual entry types handles the simple cases (message, heartbeat, cursor). Tool call pairing requires a stateful formatter that is not part of `Display` — it lives in the TUI's stdin reader task (or a thin `HomeChannelFormatter` struct that accumulates entries and emits formatted lines).

## Layout

```
┌──────────────────────────────────────────────┐
│ 2026-05-14 14:03:22 [agent] hi! how can I    │
│ help?                                         │
│ 2026-05-14 14:03:25 [user:discord] cassie:   │
│ hello                                         │
│ 2026-05-14 14:04:01 🔧 read_file(main.rs)    │
│ → 245 lines                                   │
│ 2026-05-14 14:04:03 [bystander] have you     │
│ considered...                                  │
│ 2026-05-14 14:05:11 ♡                         │
│                                               │
├───────────────────────────────────────────────┤
│ [river] iris                                  │
├───────────────────────────────────────────────┤
│ > _                                           │
└───────────────────────────────────────────────┘
```

Three regions:

- **Log** — scrollable, wrapping, auto-follows tail. Up/Down/PageUp/PageDown to scroll. Scrolling up disables auto-follow. Scrolling to bottom re-enables it.
- **Status bar** — agent name.
- **Input** — single-line text input. Enter sends. Ctrl-C quits.

## User Input

On Enter:

1. POST to `{gateway_url}/home/{agent_name}/message` with body `{ "content": "<input>" }`
2. Auth token (if configured) sent as `Authorization: Bearer <token>`
3. If POST fails, show error in status bar (briefly)

The user's message does not appear in the display buffer directly. It will appear when the home channel entry arrives via stdin — the gateway writes bystander messages to the home channel, `tail -f` picks it up, stdin reader deserializes it, and it renders as `[bystander]`. This gives accurate ordering and confirms the message was received.

## Crate

Delete existing `crates/river-tui` entirely. Create new `crates/river-tui`.

**Dependencies:**
- `river-core` (entry types, snowflake timestamp extraction, Display formatting)
- `ratatui`
- `crossterm`
- `tokio`
- `reqwest` (just for the one POST)
- `serde`, `serde_json`
- `clap`
- `chrono`

**Removed dependencies:** `river-adapter`, `axum`, `tower`, `tower-http`.

## What Moves to river-core

From `river-gateway/src/channels/entry.rs`:
- `HomeChannelEntry` enum
- `MessageEntry` struct
- `ToolEntry` struct
- `HeartbeatEntry` struct
- `CursorEntry` struct

Plus:
- `Display` impls for each type
- Snowflake-to-timestamp extraction (may already exist in river-core's snowflake module)

`river-gateway` re-exports or depends on these from `river-core`. No duplication.

## What's Removed

- Adapter registration (`POST /adapters/register`)
- HTTP server (axum, `/send`, `/health`)
- Gateway health polling
- `GatewayClient` (the full client — replaced by a single `post_bystander` function)
- `SharedState` with atomic bools for connection/server status
- `ChatLine` type
- Channel concept
- `server.rs`, `gateway.rs` (entire files)

## Out of Scope

- Multiple view modes or filtering
- Markdown rendering
- Mouse support
- Streaming / partial responses
- Message persistence (the home channel JSONL is the persistence)
- File watching (stdin is the interface)

# TUI Redesign — Home Channel Viewer

## Goal

Replace the TUI adapter with a home channel viewer. The TUI reads JSONL from stdin or tails a file, formats it as a chat window, and posts user input to the gateway's bystander endpoint. It is not an adapter. It has no HTTP server.

## Usage

```
# Tail a file directly
river-tui --agent iris --file channels/home/iris.jsonl

# Pipe from stdin
tail -f channels/home/iris.jsonl | river-tui --agent iris

# Remote via SSH
ssh athena tail -f /path/to/channels/home/iris.jsonl | river-tui --agent iris --gateway-url http://athena:3000
```

## CLI

| Flag | Default | Required |
|---|---|---|
| `--agent` | none | yes |
| `--gateway-url` | `http://127.0.0.1:3000` | no |
| `--file` | none (reads stdin) | no |

Auth token is read from the `RIVER_AUTH_TOKEN` environment variable (consistent with all other river-engine components). The TUI loads `.env` via `dotenvy` on startup.

## Architecture

Two async tasks:

1. **Input reader** — reads lines from stdin or tails a file (`--file`). Deserializes each line as `HomeChannelEntry`, formats it, pushes to display buffer, notifies TUI to re-render. When `--file` is given, the reader opens the file, reads all existing lines from the beginning, then tails new lines. When reading from stdin, it reads lines as they arrive.
2. **TUI task** — crossterm event loop, ratatui rendering. Handles keyboard input. On Enter, POSTs to the bystander endpoint.

Shared state: a display buffer (append-only vec of formatted strings behind a lock) and a tokio Notify for re-render signals.

```
stdin/file (JSONL) ──► deserialize ──► format ──► display buffer ──► ratatui
                                                                     │
keyboard ──► Enter ──► POST /home/{agent}/message ──────────────────┘
```

## Prerequisites (already done)

The snowflake serde refactor has landed:

- `Snowflake` now serializes as a 32-char hex string via custom serde (not `{"high": N, "low": N}`)
- `Snowflake::from_hex()` parses hex strings back to `Snowflake`
- `Snowflake::to_datetime()` computes wall-clock time from embedded birth + timestamp
- All entry type `id` fields are `Snowflake`, not `String`
- JSONL format is unchanged

## Entry Formatting

Entry types (`HomeChannelEntry`, `MessageEntry`, `ToolEntry`, `HeartbeatEntry`, `CursorEntry`) move from `river-gateway/src/channels/entry.rs` to `river-core`. Each type gets a `Display` impl. Timestamps are extracted via `Snowflake::to_datetime()` on the entry's `id` field (already a `Snowflake` — no parsing needed).

### river-core: `Display` (full content)

The `Display` impls in river-core render the full content of each entry. This is what the context builder and logging use. Messages render their complete text. Tool results render their complete output. Nothing is collapsed or summarized.

### river-tui: `TuiEntry` newtype (collapsed)

The TUI wraps entries in a `TuiEntry` newtype with its own `Display` impl. For most entry types, it delegates to the river-core `Display`. For tool entries, it renders collapsed one-liners:

| Entry type        | TUI format                                                           |
| ----------------- | -------------------------------------------------------------------- |
| message/agent     | `2026-05-14 14:03:22 [agent] content`                                |
| message/user      | `2026-05-14 14:03:22 [user:discord] cassie: content`                 |
| message/bystander | `2026-05-14 14:03:22 [bystander] content`                            |
| message/system    | `2026-05-14 14:03:22 [system] content`                               |
| tool/tool_call    | `2026-05-14 14:03:22 🔧 tool_name(args_summary)`                     |
| tool/tool_result  | appended to call line: `→ result_file path` or `→ N lines` or `→ ok` |
| heartbeat         | `2026-05-14 14:03:22 💓`                                             |
| cursor            | `2026-05-14 14:03:22 ┄ read cursor`                                  |

```rust
// in river-tui
struct TuiEntry(HomeChannelEntry);

impl Display for TuiEntry {
    // delegates to inner Display for messages, heartbeats, cursors
    // overrides for tool entries: one-liner summaries
}
```

### Tool call pairing

Tool calls and results are paired by `tool_call_id` in the TUI's `HomeChannelFormatter` — a stateful struct that accumulates entries and emits formatted lines. When a tool result arrives, it renders the combined one-liner: `2026-05-14 14:03:22 🔧 read_file(src/main.rs) → tool-results/abc123.txt` (if result was written to file) or `→ 245 lines` (if inline result, showing line count). If a tool call has no result yet, it renders without the arrow. If a tool result arrives without a matching call (e.g., TUI started mid-session), it renders standalone: `2026-05-14 14:03:22 🔧 tool_name → result_file` or `→ N lines`.

## Layout

```
┌──────────────────────────────────────────────┐
│ 2026-05-14 14:03:22 [agent] hi! how can I    │
│ help?                                        │
│ 2026-05-14 14:03:25 [user:discord] cassie:   │
│ hello                                        │
│ 2026-05-14 14:04:01 🔧 read_file(main.rs)    │
│ → 245 lines                                  │
│ 2026-05-14 14:04:03 [bystander] have you     │
│ considered...                                │
│ 2026-05-14 14:05:11 💓                       │
│                                              │
├──────────────────────────────────────────────┤
│ [river] iris                                 │
├──────────────────────────────────────────────┤
│ > _                                          │
└──────────────────────────────────────────────┘
```

Three regions:

- **Log** — scrollable, auto-follows tail. Up/Down/PageUp/PageDown to scroll. Scrolling up disables auto-follow. Scrolling to bottom re-enables it. Messages can contain newlines. The first line gets the timestamp and role prefix. Continuation lines are indented to align with the content start:
  ```
  2026-05-14 14:03:22 [agent] here is a multi-line response
                              that continues on the next line
                              and the next
  2026-05-14 14:03:25 [user:discord] cassie: hello
  ```
  Long lines within a message wrap at the terminal width, also indented to the content column.
- **Status bar** — agent name.
- **Input** — text input that expands vertically as the message grows beyond one line. The log region shrinks to accommodate. Enter sends. Ctrl-C quits.

## User Input

On Enter:

1. POST to `{gateway_url}/home/{agent_name}/message` with body `{ "content": "<input>" }`
2. Auth token from `RIVER_AUTH_TOKEN` env var sent as `Authorization: Bearer <token>`
3. If POST fails, show error in status bar (briefly)

The user's message does not appear in the display buffer directly. It will appear when the home channel entry arrives via stdin — the gateway writes bystander messages to the home channel, `tail -f` picks it up, stdin reader deserializes it, and it renders as `[bystander]`. This gives accurate ordering and confirms the message was received.

## Crate

Delete existing `crates/river-tui` entirely. Create new `crates/river-tui`.

**Dependencies:**
- `river-core` (entry types, `Snowflake::to_datetime()`, `Display` formatting)
- `ratatui`
- `crossterm`
- `tokio`
- `reqwest` (just for the one POST)
- `serde`, `serde_json`
- `clap`
- `chrono`
- `dotenvy`
- `tracing`, `tracing-subscriber`

**Removed dependencies:** `river-adapter`, `axum`, `tower`, `tower-http`.

## What Moves to river-core

From `river-gateway/src/channels/entry.rs`:
- `HomeChannelEntry` enum
- `MessageEntry` struct
- `ToolEntry` struct
- `HeartbeatEntry` struct
- `CursorEntry` struct
- `ChannelEntry` enum

Plus:
- `Display` impls for each type (using `Snowflake::to_datetime()` for timestamps — no hex parsing needed, `id` is already a `Snowflake`)

`river-gateway` re-exports or depends on these from `river-core`. No duplication.

## What's Removed

- Adapter registration (`POST /adapters/register`)
- HTTP server (axum, `/send`, `/health`)
- Gateway health polling
- `GatewayClient` (the full client — replaced by a single `post_bystander` function)
- `--auth-token-file` CLI flag (auth from env now)
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

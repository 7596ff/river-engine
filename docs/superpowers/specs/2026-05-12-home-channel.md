# Home Channel — Design Spec

The home channel replaces the agent's invisible thinking with a visible, inspectable, append-only log. Every model response, tool call, incoming message, and heartbeat lives in one channel. The model context is a derived view of this log plus spectator moves.

## Home Channel

Every agent has a home channel at `channels/home/{agent_name}.jsonl`. This is the single source of truth for the agent's entire stream of consciousness. The log is strictly append-only — it is never modified after entries are written.

### Snowflake IDs

Every entry in the home channel carries a snowflake ID. Snowflakes are unique, sortable, and encode a timestamp. Moves reference snowflake ranges (e.g., "covers entries from snowflake X to snowflake Y") so the agent can always locate the raw entries a move summarizes.

### Entry Types

**MessageEntry** (existing, extended with new roles):
- `id` — snowflake ID
- `agent` — model responses (all of them, not just send_message)
- `user` — messages from adapters, tagged with source: `[user:discord:789012/general] cassie: hello`
- `system` — system-level notifications
- `bystander` — messages posted directly to the home channel, anonymous by design

**ToolEntry** (new):
- `id` — snowflake ID
- `tool_name` — which tool was called
- `arguments` — the tool call arguments (JSON)
- `result` — the tool result, OR a file path if the result exceeds a size threshold
- `tool_call_id` — the model's tool call ID for threading

Large tool results get written to a file (e.g. `channels/home/{agent_name}/tool-results/{id}.txt`) and the entry contains a link instead of the full content. Tool result files are cleaned up by the log writer actor (not the spectator) when the entries they belong to are superseded by a spectator move — the move summary replaces the need for the raw file. The log writer owns the home channel directory and is the only process that creates or deletes files in it.

**CursorEntry** (existing): read-up-to markers. Carries a snowflake ID.

**HeartbeatEntry** (new):
- `id` — snowflake ID
- Heartbeat wake events
- Only written to the home channel if the heartbeat actually produces a turn (the agent has something to do). No-op heartbeats are transient and don't persist.

### Per-Adapter Logs

`channels/{adapter}/{channel}.jsonl` still tracks what happened on each platform. These are secondary projections — the home channel is written first (write-ahead), adapter logs are written second. If the adapter log write fails, the home channel entry still exists.

Adapter logs exist for easy per-platform context retrieval. The home channel is where the agent thinks.

### Log Writer

All writes to the home channel go through a single serialized writer task in the gateway. This ensures ordering and prevents interleaved entries from concurrent sources (agent task, HTTP handlers, heartbeat timer).

## Turn Cycle

### Trigger

Any write to the home channel triggers a turn notification. This includes user messages (from adapters), bystander messages (direct posts), and system messages. Heartbeats trigger a turn check but only persist an entry if the agent actually acts.

### Batching

Incoming messages don't interrupt mid-turn. They queue up and get injected after tool results, before the next model completion call.

Turn sequence:
1. Wake (notification from home channel)
2. Build context from home channel + moves (derived view)
3. Model completion call
4. Write assistant response to home channel
5. If tool calls:
   a. Write tool call entry to home channel
   b. Execute tool
   c. Write tool result entry to home channel (large results → file link)
   d. Append any batched messages that arrived during execution
   e. Go to 3
6. If no tool calls: check for batched messages that arrived during the model call. If any exist, append them and go to 3. Otherwise, turn complete.

### No More Channel Switching

The agent lives in the home channel. There is no channel switching. `ChannelContext`, `pending_channel_switch`, and `ChannelSwitched` events are removed.

The agent knows where messages came from by their `[user:adapter:channel_id/channel_name]` tags. It uses `send_message` to respond to specific adapters/channels.

## Context Building

The model context is a derived view of the home channel log. It is built ephemerally on each turn — the home channel is never modified. `PersistentContext` is replaced by a context builder that reads the home channel and spectator moves to produce model messages.

### How context is built

1. Read spectator moves (compressed summaries of older history)
2. Find the most recent move's coverage — it summarizes entries up to turn N
3. Read home channel entries after turn N (the "live" tail)
4. Map entries to model messages (see table below)
5. The model sees: moves (compressed past) + recent entries (full resolution present)

The home channel log itself is never modified. "Compression" is ephemeral — the context builder just reads moves instead of old entries. The log retains everything. The agent sees a sliding window: compressed history from moves, full-resolution recent entries from the log tail.

### Mapping entries to model messages

| Home channel entry | Model message |
|---|---|
| `agent` message | assistant message |
| `user` message | user message (tag preserved) |
| `system` message | system message |
| `bystander` message | user message (attributed as bystander) |
| `ToolEntry` (call) | assistant message with tool_use |
| `ToolEntry` (result) | tool result message |
| `HeartbeatEntry` | system message noting the wake |

### Spectator and Moves

The spectator operates as before — it observes the home channel and produces moves/moments. Moves summarize ranges of the home channel, referenced by snowflake range (e.g., "this move covers snowflake X through snowflake Y"). The context builder uses moves to represent compressed history, reading the raw log only for entries after the most recent move's ending snowflake.

This means the spectator is the compressor. It reads the home channel tail, writes move files with snowflake ranges, and the next context build uses those moves instead of the raw entries they summarize.

### Move Verifiability

Because moves reference snowflake ranges and the home channel is append-only and immutable, the agent can always go back and read the raw entries a move was made from. If a move seems wrong or incomplete, the raw log is still there. Moves are summaries, not replacements — the source material is never lost.

### Tool Result Cleanup

When the log writer actor detects that a move has been written covering a range of entries, it cleans up any tool result files referenced by entries in that range. The log writer is the only process that deletes files in the home channel directory. The spectator writes moves but never touches tool result files — this avoids coupling between the observational component and the agent's private files.

## Bystander Interface

Messages posted directly to the home channel become `[bystander]` entries.

**Endpoint:** `POST /home/{agent_name}/message`

Request body:
```json
{
  "content": "the message text"
}
```

No adapter registration. No author identity in the entry. Authentication is required (bearer token) — "anonymous" means the entry has no author field, not that the caller is unauthenticated. This is deferred to the auth spec but noted here as a requirement.

This is the architectural hook for the spectator/bystander split — a separate agent or process can observe and comment on the agent's work.

## What Gets Removed

- `ChannelContext` struct — `agent/channel.rs`
- `pending_channel_switch` field on `AgentTask`
- Channel switching logic in `task.rs` (auto-set from first message, switch commands)
- `ChannelSwitched` event from coordinator events
- `PersistentContext` (replaced by home channel context builder)
- Per-channel context building
- SQL database for message storage (`river_db` message tables) — the home channel replaces this
- Database message persistence in `persist_turn_messages`

## What Gets Added

- `ToolEntry` — new entry type in `channels/entry.rs`
- `HeartbeatEntry` — new entry type in `channels/entry.rs`
- Home channel log — `channels/home/{agent_name}.jsonl`, created at agent birth
- Home channel context builder — reads home channel + moves, maps entries to model messages
- Log writer actor — single serialized writer for all home channel writes, ensures ordering
- Bystander endpoint — `POST /home/{agent_name}/message`
- Home channel writes on: every model response, every tool call/result, every incoming user message
- Message batching — queue incoming home channel messages, inject after tool results before next completion
- Final batch check — always check for batched messages before settling, even if no tool calls occurred

## What Gets Modified

- `AgentTask` — turn cycle writes to home channel, context built from home channel + moves, no channel switching
- `handle_incoming` in `routes.rs` — writes to home channel first (write-ahead), then adapter log
- Heartbeat handler — only writes to home channel if it produces an actual turn
- Spectator — reads from home channel instead of `PersistentContext`, moves serve as compression

## Deferred (separate specs)

- **TUI as home channel viewer** — the TUI becomes a log viewer that tails the home channel and posts to the bystander endpoint. Not an adapter.
- **Authentication** — bearer token for bystander endpoint and all gateway endpoints.
- **Target/focus hinting** — deterministic way to track which adapter/channel the agent should respond to, without relying on model-side tag parsing.

## Scope

This spec covers the gateway changes only. The TUI rewrite, auth system, and focus hinting are follow-up specs. Adapter code (discord, adapter crate) is not modified — adapters still register and receive outbound messages via `send_message` as before.

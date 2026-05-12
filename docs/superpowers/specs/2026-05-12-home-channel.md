# Home Channel — Design Spec

The home channel replaces the agent's invisible thinking with a visible, inspectable, append-only log. Every model response, tool call, incoming message, and heartbeat lives in one channel. The model context is a derived compressed view of this log.

## Home Channel

Every agent has a home channel at `channels/home/{agent_name}.jsonl`. This is the single source of truth for the agent's entire stream of consciousness.

### Entry Types

**MessageEntry** (existing, extended with new roles):
- `agent` — model responses (all of them, not just send_message)
- `user` — messages from adapters, tagged with source: `[user:discord:789012/general] cassie: hello`
- `system` — system-level notifications
- `bystander` — messages posted directly to the home channel, anonymous by design

**ToolEntry** (new):
- `tool_name` — which tool was called
- `arguments` — the tool call arguments (JSON)
- `result` — the tool result, OR a file path if the result exceeds a size threshold
- `tool_call_id` — the model's tool call ID for threading

Large tool results get written to a file (e.g. `channels/home/{agent_name}/tool-results/{id}.txt`) and the entry contains a link instead of the full content.

**CursorEntry** (existing): read-up-to markers, unchanged.

**HeartbeatEntry** (new):
- Heartbeat wake events
- Written to the home channel, triggering a turn like any other entry

### Per-Adapter Logs Remain

`channels/{adapter}/{channel}.jsonl` still tracks what happened on each platform. User messages are written to both the adapter log and the home channel. The adapter log is the record of what happened in a place. The home channel is where the agent thinks about it.

## Turn Cycle

### Trigger

Any write to the home channel triggers a turn notification. This includes user messages (from adapters), bystander messages (direct posts), system messages, and heartbeats. One notification mechanism.

### Batching

Incoming messages don't interrupt mid-turn. They queue up and get injected after tool results, before the next model completion call.

Turn sequence:
1. Wake (notification from home channel)
2. Build context from home channel (compressed/derived view)
3. Model completion call
4. Write assistant response to home channel
5. If tool calls:
   a. Write tool call entry to home channel
   b. Execute tool
   c. Write tool result entry to home channel (large results → file link)
   d. Append any batched messages that arrived during execution
   e. Go to 3
6. If no tool calls: turn complete

### No More Channel Switching

The agent lives in the home channel. There is no channel switching. `ChannelContext`, `pending_channel_switch`, and `ChannelSwitched` events are removed.

The agent knows where messages came from by their `[user:adapter:channel_id/channel_name]` tags. It uses `send_message` to respond to specific adapters/channels.

## Context Building

The model context is a derived view of the home channel log. `PersistentContext` is replaced by a context builder that reads the home channel and produces model messages.

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

### Compression

Rolling compression on the home channel. Old entries get compressed in place — a summary replacing the original content, marked as compressed. The context builder reads backward from the head: full-resolution entries for recent history, compressed entries for older history.

The spectator drives compression as before — it observes the home channel and produces moves/moments. The compaction is how you read the log, not a separate operation.

## Bystander Interface

Messages posted directly to the home channel become `[bystander]` entries.

**Endpoint:** `POST /home/{agent_name}/message`

Request body:
```json
{
  "content": "the message text"
}
```

No adapter registration. No author identity. Bystander messages are anonymous by design. This is the architectural hook for the spectator/bystander split — a separate agent or process can observe and comment on the agent's work.

## What Gets Removed

- `ChannelContext` struct — `agent/channel.rs`
- `pending_channel_switch` field on `AgentTask`
- Channel switching logic in `task.rs` (auto-set from first message, switch commands)
- `ChannelSwitched` event from coordinator events
- `PersistentContext` (replaced by home channel context builder)
- Per-channel context building

## What Gets Added

- `ToolEntry` — new entry type in `channels/entry.rs`
- `HeartbeatEntry` — new entry type in `channels/entry.rs`
- Home channel log — `channels/home/{agent_name}.jsonl`, created at agent birth
- Home channel context builder — reads home channel, maps entries to model messages, applies compression
- Bystander endpoint — `POST /home/{agent_name}/message`
- Home channel writes on: every model response, every tool call/result, every incoming user message, every heartbeat
- Message batching — queue incoming home channel messages, inject after tool results before next completion

## What Gets Modified

- `AgentTask` — turn cycle writes to home channel, context built from home channel, no channel switching
- `handle_incoming` in `routes.rs` — also writes tagged `[user]` entry to home channel
- Heartbeat handler — writes to home channel instead of just waking the agent
- Spectator — reads from home channel instead of `PersistentContext`

## Follow-Up (separate spec)

- **TUI as home channel viewer** — the TUI becomes a log viewer that tails the home channel and posts to the bystander endpoint. Not an adapter. Much simpler architecture.

## Scope

This spec covers the gateway changes only. The TUI rewrite is a follow-up spec. Adapter code (discord, adapter crate) is not modified — adapters still register and receive outbound messages via `send_message` as before.

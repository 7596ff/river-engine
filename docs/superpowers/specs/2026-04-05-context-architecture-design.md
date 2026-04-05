# Context Architecture Redesign

## Problem

context.jsonl grows too fast by storing everything: full prompts, tool calls, tool results, system messages. This bloats the stream of consciousness and conflates what the LLM produced with what was rendered for it.

## Solution

Store only what the LLM produces. Assemble context fresh from workspace data and the LLM's output record.

## Storage

### context.jsonl — Stream of Consciousness

Stores only:
- Assistant messages (LLM outputs, including tool_calls)
- System warnings that affect reasoning (context pressure warnings)

```jsonl
{"role":"assistant","content":"Looking at the bug report...","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_channel","arguments":"{...}"}}]}
{"role":"assistant","content":"The timeout is set to 30 seconds."}
{"role":"system","content":"Context at 80%. Consider wrapping up or using the summary tool."}
{"role":"assistant","content":null,"tool_calls":[{"id":"call_2","type":"function","function":{"name":"speak","arguments":"{...}"}}]}
```

Does NOT store:
- Role/identity prompts (re-rendered each assembly)
- Tool results (live in inbox or are ephemeral)
- Rendered workspace data (moments, moves, messages)

### workspace/inbox/ — Tool Results

Timestamped tool results, scoped by channel.

**Filename format:**
```
{adapter}_{channel_id}_{timestamp}_{tool}.json
```

**Examples:**
```
discord_chan123_2026-04-01T07-28-00Z_read_channel.json
discord_chan123_2026-04-01T07-30-15Z_create_move.json
```

**Content:** Tool-specific result JSON.

**Cleanup:** Agent-managed. No automatic TTL.

**Tools that write to inbox:**
- `read_channel` (actor) — records message range loaded
- `search` (actor) — records search results
- `create_move` (spectator only)
- `create_moment` (spectator only)

**Tools that don't write to inbox:**
- `speak` — effect shows in conversation file
- `switch_channel` — triggers rebuild
- `sleep`, `wake`, `flash` — no persistent result

### Existing Workspace Files

Unchanged:
- `conversations/` — raw messages
- `moves/` — turn summaries `[^]`
- `moments/` — arc summaries `[~]`
- `roles/` — baton-specific prompts
- `shared/` — identity and shared context

## Context Assembly

Assembly happens on:
- Channel switch (`switch_channel` tool)
- Worker respawn (after force_summary or crash)

### Assembly Order

1. **Role** — from `roles/{baton}.md`
2. **Identity** — from `shared/identity.md` (with agent name prepended)
3. **Channel blocks** — one per watched channel, current channel last
4. **Stream of consciousness** — LLM outputs from context.jsonl
5. **New messages** — current channel's messages since last assembly

### Time Window

`build_context` accepts:
- All moments (across channels)
- All moves (across channels)
- All messages (across channels)
- Time window: since last context reset

The function assembles content within the time window, respecting the compression hierarchy: moments summarize moves, moves summarize messages.

### Channel Block Structure

**Current channel:**
```
## #development (current)
[~] msg1000-msg1133 Sprint day 1: Team standup...
[~] msg1134-msg1204 Sprint day 2: Full staging test...
[^] msg1205-msg1215 Bug report about file upload timeouts
[inbox] 07:28 read_channel: msg1150-msg1200
[inbox] 07:30 create_move: msg1205-msg1215
```

Includes: moments, moves, inbox items (full detail)

**Other channels:**
```
## #philosophy
[~] msg1000-msg1050 Discussion about phenomenal consciousness...
[~] msg1051-msg1120 Debate on hard problem; River argued for functionalism
[^] msg1121-msg1135 Side thread about Chalmers' zombie argument
```

Includes: moments, moves only. No inbox items, no notes about agent actions — the channel's memory reflects what happened.

### Within Each Channel

Chronological order with compression hierarchy:
- **Moments** `[~]` summarize ranges of moves
- **Moves** `[^]` summarize ranges of messages
- **Messages** are raw, uncompressed

All sorted by timestamp. A moment appears at the time of its last summarized move. A move appears at the time of its last summarized message. This creates natural chronological flow where compressed summaries precede the detailed content they don't cover.

Inbox items (current channel only) are interspersed by timestamp.

## Normal Operation

While staying in one channel, context grows by appending.

### New Message Arrives

Append directly as user message in conversation format:
```rust
context.push(OpenAIMessage::user("[ ] 2026-04-01 07:32:00 msg1225 <dan:7668> found the issue"));
```

Uses `river_protocol::conversation::format::format_message()`.

### LLM Responds

1. Append response to live context
2. Persist assistant message to context.jsonl

### Tool Called

1. Execute tool
2. If tool writes to inbox: write timestamped result file
3. Append tool result to live context (NOT persisted to context.jsonl)
4. LLM continues

### Context Pressure at 80%

1. Append system warning to live context
2. Persist to context.jsonl
3. LLM should consider wrapping up or compressing

### Context Pressure at 95%

1. `force_summary`: LLM summarizes accomplishments and remaining work
2. Worker exits with summary
3. Orchestrator respawns with fresh context + summary as initial message

## Channel Switch

When agent calls `switch_channel`:

1. **Abandon current context** — it was assembled, not canonical
2. **Rebuild from workspace** — full assembly as described above
3. **Continue** — agent operates in new channel

`switch_channel` returns nothing. The rebuilt context speaks for itself.

## Flashes

Ephemeral messages between agents (spectator ↔ actor).

### Properties

- `id`: snowflake timestamp
- `from`: sender name
- `content`: whatever text the sender writes
- `expires_at`: timestamp, TTL measured in seconds

### Behavior

- **Channel-independent**: flashes appear at their timestamp regardless of which channel the agent is viewing
- Sorted into the timeline by timestamp
- Visible while TTL hasn't expired
- Then gone — not compressed, not archived

### Storage

In-memory only (`pending_flashes` in worker state). Not persisted to workspace files.

## Summary

| What | Where | Persisted |
|------|-------|-----------|
| LLM outputs | context.jsonl | Yes |
| Context warnings | context.jsonl | Yes |
| Tool results | workspace/inbox/ | Yes |
| Messages | workspace/conversations/ | Yes |
| Moves | workspace/moves/ | Yes |
| Moments | workspace/moments/ | Yes |
| Role/identity | workspace/roles/, shared/ | Yes (source files) |
| Flashes | worker memory | No |
| Rendered context | live only | No |
| Tool results in context | live only | No |

# Spectator Redesign — Event-Driven Sweep with Narrative Moves

## Overview

The spectator is an event-driven task that produces narrative move summaries by reading the home channel. It replaces the current per-turn model (one LLM call per TurnComplete, one move per turn) with a periodic sweep model (time-gated, reading the home channel directly, producing one move per sweep).

## Architecture

The spectator listens for `TurnComplete` events on the coordinator bus. On each event, it checks whether ≥N minutes (default 5) have elapsed since the last move was written. If not, it does nothing. If yes, it sweeps:

1. Read home channel entries since the last move's end snowflake
2. Format entries into a transcript (token-budgeted, max 16384 tokens)
3. If entries exceed the token budget, take only the earliest entries that fit — the spectator will catch up in subsequent sweeps
4. Read the last 10 moves from `moves.jsonl` for narrative continuity
5. Send to LLM: spectator identity (system), recent moves + new transcript + instruction (user)
6. LLM returns a plain text narrative summary covering all topics in the entries
7. Write one move to `moves.jsonl` with the snowflake range the spectator already knows (first entry ID to last entry ID) and the LLM's summary
8. Write a system message to the home channel: `[spectator] move written covering entries {start}-{end}`
9. Clean up tool result files in the covered snowflake range
10. Emit `MovesUpdated` on the bus
11. Check if there are more unseen entries beyond the cursor — if so, immediately sweep again (no time gate for catch-up sweeps)

**One sweep, one move.** The spectator owns segmentation (what entries go into this sweep). The LLM owns narration (what to say about them). The LLM does not choose boundaries, produce structured output, or decide how many moves to write. It gets a chunk of entries and writes a summary that covers everything.

The spectator processes one sweep at a time. Events received during a sweep buffer in the channel and are processed when the sweep completes.

The TurnComplete events are the clock. If the agent is idle (no turns completing), no sweeps fire. This is intentional — moves are consumed as compressed history by the context builder, which reads the home channel tail for recent activity. Unswept entries at the tail are already visible to the agent through the raw home channel. Moves only matter once entries have scrolled past the tail window.

## Storage

Moves are stored in a single append-only JSONL file at `channels/home/{agent}/moves.jsonl`.

Each line is one move:

```json
{"start":"185930001","end":"185930042","summary":"The agent configured the nix flake and resolved a build dependency issue. Then the user asked about authentication and the agent explored the existing auth module, identifying a race condition in the token refresh logic."}
```

Fields:
- `start`: snowflake ID of the first home channel entry in this sweep
- `end`: snowflake ID of the last home channel entry in this sweep
- `summary`: narrative summary covering all topics in the sweep (no length limit)

The `start` and `end` are assigned by the spectator from the entries it read, not by the LLM. No rotation for now. The file grows slowly (one entry per sweep) and the context builder only reads the tail.

## LLM Interaction

**System message:** Spectator identity loaded from `spectator/identity.md`.

**User message:** Three sections:

1. **Recent moves** — the last 10 move summaries, providing narrative continuity so the LLM maintains voice and avoids repetition
2. **New entries** — home channel entries formatted with visible snowflake IDs (see Entry Formatting below)
3. **Instruction** — asking the LLM to write a narrative summary that covers all topics in the entries. No length limit. No structured output format. Plain text.

**LLM output:** Plain text. The spectator uses the full response as the move's `summary` field. No parsing required beyond extracting the response content.

**Prompt file:** `spectator/on-sweep.md` — replaces `on-turn-complete.md`. Contains the instruction template with `{{recent_moves}}` and `{{entries}}` placeholders.

## Entry Formatting

Home channel entries are formatted for the LLM with tiered detail levels. The agent's narrative (user and agent messages) is preserved in full. Tool interactions are compressed to names and metadata — the agent's own messages already interpret tool results, so the spectator doesn't need the raw data.

| Entry type | Format |
|---|---|
| User message (with source) | `[{id}] user:{adapter}:{channel_id}/{channel_name} {author}: {content}` |
| Agent message | `[{id}] agent: {content}` |
| Bystander message | `[{id}] bystander: {content}` |
| System message | `[{id}] system: {content}` |
| Tool call | `[{id}] tool_call: {tool_name}` |
| Tool result | `[{id}] tool_result({tool_name}): [{byte_count} bytes]` |
| Heartbeat | filtered out |
| Cursor | filtered out |

## Token Budget

Each sweep has a token budget of 16384 tokens for the entries section. The spectator formats entries from the cursor forward, estimating tokens as it goes, and stops when the budget is reached. If entries exceed the budget, only the earliest entries that fit are included. The cursor advances to the last included entry. On the next sweep (which fires immediately for catch-up), the spectator picks up where it left off.

This handles:
- **First sweep on an old agent:** processes history in chunks, oldest first, multiple sweeps until caught up
- **LLM failure backlog:** recovers in bounded chunks rather than one enormous prompt
- **Normal operation:** most sweeps are well under budget

## Context Builder Changes

`load_moves` in `AgentTask` changes from reading a directory of `.md` files to reading `moves.jsonl`:

1. Read `channels/home/{agent}/moves.jsonl`
2. Parse each line as JSON, extract `summary` field
3. Return last N summaries as `Vec<String>`

The context builder feeds these summaries as system messages to the model, same as before.

## Cursor Tracking

The spectator tracks its own cursor — the `end` snowflake of the last move it wrote. On startup, it reads `moves.jsonl`, finds the last entry's `end` field, and uses that as the starting point. If the file is empty or missing, it starts from the beginning of the home channel.

Since each sweep produces exactly one move, cursor advancement is straightforward: after writing the move, the cursor becomes the `end` snowflake of that move (which is the last entry the spectator read).

This replaces the `first_snowflake`/`last_snowflake` fields on the `TurnComplete` event, which can be removed.

## Observability

The spectator writes a system message to the home channel after each successful sweep:

```
[spectator] move written covering entries 185930001-185930042
```

This is visible to both the agent (in the home channel tail) and operators (in the JSONL file). Uses the existing `MessageEntry` with role `"system"` and adapter `"home"`. The spectator also logs sweep activity via tracing (sweep started, entries read, LLM called, move written, errors).

## Failure Mode

If the LLM call fails, nothing is written. The spectator logs the error and continues listening. The next successful sweep covers a larger window — all entries since the last successful move, up to the token budget. No mechanical fallbacks. Moves are narrative or nothing.

If all entries in a sweep are filtered (heartbeats, cursors, spectator messages), write a move with `"[no activity]"` as the summary to advance the cursor past the noise.

## Cleanup

After writing a move, the spectator cleans up tool result files in the covered snowflake range. The context builder already handles missing tool result files gracefully (`[tool result file missing: {path}]`). The spectator only sweeps after TurnComplete (settle gate), so the agent is between turns during cleanup.

## Configuration

| Field | Default | Description |
|---|---|---|
| `sweep_interval` | 5 minutes | Minimum time between sweeps (ignored during catch-up) |
| `sweep_token_budget` | 16384 tokens | Max tokens for entries in a single sweep |
| `moves_tail` | 10 | Number of recent moves to include as LLM context |
| `moves_path` | `channels/home/{agent}/moves.jsonl` | Path to moves file |
| `home_channel_path` | `channels/home/{agent}.jsonl` | Path to home channel |
| `spectator_dir` | `spectator/` | Directory containing identity and prompt files |

## What Changes

**Removed:**
- `first_snowflake` / `last_snowflake` fields from `TurnComplete` event
- Snowflake tracking in `AgentTask::turn_cycle` (`next_snowflake` helper)
- `on-turn-complete.md` prompt file (replaced by `on-sweep.md`)
- `moves/` directory of `.md` files (replaced by `moves.jsonl`)
- `moments_dir` from SpectatorConfig (moments deferred — separate concern)
- JSONL parsing of LLM output (LLM returns plain text)
- Segmentation logic (spectator owns boundaries, not LLM)

**Added:**
- `on-sweep.md` prompt file with `{{recent_moves}}` and `{{entries}}` placeholders
- Entry formatting function with tiered detail (full text for messages, name-only for tools)
- Token-budgeted entry selection (16384 token cap per sweep)
- Cursor tracking (last move's end snowflake)
- Time-gating logic (check elapsed time on each TurnComplete)
- Catch-up loop (immediate re-sweep if more unseen entries exist)
- Spectator system messages written to home channel for observability

**Modified:**
- `SpectatorTask` — new sweep logic replaces `handle_turn_complete`
- `SpectatorConfig` — simplified (drops `moments_dir`, adds `sweep_interval`, `sweep_token_budget`, `moves_path`)
- `AgentTask::load_moves` — reads `moves.jsonl` instead of directory of `.md` files
- `HomeContextConfig` — no changes needed, context builder works the same

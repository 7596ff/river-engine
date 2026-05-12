# Spectator Redesign — Event-Driven Sweep with Narrative Moves

## Overview

The spectator is an event-driven task that produces narrative move summaries by reading the home channel. It replaces the current per-turn model (one LLM call per TurnComplete, one move per turn) with a periodic sweep model (time-gated, reading the home channel directly, producing one or more moves per sweep based on arcs of work).

## Architecture

The spectator listens for `TurnComplete` events on the coordinator bus. On each event, it checks whether ≥N minutes (default 5) have elapsed since the last move was written. If not, it does nothing. If yes, it sweeps:

1. Read home channel entries since the last move's end snowflake
2. Format entries into a transcript (token-budgeted, max 16384 tokens)
3. If entries exceed the token budget, take only the earliest entries that fit — the spectator will catch up in subsequent sweeps
4. Read the last 10 moves from `moves.jsonl` for narrative continuity
5. Send to LLM: spectator identity (system), recent moves + new transcript + instruction (user)
6. Parse JSONL response — one JSON object per line, each is a move
7. Append each move to `moves.jsonl`
8. Write a system message to the home channel: `[spectator] N moves written covering entries {start}-{end}`
9. Clean up tool result files in the covered snowflake range
10. Emit `MovesUpdated` on the bus
11. Check if there are more unseen entries beyond the cursor — if so, immediately sweep again (no time gate for catch-up sweeps)

The spectator processes one sweep at a time. Events received during a sweep buffer in the channel and are processed when the sweep completes.

The TurnComplete events are the clock. If the agent is idle (no turns completing), no sweeps fire. This is intentional — moves are consumed as compressed history by the context builder, which reads the home channel tail for recent activity. Unswept entries at the tail are already visible to the agent through the raw home channel. Moves only matter once entries have scrolled past the tail window.

## Storage

Moves are stored in a single append-only JSONL file at `channels/home/{agent}/moves.jsonl`.

Each line is one move:

```json
{"start":"185930001","end":"185930042","summary":"The agent configured the nix flake and resolved a build dependency issue."}
{"start":"185930043","end":"185930099","summary":"The user asked about authentication. The agent explored the existing auth module and proposed a token-based approach."}
```

Fields:
- `start`: snowflake ID of the first home channel entry covered by this move
- `end`: snowflake ID of the last home channel entry covered by this move
- `summary`: narrative summary of what happened in this arc

No rotation for now. The file grows slowly (a few entries per sweep) and the context builder only reads the tail.

## LLM Prompt

**System message:** Spectator identity loaded from `spectator/identity.md`.

**User message:** Three sections:

1. **Recent moves** — the last 10 move summaries, providing narrative continuity so the LLM maintains voice and avoids repetition
2. **New entries** — home channel entries since the last move's end snowflake, formatted with visible snowflake IDs (see Entry Formatting below)
3. **Instruction** — asking the LLM to segment the entries into narrative moves and output one JSON object per line with `start`, `end`, and `summary` fields

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

The full tool result content is in the home channel for anyone who needs it. The spectator only needs to know which tools were called and that the agent saw the results — the agent's subsequent messages carry the semantic meaning.

## Token Budget

Each sweep has a token budget of 16384 tokens for the entries section. The spectator formats entries from the cursor forward, estimating tokens as it goes, and stops when the budget is reached. If entries exceed the budget, only the earliest entries that fit are included. The cursor advances to the last included entry. On the next sweep (which fires immediately for catch-up), the spectator picks up where it left off.

This handles:
- **First sweep on an old agent:** processes history in chunks, oldest first, multiple sweeps until caught up
- **LLM failure backlog:** recovers in bounded chunks rather than one enormous prompt
- **Normal operation:** most sweeps are well under budget (5 minutes of activity is typically a few hundred tokens of formatted entries)

## Context Builder Changes

`load_moves` in `AgentTask` changes from reading a directory of `.md` files to reading `moves.jsonl`:

1. Read `channels/home/{agent}/moves.jsonl`
2. Parse each line as JSON, extract `summary` field
3. Return last N summaries as `Vec<String>`

The context builder feeds these summaries as system messages to the model, same as before.

## Cursor Tracking

The spectator tracks its own cursor — the `end` snowflake of the last move it wrote. On startup, it reads `moves.jsonl`, finds the last entry's `end` field, and uses that as the starting point. If the file is empty or missing, it starts from the beginning of the home channel.

The cursor only advances to the `end` of the last successfully written move. If a sweep produces 5 moves but the 3rd fails to parse, moves 1 and 2 are written and the cursor advances to move 2's `end`. The entries for moves 3-5 are re-read on the next sweep.

This replaces the `first_snowflake`/`last_snowflake` fields on the `TurnComplete` event, which can be removed.

## Observability

The spectator writes a system message to the home channel after each successful sweep:

```
[spectator] 2 moves written covering entries 185930001-185930099
```

This is visible to both the agent (in the home channel tail) and operators (in the JSONL file). Uses the existing `MessageEntry` with role `"system"` and adapter `"home"`. The spectator also logs sweep activity via tracing (sweep started, entries read, LLM called, moves written, errors).

## Failure Mode

If the LLM call fails, nothing is written. The spectator logs the error and continues listening. The next successful sweep covers a larger window — all entries since the last successful move, up to the token budget. No mechanical fallbacks. Moves are narrative or nothing.

If a JSONL response line fails to parse, skip that line, log a warning, and continue parsing the remaining lines. The cursor advances only to the last successfully written move.

## Cleanup

After writing moves, the spectator cleans up tool result files in the covered snowflake range. The context builder already handles missing tool result files gracefully (`[tool result file missing: {path}]`). The spectator only sweeps after TurnComplete (settle gate), so the agent is between turns during cleanup.

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
- `tool_result_truncate` config (replaced by tiered formatting — tool results show name + byte count only)

**Added:**
- `on-sweep.md` prompt file with `{{recent_moves}}` and `{{entries}}` placeholders
- Entry formatting function with tiered detail (full text for messages, name-only for tools)
- Token-budgeted entry selection (16384 token cap per sweep)
- JSONL move parsing (response line → `{start, end, summary}`)
- Cursor tracking (last successfully written move's end snowflake)
- Time-gating logic (check elapsed time on each TurnComplete)
- Catch-up loop (immediate re-sweep if more unseen entries exist)
- Spectator system messages written to home channel for observability

**Modified:**
- `SpectatorTask` — new sweep logic replaces `handle_turn_complete`
- `SpectatorConfig` — simplified (drops `moments_dir`, adds `sweep_interval`, `sweep_token_budget`, `moves_path`)
- `AgentTask::load_moves` — reads `moves.jsonl` instead of directory of `.md` files
- `HomeContextConfig` — no changes needed, context builder works the same

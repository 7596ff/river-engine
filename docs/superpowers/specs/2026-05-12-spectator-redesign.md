# Spectator Redesign — Event-Driven Sweep with Narrative Moves

## Overview

The spectator is an event-driven task that produces narrative move summaries by reading the home channel. It replaces the current per-turn model (one LLM call per TurnComplete, one move per turn) with a periodic sweep model (time-gated, reading the home channel directly, producing one or more moves per sweep based on arcs of work).

## Architecture

The spectator listens for `TurnComplete` events on the coordinator bus. On each event, it checks whether ≥N minutes (default 5) have elapsed since the last move was written. If not, it does nothing. If yes, it sweeps:

1. Read home channel entries since the last move's end snowflake
2. Read the last 10 moves from `moves.jsonl` for narrative continuity
3. Format entries as a transcript with visible snowflake IDs
4. Send to LLM: spectator identity (system), recent moves + new transcript + instruction (user)
5. Parse JSONL response — one JSON object per line, each is a move
6. Append each move to `moves.jsonl`
7. Clean up tool result files in the covered snowflake range
8. Emit `MovesUpdated` on the bus

The TurnComplete events are the clock. If the agent is idle (no turns completing), no sweeps fire. The settle gate is built in — sweeps only happen in response to TurnComplete, meaning the agent has finished a turn.

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
2. **New entries** — home channel entries since the last move's end snowflake, formatted with visible snowflake IDs:
   ```
   [185930043] user:discord:general/general cassie: now what about auth?
   [185930044] agent: For authentication, I'd recommend...
   [185930045] tool_call: read_file {"path": "src/auth.rs"}
   [185930046] tool_result: <file contents>
   ```
3. **Instruction** — asking the LLM to segment the entries into narrative moves and output one JSON object per line with `start`, `end`, and `summary` fields

**Prompt file:** `spectator/on-sweep.md` — replaces `on-turn-complete.md`. Contains the instruction template with `{{recent_moves}}` and `{{entries}}` placeholders.

## Entry Formatting

Home channel entries are formatted for the LLM with their snowflake IDs as prefixes:

| Entry type | Format |
|---|---|
| User message (with source) | `[{id}] user:{adapter}:{channel_id}/{channel_name} {author}: {content}` |
| Agent message | `[{id}] agent: {content}` |
| Bystander message | `[{id}] bystander: {content}` |
| System message | `[{id}] system: {content}` |
| Tool call | `[{id}] tool_call: {tool_name} {arguments_json}` |
| Tool result | `[{id}] tool_result({tool_name}): {result}` (or `[file: {path}]` for file results) |
| Heartbeat | `[{id}] heartbeat` |

Tool result content is truncated at a reasonable length (e.g., 500 chars) to avoid flooding the prompt with large outputs. The full content is in the home channel for anyone who needs it.

## Context Builder Changes

`load_moves` in `AgentTask` changes from reading a directory of `.md` files to reading `moves.jsonl`:

1. Read `channels/home/{agent}/moves.jsonl`
2. Parse each line as JSON, extract `summary` field
3. Return last N summaries as `Vec<String>`

The context builder feeds these summaries as system messages to the model, same as before.

## Cursor Tracking

The spectator tracks its own cursor — the `end` snowflake of the last move it wrote. On startup, it reads `moves.jsonl`, finds the last entry's `end` field, and uses that as the starting point. If the file is empty or missing, it starts from the beginning of the home channel.

This replaces the `first_snowflake`/`last_snowflake` fields on the `TurnComplete` event, which can be removed.

## Failure Mode

If the LLM call fails, nothing is written. The spectator logs the error and continues listening. The next successful sweep covers a larger window — all entries since the last successful move. No mechanical fallbacks. Moves are narrative or nothing.

If a JSONL response line fails to parse, skip that line, log a warning, and continue parsing the remaining lines.

## Cleanup

After writing moves, the spectator cleans up tool result files in the covered snowflake range. This is unchanged from the current implementation — `HomeChannelWriter::cleanup_tool_results` reads the home channel, finds `ToolEntry` entries with `result_file` in the range, and deletes the files.

## Configuration

| Field | Default | Description |
|---|---|---|
| `sweep_interval` | 5 minutes | Minimum time between sweeps |
| `moves_tail` | 10 | Number of recent moves to include as LLM context |
| `tool_result_truncate` | 500 chars | Max length of tool result content in the formatted transcript |
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

**Added:**
- `on-sweep.md` prompt file with `{{recent_moves}}` and `{{entries}}` placeholders
- Entry formatting function (home channel entries → transcript with IDs)
- JSONL move parsing (response line → `{start, end, summary}`)
- Cursor tracking (last move's end snowflake)
- Time-gating logic (check elapsed time on each TurnComplete)

**Modified:**
- `SpectatorTask` — new sweep logic replaces `handle_turn_complete`
- `SpectatorConfig` — simplified (drops `moments_dir`, adds `sweep_interval`, `moves_path`)
- `AgentTask::load_moves` — reads `moves.jsonl` instead of directory of `.md` files
- `HomeContextConfig` — no changes needed, context builder works the same

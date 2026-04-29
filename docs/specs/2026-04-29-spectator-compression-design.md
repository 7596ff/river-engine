# Spectator Compression: Moves and Moments

Date: 2026-04-29

## Goal

Rework the spectator's compression pipeline so that moves are LLM-generated structural summaries stored in the database, and moments are LLM-compressed narrative arcs written to embeddings/ for vector indexing. Moves leave `embeddings/` entirely — that directory is for the knowledge layer only.

## Context

The context assembly design (stream/engine/context-assembly-design.md) describes a warm layer of moves that holds the structural arc of the conversation. The v3 spectator implementation has the file layout but none of the intelligence: moves are 80-char truncated one-liners with heuristic classification, moment creation dumps raw text, and the model client is unused in all three spectator jobs. Nothing reads moves back into the context window.

This spec builds the compression pipeline. Context assembly integration is deferred to a separate spec.

### How the spectator gets turn content

The `transcript_summary` field in `TurnComplete` events is currently a stats line (`"Turn 5 completed: 2 messages, 3 tool calls (0 failed)"`), not actual content. The spectator ignores it.

Instead, the spectator queries the messages table directly for the turn's messages using the `turn_number`. This gives it the raw conversational material — user input, assistant response, tool calls and results — and lets it form its own structural summary without depending on the agent to self-summarize.

### Turn numbering

A new `turn_number INTEGER` column is added to the messages table. A turn begins with each new user message. All messages within that conversational cycle (assistant responses, tool calls, tool results, system messages) share the same turn number. The agent increments the turn counter on each new user input.

This is added directly to the existing `001_messages.sql` migration (fresh DB, no backward compatibility needed):

```sql
turn_number INTEGER NOT NULL
```

With index:

```sql
CREATE INDEX IF NOT EXISTS idx_messages_turn ON messages (session_id, turn_number);
```

No separate migration file. The agent sets `turn_number` on every message it persists. The spectator queries:

```sql
SELECT * FROM messages WHERE session_id = ? AND turn_number = ? ORDER BY created_at
```

---

## Database Schema

New migration `004_moves.sql` in `river-db`:

```sql
CREATE TABLE IF NOT EXISTS moves (
    id BLOB PRIMARY KEY,
    channel TEXT NOT NULL,
    turn_number INTEGER NOT NULL,
    summary TEXT NOT NULL,
    tool_calls TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_moves_channel_turn
    ON moves (channel, turn_number);
```

Fields:
- `id` — snowflake ID (same convention as messages)
- `channel` — conversation channel this move belongs to
- `turn_number` — the agent turn this move summarizes
- `summary` — LLM-generated structural summary, 1-2 sentences
- `tool_calls` — JSON array of tool names used this turn (stored for analysis, not used in the summary)
- `created_at` — unix timestamp

New methods on `Database`:
- `insert_move(move: &Move) → RiverResult<()>`
- `get_moves(channel: &str, limit: usize) → RiverResult<Vec<Move>>`  — ordered by turn_number ascending
- `get_max_turn(channel: &str) → RiverResult<Option<u64>>` — the cursor (highest turn number with a move)
- `count_moves(channel: &str) → RiverResult<usize>`

---

## Move Generation

On each `TurnComplete` event, the spectator runtime:

1. Receives `{ channel, turn_number }` from the event
2. Queries the messages table for all messages with that `turn_number` and `session_id` (lock-query-drop)
3. Formats the messages into a readable transcript (role, content, tool calls/results)
4. Loads `workspace/spectator/on-turn-complete.md`, substitutes `{transcript}` and `{turn_number}`
5. Calls the model client with:
   - System prompt: `workspace/spectator/identity.md`
   - User prompt: the substituted `on-turn-complete.md`
6. Writes the LLM response as the `summary` field in a new moves row via `insert_move()` (lock-query-drop)
7. Emits `MovesUpdated { channel }` on the event bus

If `on-turn-complete.md` does not exist, this handler is skipped silently.

**Fallback**: if the model call fails (timeout, connection error, malformed response), write a fallback summary constructed from the message roles and tool names (e.g., "User message → assistant response with tools: read, write") so the moves table always gets an entry. Log a warning. The turn is never lost.

**Ordering guarantee**: the agent must persist all messages for the turn to the database *before* emitting `TurnComplete` on the event bus. This requires a change to `agent/task.rs`: move message persistence before the `bus.publish(TurnComplete)` call.

If the messages query returns empty despite this guarantee (bug or DB error), skip this turn and log an error.

---

## Moment Generation

When `count_moves(channel)` exceeds 50, the spectator considers creating a moment.

### Process

1. Read all moves for the channel from the DB via `get_moves(channel, limit)` (lock-query-drop)
2. Load `workspace/spectator/on-compress.md`, substitutes `{moves}` (formatted move list) and `{channel}`
3. Call the model with:
   - System prompt: `workspace/spectator/identity.md`
   - User prompt: the substituted `on-compress.md`

If `on-compress.md` does not exist, this handler is skipped silently (moves accumulate but no moments are created).
4. The `on-compress.md` prompt must instruct the model to respond in exactly this format:

```
turns: {start}-{end}
---
{narrative paragraph}
```

The model chooses which range of moves forms a coherent arc. It does not have to use all of them.

5. Parse the response with a single method (`parse_moment_response`):
   - Split on first `---`
   - Parse `turns: N-M` from the header (regex: `turns:\s*(\d+)\s*-\s*(\d+)`)
   - Everything after `---` is the narrative
   - If parsing fails (no `---`, no valid turn range, empty narrative), the moment creation fails. Log an error and return. No fallback, no guessing. The moves stay in the DB and compression will be attempted again next time the threshold is checked.

6. On successful parse, write the moment to `embeddings/moments/{channel}-{timestamp}.md`:

```yaml
---
channel: general
turns: 12-34
created: 2026-04-29T22:30:00Z
author: spectator
type: moment
---

The agent spent turns 12 through 34 working through...
```

7. Moves stay in the DB. They are the detailed record. Moments are an interpretive overlay, not a replacement. The spectator may create multiple moments from overlapping or non-contiguous ranges of the same moves.

8. Emit `MomentCreated { summary }` on the event bus

Moments live in `embeddings/moments/` so they are available for vector indexing by the sync service.

---

## Prompt-Driven Spectator

The spectator is a prompt-driven runtime, not a hardcoded pipeline. Its behavior is defined entirely by files in `workspace/spectator/`. The Rust code is a thin event dispatcher: receive event, load the right prompt, assemble context, call LLM, handle the structured output.

### Spectator directory

```
workspace/spectator/
  identity.md           — system prompt for all spectator LLM calls
  on-turn-complete.md   — produces a move (runs on every TurnComplete)
  on-compress.md        — produces a moment (runs when moves exceed 50)
  on-pressure.md        — produces a warning (runs on ContextPressure)
```

`identity.md` is the spectator's system prompt, used in every LLM call. The event-specific files are user prompts. Each event-specific prompt defines what the spectator thinks about; the runtime defines what it does with the output:

| Prompt file | Trigger | Input assembled by runtime | Output type | Runtime action |
|---|---|---|---|---|
| `on-turn-complete.md` | `TurnComplete` event | Messages for this turn (from DB) | Free text (1-2 sentences) | Insert as move in DB |
| `on-compress.md` | Move count > 50 | All moves for channel (from DB) | `turns: N-M\n---\nnarrative` | Parse turn range, write moment file |
| `on-pressure.md` | `ContextPressure` event | Usage percentage | Free text (short warning) | Emit Warning event on bus |

The prompts contain template variables that the runtime substitutes before calling the LLM:

- `on-turn-complete.md` receives `{transcript}` (formatted messages) and `{turn_number}`
- `on-compress.md` receives `{moves}` (all moves, formatted) and `{channel}`
- `on-pressure.md` receives `{usage_percent}`

If a prompt file does not exist, that handler is disabled — the spectator silently skips it. No hardcoded fallbacks for prompt content. The only hardcoded fallback is the model-failure path for move generation (role/tool summary).

### What this replaces

The three-file identity split (AGENTS.md, IDENTITY.md, RULES.md) is replaced by the single `identity.md`. The `Compressor`, `Curator`, and `RoomWriter` structs are replaced by the prompt dispatch runtime. Room notes and curation are removed from this spec — they can be re-added later as additional prompt files if desired.

---

## What Gets Removed

- `embeddings/moves/` directory and all flat file operations
- `Compressor` struct entirely — replaced by prompt dispatch runtime
- `Curator` struct — removed (can return as a prompt file later)
- `RoomWriter` struct — removed (can return as a prompt file later)
- `classify_move()` — removed
- 80-char truncation of transcript summaries
- Three-file spectator identity (AGENTS.md, IDENTITY.md, RULES.md) — replaced by single `identity.md`

## What Stays Unchanged

- Event bus and coordinator — same events, same flow
- Spectator config (model URL, model name, timeouts) — extended but not restructured
- `embeddings/moments/` directory — now the only thing the spectator writes to in embeddings/
- Flash system — stays as infrastructure, just not driven by the spectator in this spec

---

## Changes by Crate

### river-db

- New file: `src/migrations/004_moves.sql`
- New file: `src/moves.rs` — `Move` struct, CRUD methods on `Database`
- `schema.rs` — add `004_moves` to migration list
- `migrations/001_messages.sql` — add `turn_number INTEGER NOT NULL` column and `idx_messages_turn` index to messages table (fresh DB, no backward compat needed)
- `messages.rs` — add `turn_number: u64` field to `Message` struct, update insert/query methods, add `get_turn_messages(session_id, turn_number)` method
- `lib.rs` — add `pub mod moves` and re-exports

### river-gateway (spectator)

The `spectator/` module is rewritten. `compress.rs`, `curate.rs`, `room.rs` are deleted and replaced by a prompt dispatch runtime.

- `mod.rs` — rewritten as the prompt dispatch runtime:
  - `SpectatorConfig` gains `spectator_dir: PathBuf` (default `workspace/spectator/`)
  - `SpectatorTask` holds `Arc<Mutex<Database>>`, `ModelClient`, `EventBus`, `moments_dir: PathBuf`
  - All DB access uses lock-query-drop: acquire mutex, do sync operation, drop guard before any `.await`
  - On startup: load `identity.md` from spectator dir, check which prompt files exist
  - Event dispatch:
    - `TurnComplete` → if `on-turn-complete.md` exists, run move generation handler
    - Move count > 50 → if `on-compress.md` exists, run moment generation handler
    - `ContextPressure` → if `on-pressure.md` exists, run pressure handler
  - `COMPRESSION_MOVES_THRESHOLD` = 50

- `handlers.rs` — new file containing the three handler functions:
  - `handle_turn_complete()` — query messages, format transcript, call LLM, insert move
  - `handle_compress()` — query moves, call LLM, parse turn range, write moment file
  - `handle_pressure()` — call LLM, emit Warning event

- `prompt.rs` — new file for prompt loading and template substitution:
  - `load_prompt(path) → Option<String>` — returns None if file missing
  - `substitute(template, vars: &[(&str, &str)]) → String` — replaces `{key}` with value

### river-gateway (agent)

- `agent/task.rs`:
  - Gains `Arc<Mutex<Database>>` handle (currently absent — must be wired in at construction in `server.rs`)
  - Maintains a `turn_number: u64` counter, incremented on each new user message
  - All messages persisted during that cycle get the current turn number
  - Persists messages to DB *before* emitting `TurnComplete` on the bus (ordering guarantee)
- `server.rs` — passes `Arc<Mutex<Database>>` to `AgentTask` constructor (same instance shared with `AgentLoop` and `SpectatorTask`)
- `loop/mod.rs` (deprecated AgentLoop) — same change if this code path is still active: tag persisted messages with turn number

### New workspace files

- `workspace/spectator/identity.md` — spectator system prompt
- `workspace/spectator/on-turn-complete.md` — move generation prompt (template vars: `{transcript}`, `{turn_number}`)
- `workspace/spectator/on-compress.md` — moment generation prompt (template vars: `{moves}`, `{channel}`)
- `workspace/spectator/on-pressure.md` — pressure warning prompt (template vars: `{usage_percent}`)

All user-editable. The spectator reads them; it does not write them.

`identity.md` is required — the gateway fails to start if it is missing. Event prompt files are optional — if missing, that handler is disabled silently.

---

## Testing

### Delete
- Tests that assert on `embeddings/moves/` file existence
- Tests that check flat file append behavior

### Update
- `test_update_moves_creates_file` → `test_update_moves_inserts_to_db`
- `test_update_moves_appends` → `test_update_moves_sequential_turns`
- `test_create_moment` → verify writes to `embeddings/moments/`, verify YAML frontmatter includes parsed turn range
- `test_should_compress_on_interval` — update threshold to 50
- `test_compression_trigger` — update threshold to 50

### New
- `test_move_insert_and_query` — round-trip through DB
- `test_get_max_turn` — cursor behavior, empty table returns None
- `test_count_moves` — accurate count per channel
- `test_moment_parses_turn_range` — response with `turns: 5-20\n---\nnarrative` correctly parsed
- `test_moment_rejects_missing_separator` — response without `---` returns error
- `test_moment_rejects_missing_turn_range` — response with `---` but no `turns:` line returns error
- `test_moment_rejects_empty_narrative` — response with valid header but empty body returns error
- `test_move_fallback_on_model_failure` — heuristic one-liner written when LLM is unavailable
- `test_moves_persist_after_moment` — moment creation does not delete moves
- `test_get_turn_messages` — messages with matching session_id and turn_number returned in order
- `test_spectator_queries_messages_not_summary` — spectator uses DB messages, not transcript_summary field

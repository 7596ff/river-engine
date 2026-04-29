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

This requires a new migration (`005_messages_turn_number.sql`):

```sql
ALTER TABLE messages ADD COLUMN turn_number INTEGER;
CREATE INDEX IF NOT EXISTS idx_messages_turn ON messages (session_id, turn_number);
```

The agent is responsible for setting `turn_number` on every message it persists. The spectator queries:

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

On each `TurnComplete` event, the spectator:

1. Receives `{ channel, turn_number, tool_calls }` (ignores `transcript_summary`)
2. Queries the messages table for all messages with that `turn_number` and `session_id`
3. Formats the messages into a readable transcript (role, content, tool calls/results)
4. Loads the move prompt from `workspace/spectator/prompts/move.md` (loaded once at startup, cached)
5. Calls the model client with:
   - System prompt: spectator identity (AGENTS.md + IDENTITY.md + RULES.md, concatenated as today)
   - User prompt: the move prompt template with the formatted transcript substituted
6. Writes the LLM response as the `summary` field in a new moves row via `insert_move()`
7. Emits `MovesUpdated { channel }` on the event bus

**Fallback**: if the model call fails (timeout, connection error, malformed response), write a fallback summary constructed from the message roles and tool names (e.g., "User message → assistant response with tools: read, write") so the moves table always gets an entry. Log a warning. The turn is never lost.

If the messages query returns empty (timing issue — messages not yet persisted), skip this turn and log a warning. The move will be missing but subsequent moments can still compress the surrounding turns.

---

## Moment Generation

When `count_moves(channel)` exceeds 50, the spectator considers creating a moment.

### Process

1. Read all moves for the channel from the DB via `get_moves(channel, limit)`
2. Load the moment prompt from `workspace/spectator/prompts/moment.md` (loaded once at startup, cached)
3. Call the model with:
   - System prompt: spectator identity
   - User prompt: the moment prompt template with the full list of moves
4. The prompt instructs the model to respond in this format:

```
turns: {start}-{end}
---
{narrative paragraph}
```

The model chooses which range of moves forms a coherent arc. It does not have to use all of them.

5. Parse the response:
   - Split on first `---`
   - Parse `turns: N-M` from the header (regex: `turns:\s*(\d+)\s*-\s*(\d+)`)
   - Everything after `---` is the narrative
   - **Fallback**: if parsing fails, use the full move range from the DB (`SELECT MIN(turn_number), MAX(turn_number) FROM moves WHERE channel = ?`) and treat the entire response as the narrative

6. Write the moment to `embeddings/moments/{channel}-{timestamp}.md`:

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

## Prompt Files

Two new prompt files, loaded once at spectator startup:

- `workspace/spectator/prompts/move.md` — move generation prompt
- `workspace/spectator/prompts/moment.md` — moment generation prompt

If a prompt file does not exist, fall back to a hardcoded default and log a warning.

The `SpectatorConfig` struct gains a `prompts_dir: PathBuf` field, defaulting to `workspace/spectator/prompts/`.

---

## What Gets Removed

- `embeddings/moves/` directory and all flat file operations
- `Compressor::moves_dir()` — no longer needed
- `Compressor::read_moves()` — replaced by `Database::get_moves()`
- `Compressor::archive_moves()` — moves stay in DB, no archival step
- `Compressor::list_channels()` — replaced by `SELECT DISTINCT channel FROM moves`
- `Compressor::classify_move()` — removed entirely
- 80-char truncation of transcript summaries

## What Stays Unchanged

- Room notes (`RoomWriter`) — separate concern, untouched
- Curator / flash system — separate concern, untouched
- Event bus and coordinator — same events, same flow
- Spectator identity loading (AGENTS.md, IDENTITY.md, RULES.md)
- Spectator config (model URL, model name, timeouts) — extended but not restructured
- `embeddings/moments/` directory — now the only thing the spectator writes to in embeddings/

---

## Changes by Crate

### river-db

- New file: `src/migrations/004_moves.sql`
- New file: `src/migrations/005_messages_turn_number.sql` — adds `turn_number` column and index to messages
- New file: `src/moves.rs` — `Move` struct, CRUD methods on `Database`
- `schema.rs` — add `004_moves` and `005_messages_turn_number` to migration list
- `messages.rs` — add `turn_number: Option<u64>` field to `Message` struct, update insert/query methods, add `get_turn_messages(session_id, turn_number)` method
- `lib.rs` — add `pub mod moves` and re-exports

### river-gateway (spectator)

- `compress.rs` — rewritten:
  - `Compressor` takes a `Database` handle via `Arc<Mutex<Database>>` (matching the existing pattern in `AgentLoop`) instead of `embeddings_dir: PathBuf`
  - Constructor also takes `moments_dir: PathBuf` (for writing moment files to `embeddings/moments/`)
  - `update_moves()` calls LLM then inserts to DB. Falls back to heuristic on model failure.
  - `create_moment()` reads from DB, calls LLM, parses turn range from response, writes to `embeddings/moments/`
  - `count_moves()` delegates to `Database::count_moves()`
  - `classify_move()` removed

- `mod.rs`:
  - `SpectatorConfig` gains `prompts_dir: PathBuf`
  - `SpectatorTask` gains `Arc<Mutex<Database>>` handle
  - `SpectatorTask::new()` takes DB handle parameter (same `Arc<Mutex<Database>>` already used by `AgentLoop`)
  - Prompt files loaded in `run()` at startup alongside identity
  - `COMPRESSION_MOVES_THRESHOLD` changed from 15 to 50
  - `should_compress()` updated to use DB count

### river-gateway (agent)

- `agent/task.rs` — the agent maintains a `turn_number: u64` counter, incremented on each new user message. All messages persisted during that cycle get the current turn number. The `TurnComplete` event continues to carry `turn_number` as it does today.
- `loop/mod.rs` (deprecated AgentLoop) — same change if this code path is still active: tag persisted messages with turn number.

### New workspace files (defaults)

- `workspace/spectator/prompts/move.md`
- `workspace/spectator/prompts/moment.md`

These are user-editable. The spectator reads them; it does not write them.

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
- `test_moment_fallback_on_parse_failure` — malformed response uses full DB range
- `test_move_fallback_on_model_failure` — heuristic one-liner written when LLM is unavailable
- `test_moves_persist_after_moment` — moment creation does not delete moves
- `test_get_turn_messages` — messages with matching session_id and turn_number returned in order
- `test_spectator_queries_messages_not_summary` — spectator uses DB messages, not transcript_summary field

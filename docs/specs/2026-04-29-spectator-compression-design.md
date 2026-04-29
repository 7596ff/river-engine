# Spectator Compression: Moves and Moments

Date: 2026-04-29

## Goal

Rework the spectator's compression pipeline so that moves are LLM-generated structural summaries stored in the database, and moments are LLM-compressed narrative arcs written to embeddings/ for vector indexing. Moves leave `embeddings/` entirely — that directory is for the knowledge layer only.

## Context

The context assembly design (stream/engine/context-assembly-design.md) describes a warm layer of moves that holds the structural arc of the conversation. The v3 spectator implementation has the file layout but none of the intelligence: moves are 80-char truncated one-liners with heuristic classification, moment creation dumps raw text, and the model client is unused in all three spectator jobs. Nothing reads moves back into the context window.

This spec builds the compression pipeline. Context assembly integration is deferred to a separate spec.

### transcript_summary provenance

The `transcript_summary` field in `TurnComplete` events is constructed in `agent/task.rs` via `format!()`, combining the assistant's response content and tool call names. Its length is bounded by the model's output — typically a few hundred to a few thousand characters. This is what the spectator sends to its model for move generation.

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

1. Receives `{ channel, turn_number, transcript_summary, tool_calls }`
2. Loads the move prompt from `workspace/spectator/prompts/move.md` (loaded once at startup, cached)
3. Calls the model client with:
   - System prompt: spectator identity (AGENTS.md + IDENTITY.md + RULES.md, concatenated as today)
   - User prompt: the move prompt template with `{transcript_summary}` and `{tool_calls}` substituted
4. Writes the LLM response as the `summary` field in a new moves row via `insert_move()`
5. Emits `MovesUpdated { channel }` on the event bus

**Fallback**: if the model call fails (timeout, connection error, malformed response), write the raw `transcript_summary` truncated to 200 chars as the move summary so the moves table always gets an entry. Log a warning. The turn is never lost.

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
- New file: `src/moves.rs` — `Move` struct, CRUD methods on `Database`
- `schema.rs` — add `004_moves` to migration list
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

# Context Persistence Design

## Overview

Replace the current ephemeral in-memory context with file-backed persistence. Context is stored as JSONL in the workspace during active use, archived to SQLite on rotation.

## Goals

- **Continuity across restarts**: Resume conversation after process restart
- **Crash recovery**: File-based persistence survives unexpected termination
- **Archival**: Rotated contexts stored in database with summaries
- **Token awareness**: Warn at 80%, auto-rotate at 90%

## Data Model

### New Snowflake Type

Add to `crates/river-core/src/snowflake/types.rs`:

```rust
Context = 0x06
```

### Database Schema

New migration `003_contexts.sql`:

```sql
CREATE TABLE IF NOT EXISTS contexts (
    id BLOB PRIMARY KEY,              -- 128-bit snowflake (type 0x06)
    archived_at BLOB,                 -- Snowflake generated at rotation, NULL while active
    token_count INTEGER,              -- Last known prompt_tokens from API
    summary TEXT,                     -- Summary provided at rotation
    blob BLOB                         -- JSONL content, NULL while active
);
```

Notes:
- `created_at` not needed - timestamp encoded in `id` snowflake
- `archived_at` is a snowflake for future-proofing

### Context File

- Location: `{workspace}/context.jsonl`
- Format: One `ChatMessage` JSON object per line
- Excludes system prompt (loaded separately)

## Context Lifecycle

### Startup

1. Query DB: `SELECT * FROM contexts ORDER BY id DESC LIMIT 1`
2. If no row exists OR latest row has `blob IS NOT NULL` (archived):
   - Generate new Context snowflake
   - Insert row with `id`, all other fields NULL
   - Create empty `context.jsonl`
3. If latest row has `blob IS NULL` (active, unarchived):
   - Resume: load `context.jsonl` into memory
   - Use existing context ID

### During Loop

1. **On wake**: Load `context.jsonl` → parse JSONL → prepend system prompt → send to API
2. **After model response**: Append new messages to `context.jsonl`
3. **Token tracking**: Check `prompt_tokens` from API response
   - At 80%: inject warning system message (ephemeral, not persisted)
   - At 90%: trigger auto-rotation

### Rotation (Manual)

Triggered via `rotate_context` tool with required `summary` parameter:

1. Read `context.jsonl` contents
2. Generate archive snowflake
3. Update row: `SET archived_at = ?, token_count = ?, summary = ?, blob = ?`
4. Generate new Context snowflake
5. Insert new row with `id` only
6. Write new `context.jsonl` with single system message containing summary

### Rotation (Automatic at 90%)

1. Archive current context with `summary = NULL`
2. Create new context row
3. Write empty `context.jsonl` (no summary carried forward)
4. Log warning: "Context auto-rotated at {percent}% - no summary preserved"

## File Format

### JSONL Structure

```jsonl
{"role":"user","content":"[general] Alice: hello"}
{"role":"assistant","content":"Hello! How can I help?"}
{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{\"path\":\"foo.txt\"}"}}]}
{"role":"tool","tool_call_id":"call_1","content":"file contents here"}
```

### What Gets Appended

- User messages (formatted as `[channel] author: content`)
- Assistant responses (content and/or tool_calls)
- Tool results

### What Does NOT Go in File

- System prompt (loaded from `AGENTS.md`, `IDENTITY.md`, etc.)
- Context status messages (ephemeral)
- 80% warning message (injected at runtime)

### Loading Sequence

1. Assemble system prompt from workspace files
2. If resuming from rotation: inject summary as system message
3. Parse `context.jsonl` line by line → append each as `ChatMessage`

## Tool Changes

### Updated `rotate_context` Tool

```json
{
  "type": "object",
  "properties": {
    "summary": {
      "type": "string",
      "description": "Summary of current context to carry forward. This becomes a system message in the new context."
    }
  },
  "required": ["summary"]
}
```

The optional `reason` parameter is removed - summary serves that purpose.

### Warning Injection

After API response, if `prompt_tokens >= context_limit * 0.80` and below 90%:

```
Inject system message: "Context at {percent}%. Consider summarizing and rotating soon."
```

This warning is ephemeral - not persisted to file.

## Code Changes

### New Files

| File | Purpose |
|------|---------|
| `crates/river-gateway/src/db/migrations/003_contexts.sql` | Schema migration |
| `crates/river-gateway/src/db/contexts.rs` | Context DB operations |
| `crates/river-gateway/src/loop/persistence.rs` | JSONL file read/write/append |

### Modified Files

| File | Changes |
|------|---------|
| `crates/river-core/src/snowflake/types.rs` | Add `Context = 0x06` |
| `crates/river-gateway/src/db/mod.rs` | Export contexts module |
| `crates/river-gateway/src/db/schema.rs` | Run new migration |
| `crates/river-gateway/src/loop/mod.rs` | Integrate context persistence, startup logic, file append |
| `crates/river-gateway/src/loop/context.rs` | Load from file instead of rebuilding, inject warnings |
| `crates/river-gateway/src/tools/scheduling.rs` | Update `rotate_context` - require summary, trigger archival |
| `docs/snowflake-generation.md` | Document new Context type |

### Key Structs

```rust
// db/contexts.rs
pub struct Context {
    pub id: Snowflake,
    pub archived_at: Option<Snowflake>,
    pub token_count: Option<i64>,
    pub summary: Option<String>,
    pub blob: Option<Vec<u8>>,
}

// loop/persistence.rs
pub struct ContextFile {
    path: PathBuf,
}

impl ContextFile {
    pub fn create(path: &Path) -> Result<Self>;
    pub fn append(&self, message: &ChatMessage) -> Result<()>;
    pub fn load(&self) -> Result<Vec<ChatMessage>>;
    pub fn read_raw(&self) -> Result<Vec<u8>>;
}
```

## Error Handling

### File/DB Inconsistencies

| Scenario | Resolution |
|----------|------------|
| DB says active context, file missing | Create empty file, log warning, continue |
| DB says active context, file corrupted | Archive what parses, start fresh, log error |
| File exists, no DB row | Orphan file - delete it, start fresh |
| File exists, DB row archived | Stale file - delete it, start fresh |

### Rotation Failures

| Scenario | Resolution |
|----------|------------|
| DB write fails during archive | Keep file intact, retry next cycle |
| File write fails for new context | Rollback DB transaction, keep old context |

### Concurrent Access

- Single agent per workspace assumed
- No file locking needed initially

## Testing Strategy

### Unit Tests

| Module | Tests |
|--------|-------|
| `snowflake/types.rs` | `Context` type is `0x06`, roundtrip serialization |
| `db/contexts.rs` | Insert, query latest, update on archive, NULL handling |
| `loop/persistence.rs` | JSONL append, parse, corrupted line handling, empty file |

### Integration Tests

| Scenario | Verification |
|----------|--------------|
| Fresh start | Creates context row + file |
| Resume after crash | Loads existing file, same context ID |
| Clean rotation | Archives to DB, summary in new file |
| 80% warning | Warning injected, not persisted |
| 90% auto-rotate | Archived with NULL summary |
| Restart after rotation | Starts fresh |

### Manual Testing

1. Start agent, send messages, verify file grows
2. Kill process mid-conversation, restart, verify continuity
3. Fill context to 80%, verify warning
4. Fill to 90%, verify auto-rotation

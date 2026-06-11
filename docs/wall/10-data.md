# 10 — Data

One SQLite database per agent, in its data directory. Workspace files
beside it. Nothing else holds state. This chapter is the truth
hierarchy, the ID scheme, and the schemas.

## IDs

Every engine-generated identifier is a **ULID**: 48 bits of millisecond
timestamp + 80 bits of randomness, lexically sortable, collision-safe
under concurrency, generated anywhere without coordination, via a
standard crate. ULIDs order everything: channel entries, messages,
queue items. There is no custom ID scheme, no ID service, and no
meaning packed into ID bits — provenance facts (like birth) are
records, not encodings.

## The truth hierarchy

| tier | contents | on loss |
|---|---|---|
| **ground truth** | workspace files (identity, knowledge, channel logs) and the record tables (birth, messages, moves) | unrecoverable — this is the life |
| **derived** | vector index, sync file-hashes | rebuilt automatically from the workspace |
| **ephemeral** | activation scores, extraction queue | warmth and pending digestion lost; knowledge untouched |

Backup policy follows directly: back up the workspace and the record
tables; everything else regenerates. (The extraction queue is
technically durable in SQLite, but the design treats its loss as
acceptable: the witness gleans again.)

## Schema

```sql
-- the founding record (ch. 08); exactly one row
CREATE TABLE birth (
    id          TEXT PRIMARY KEY,          -- ULID
    name        TEXT NOT NULL,             -- "i am <name>"
    born_at     INTEGER NOT NULL           -- unix seconds
);

-- every context message, persisted append-time (ch. 01)
CREATE TABLE messages (
    id           TEXT PRIMARY KEY,         -- ULID
    channel      TEXT NOT NULL,
    role         TEXT NOT NULL,            -- user | assistant | tool
    content      TEXT,
    tool_calls   TEXT,                     -- JSON, assistant rows
    tool_call_id TEXT,                     -- tool rows
    turn_number  INTEGER NOT NULL,
    created_at   INTEGER NOT NULL
);
CREATE INDEX idx_messages_turn    ON messages (turn_number);
CREATE INDEX idx_messages_channel ON messages (channel, turn_number);

-- the witness's compressions (ch. 04)
CREATE TABLE moves (
    id          TEXT PRIMARY KEY,          -- ULID
    channel     TEXT NOT NULL,
    turn_number INTEGER NOT NULL,
    summary     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX idx_moves_channel_turn ON moves (channel, turn_number);
-- the witness cursor is MAX(turn_number) per channel: derived, never stored

-- digestion queue (ch. 02)
CREATE TABLE extraction_queue (
    id          TEXT PRIMARY KEY,          -- ULID; FIFO by ULID order
    candidate   TEXT NOT NULL,             -- the witness's prose
    created_at  INTEGER NOT NULL
);

-- activation scores (ch. 02); ephemeral
CREATE TABLE activation (
    note_id     TEXT PRIMARY KEY,          -- atomic note ULID
    score       REAL NOT NULL,
    bumped_at   INTEGER NOT NULL
);

-- vector index (ch. 02); derived
CREATE TABLE segments (
    id          TEXT PRIMARY KEY,          -- ULID
    file_path   TEXT NOT NULL,
    seq         INTEGER NOT NULL,          -- segment order in file
    text        TEXT NOT NULL,
    embedding   BLOB NOT NULL              -- f32 vector
);
CREATE INDEX idx_segments_path ON segments (file_path);

-- sync state (ch. 02); derived
CREATE TABLE file_hashes (
    file_path   TEXT PRIMARY KEY,
    hash        TEXT NOT NULL,
    indexed_at  INTEGER NOT NULL
);
```

Migrations are embedded in the binary and run at startup, tracked in a
`migrations` table. Vector search may use an extension (e.g.
sqlite-vec) or in-process cosine over the blobs — an implementation
choice; the schema above is the floor, and the builder may add an
extension's virtual table beside it.

What is deliberately **not** here: sessions (one agent, one life — no
session table), conversation archives (the channel logs are the
archive), summaries-of-summaries, soft-delete flags, anything not
written by the live path.

## Invariants

These bind every component that touches the database:

- **Persist-once** (ch. 01): a message row is inserted exactly once,
  at context-append time, with the turn number it was appended under.
  Nothing ever re-inserts, re-tags, or back-fills message rows.
- **Turn-atomicity:** all rows of a turn share its turn_number; any
  consumer that drops or loads by turn does so for whole turns.
- **The cursor is derived.** `MAX(turn_number)` over a channel's
  moves, computed at need, never cached in a column.
- **Single writer per concern.** The agent task writes messages; the
  witness writes moves and the queue; the memory system writes
  activation, segments, hashes. No table has two writers.
- **WAL mode**, busy-timeout configured; SQLite serializes the rest.

## Contracts

- ULIDs everywhere; no custom ID bits; provenance is records.
- The truth hierarchy governs backup and rebuild behavior; derived
  and ephemeral tiers must be safely deletable at rest.
- The schema floor above; additions allowed, removals not.
- The five invariants are law for every component.

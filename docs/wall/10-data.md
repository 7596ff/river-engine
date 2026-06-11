# 10 — Data

The agent's life is files. The database is a disposable cache. This
chapter is the truth hierarchy, the ID scheme, the record file formats,
and what little schema the database holds.

## IDs

Every engine-generated identifier is a **ULID**: 48 bits of millisecond
timestamp + 80 bits of randomness, lexically sortable, collision-safe
under concurrency, generated anywhere without coordination, via a
standard crate. ULIDs order everything: channel entries, record lines,
queue items. There is no custom ID scheme, no ID service, and no
meaning packed into ID bits — provenance facts (like birth) are
records, not encodings.

## The truth hierarchy

| tier | where | contents | on loss |
|---|---|---|---|
| **ground truth** | workspace files | identity files, knowledge, channel logs, the turn record, moves, birth | unrecoverable — this is the life |
| **derived** | sqlite | vector index (segments), sync file-hashes | rebuilt automatically from the workspace |
| **ephemeral** | sqlite | activation scores, extraction queue | warmth and pending digestion lost; the witness gleans again |

The consequence, stated plainly: **the database is disposable.** Delete
`river.db` and the engine rebuilds everything it needs from the
workspace. Backup policy is therefore one line: back up the workspace.
Since the workspace is plain text, "backup" can be `git`.

## The record files

All record files are append-only JSONL in the workspace, one JSON
object per line, written by exactly one writer, fsynced on append.
Readers skip malformed lines with a logged warning (a crash mid-append
must never poison a file).

**`record/birth.json`** — the founding record (ch. 08). Not JSONL; one
object, written once by the birth ritual:

```json
{"id":"01JXX...","name":"ada","born_at":"2026-06-11T03:00:00Z"}
```

**`record/{channel}.jsonl`** — the turn record. Every context message,
one line, appended at the moment it enters the context (ch. 01):

```json
{"id":"01JXX...","turn":41,"role":"user","content":"[discord_general] cassie: hello"}
{"id":"01JXX...","turn":41,"role":"assistant","content":"morning","tool_calls":null}
{"id":"01JXX...","turn":42,"role":"assistant","content":null,
 "tool_calls":[{"id":"call_1","name":"read","arguments":"{\"path\":\"notes.md\"}"}]}
{"id":"01JXX...","turn":42,"role":"tool","tool_call_id":"call_1","content":"..."}
```

Fields: `id` (ULID), `turn` (the turn number it was appended under),
`role` (`user` | `assistant` | `tool` | `system`), `content`,
`tool_calls` (assistant lines), `tool_call_id` (tool lines). The ULID
carries the timestamp; no separate time field.

**`record/moves/{channel}.jsonl`** — the witness's compressions
(ch. 04). One line per turn:

```json
{"id":"01JXX...","turn":41,"summary":"Cassie asked about X; you answered from the notes and flagged an open question."}
```

**The cursor is the tail.** The witness cursor for a channel — the
highest turn compressed into a move — is the `turn` field of the last
line of its moves file. Read the tail; no index, no query, no stored
state. Compaction (ch. 03) and session start read it the same way.

**Reading by turn** is a scan: session start and compaction backfill
read `record/{channel}.jsonl` from the end, collecting whole turns
until they have what they need; the witness reads a turn's lines the
same way. At a personal scale — thousands of lines per channel — the
scan is microseconds. This design does not serve analytical queries;
the knowledge layer exists for what the record cannot answer.

**`channels/*.jsonl`** is specified in ch. 05 and unchanged here: the
wire record, with its own entry format and cursor semantics.

## The database

One SQLite file per agent in its data directory, WAL mode, embedded
migrations. It holds only the derived and ephemeral tiers:

```sql
-- digestion queue (ch. 02); ephemeral
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

Vector search may use an extension (e.g. sqlite-vec) or in-process
cosine over the blobs — an implementation choice; the schema above is
the floor, and the builder may add an extension's virtual table beside
it.

What is deliberately **not** stored anywhere: sessions (one agent, one
life), conversation archives separate from the logs (the logs are the
archive), summaries-of-summaries, soft-delete flags, anything not
written by the live path.

## Invariants

These bind every component that touches the record or the database:

- **Persist-once** (ch. 01): a record line is appended exactly once,
  at context-append time, with the turn number it was appended under.
  Nothing ever re-appends, re-tags, or back-fills record lines.
- **Append-only.** Record files are never edited or truncated by the
  engine. Corrections are new lines, history is history.
- **Turn-atomicity:** all lines of a turn share its turn number; any
  consumer that drops or loads by turn does so for whole turns.
- **The cursor is the tail.** Derived from the last line of the moves
  file at need, never cached elsewhere.
- **Single writer per file.** The agent task writes the turn record;
  the witness writes moves; adapter inbound writes channel logs (one
  writer task per file). The memory system alone writes the database.
- **Torn-line tolerance** everywhere a JSONL file is read.
- **Disposability.** The database must be safely deletable at rest;
  startup with a missing database rebuilds derived state and starts
  ephemeral state empty, with no other behavioral change.

## Contracts

- ULIDs everywhere; no custom ID bits; provenance is records.
- Ground truth is workspace files only; the database is disposable.
- The record file formats above are the floor; fields may be added,
  never removed or repurposed.
- The seven invariants are law for every component.

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
| **ground truth** | workspace files | identity files, knowledge, channel logs, the turn record, moves, birth, witness glean-log, witness rejections | unrecoverable — this is the life |
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

**`record/turns.jsonl`** — the turn record, one stream for the whole
life. Every context message, one line, appended at the moment it
enters the context (ch. 01), tagged with the channel it concerns:

```json
{"id":"01JXX...","turn":41,"channel":"discord_general","role":"user","content":"[discord_general] cassie: hello"}
{"id":"01JXX...","turn":41,"channel":"discord_general","role":"assistant","content":"morning","tool_calls":null}
{"id":"01JXX...","turn":42,"channel":"local_main","role":"assistant","content":null,
 "tool_calls":[{"id":"call_1","name":"read","arguments":"{\"path\":\"notes.md\"}"}]}
{"id":"01JXX...","turn":42,"channel":"local_main","role":"tool","tool_call_id":"call_1","content":"..."}
```

Fields: `id` (ULID), `turn` (the turn number it was appended under),
`channel` (inbound messages carry their home channel; everything else
carries the channel the turn was facing), `role` (`user` | `assistant`
| `tool` | `system`), `content`, `tool_calls` (assistant lines),
`tool_call_id` (tool lines). The ULID carries the timestamp; no
separate time field. One agent, one life, one file: a turn that reads
three channels is still one turn in one place, and no exchange is ever
invisible to a channel it touched.

**`record/moves.jsonl`** — the witness's compressions (ch. 04). One
stream, one line per turn:

```json
{"id":"01JXX...","turn":41,"summary":"Cassie asked about X; you answered from the notes and flagged an open question."}
```

**The cursor is the contiguous frontier.** The witness cursor — the
highest turn through which *every* turn has a move — is computed from
the moves file's turn numbers: sort, walk from the first move until
the first gap. A gapless file (the normal state) makes this the tail;
no index, no query, no stored state. Compaction (ch. 03) and session
start read it the same way. The frontier, not the raw tail, is what
keeps the lossless guarantee honest against hand edits: deleting a
move line makes those turns undroppable again until the witness
regenerates them from the record, and a backfilled move appends at
the tail out of turn order — readers of the moves file sort by turn,
never trust file order.

**Reading by turn** is a scan: session start and compaction backfill
read `record/turns.jsonl` from the end, collecting whole turns that
touch the wanted channel until they have what they need; the witness
reads a turn's lines the same way. At a personal scale — thousands of
lines — the scan is microseconds. This design does not serve
analytical queries; the knowledge layer exists for what the record
cannot answer.

**`channels/*.jsonl`** is specified in ch. 05 and unchanged here: the
wire record, with its own entry format and cursor semantics.

**`witness/rejections.jsonl`** — append-only record of the agent's
rejections (ch. 04 rejection memory). One line per
`reject_candidate` call:

```json
{
  "candidate_id": "01JXP...",
  "candidate": "<full text of the rejected candidate>",
  "reason": "warm goodnight, not a claim",
  "turn": 731,
  "at": "2026-06-17T03:14:22Z"
}
```

The `candidate_id` cross-references `glean-log.jsonl` and the
disposable extraction queue. `reason` is omitted when the agent
called the tool without an argument. The witness reads the last N
entries before each glean and renders them into the prompt; the file
itself survives data_dir disposal so the witness's memory of what
didn't land persists across SQLite resets.

**`witness/glean-log.jsonl`** — append-only receipts for queued
extraction candidates (ch. 04 refractory). One line per enqueue:

```json
{"id":"01JXP...","turn":47,"at":"2026-06-16T03:14:22Z"}
```

The `id` matches the candidate's row id in the disposable extraction
queue, so the log cross-references the queue while surviving it. The
tail's `turn` is the witness's `last_glean_through` — the gate
recovers from the file alone, with no SQLite dependency. Hand-deleting
the log resets the gate to "open"; deleting individual lines is the
same idiom as hand-editing `moves.jsonl` (the next glean reads what's
left and acts accordingly).

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
- **The cursor is the contiguous frontier.** Derived from the moves
  file's turn numbers at need (the tail when gapless), never cached
  elsewhere. Moves readers sort by turn, never trust file order.
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

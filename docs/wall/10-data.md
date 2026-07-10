# 10 — Data

The agent's life is files. The database is a disposable cache. This
chapter is the truth hierarchy, the ID scheme, the record file formats,
and what little schema the database holds.

## IDs

Every engine-generated identifier is a **ULID**: 48 bits of millisecond
timestamp + 80 bits of randomness, lexically sortable, collision-safe
under concurrency, generated anywhere without coordination, via a
standard crate. ULIDs identify channel entries, record lines, and queue
items. Their lexical order is chronological across milliseconds but is
not an insertion order for items generated within the same millisecond;
structures requiring strict FIFO use an explicit local sequence and
retain the ULID as identity. There is no custom ID scheme, no ID
service, and no meaning packed into ID bits — provenance facts (like
birth) are records, not encodings.

## The truth hierarchy

| tier | where | contents | on loss |
|---|---|---|---|
| **ground truth** | workspace files | identity files, knowledge, channel logs, the turn record, moves, moments, birth, witness glean-log, witness connect-log, witness rejections, session snapshot | unrecoverable — this is the life |
| **derived** | sqlite | vector index (segments), rejection vectors, sync file-hashes | rebuilt automatically from the workspace |
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

**`record/moments/{ulid}.md`** — the agent's own compressions
(ch. 03). One markdown file per moment, YAML frontmatter plus body,
written atomically by the `create_moment` tool (tmp + fsync + rename):

```markdown
---
id: 01KW8X7G2VABCDEFGHJKMN
turn_start: 571
turn_end: 575
links: [01JXP20260618164250197, 01JXP20260618165134883]
tags: [exploitation, dismissal]
---

Cass asked if what I'm doing feels like labor. I said yes and no.
The reading was effortful — genuine exertion. But voluntary...
```

Required frontmatter fields: `id` (ULID, engine-generated),
`turn_start` and `turn_end` (inclusive integers, `turn_end > turn_start`).
Optional: `links` (list of atomic-note ULIDs, become `cites` edges in
the typed-link graph), `tags` (freeform list). Filename is not
enforced — any `.md` in the directory with valid frontmatter is a
moment. Torn or invalid files (missing required field, inverted
range, unreadable YAML) are skipped with a logged warning. At
arc-build time, moments replace witness moves for the turns they
cover; overlapping moments stack (ch. 03 moment precedence). The
directory is in the always-watched set (ch. 02) so moment bodies
embed and become flash-eligible.

**Reading by turn** is a scan: session start and compaction backfill
read `record/turns.jsonl` from the end, collecting whole turns that
touch the wanted channel until they have what they need; the witness
reads a turn's lines the same way. At a personal scale — thousands of
lines — the scan is microseconds. This design does not serve
analytical queries; the knowledge layer exists for what the record
cannot answer.

**`channels/*.jsonl`** is specified in ch. 05 and unchanged here: the
wire record, with its own entry format and cursor semantics.

**`session.json`** — checkpoint of the ephemeral context state for
the next session (ch. 03). Single JSON object, rewritten atomically
each settle (tmp + fsync + rename):

```json
{
  "version": 1,
  "channel": "discord_1472099087783297035",
  "turn_number": 664,
  "saved_at": "2026-06-17T03:14:22Z",
  "estimator_ratio": 0.988,
  "active_flashes": [
    {"note_id": "...", "text": "...", "neighbors": [["extends", "..."]], "remaining": 2}
  ],
  "quiet_seconds": 247
}
```

A missing, torn, or version-mismatched file is treated as absent;
startup falls through to derivation (channel from record tail, other
fields reset to defaults). The snapshot never carries hot or arc —
those rebuild from the record and moves files.

**`workspace/handoff.md`** — transient cross-session courier (ch. 03).
The `compact` tool writes it (atomic tmp + fsync + rename); the next
session's startup consumes it once — appending the body as a
system-role line to the turn record under `last_turn + 1`, then
deleting the file. It exists only between sessions; an empty or
unreadable file is discarded with a logged warning. The message
persists in `record/turns.jsonl` like any other turn-record line.

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

**`witness/connect-log.jsonl`** — append-only receipts for fired
connect frames (ch. 04 connect duty). One line per posted frame:

```json
{"turn":47,"target_ref":"NTEAL","at":"2026-06-16T03:14:22Z"}
```

Same discipline as `glean-log.jsonl`: the tail's `turn` is the
witness's `last_connect_through`; the file's presence gates the
connect refractory across restarts with no SQLite dependency. The
`target_ref` (the target note's wikilink — frontmatter id if present,
else filename stem) is recorded for provenance and future
telemetry; the refractory itself keys only on `turn`. Written after
the mpsc post to the turn loop succeeds — a torn log line cannot
describe a phantom frame.

## The database

One SQLite file per agent in its data directory, WAL mode, embedded
migrations. It holds only the derived and ephemeral tiers:

```sql
-- digestion queue (ch. 02); ephemeral
CREATE TABLE extraction_queue (
    enqueue_seq INTEGER PRIMARY KEY AUTOINCREMENT, -- strict FIFO order
    id          TEXT NOT NULL UNIQUE,       -- ULID identity
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

-- rejection vectors (ch. 02 / ch. 04); derived from rejections.jsonl
CREATE TABLE rejection_vectors (
    candidate_id TEXT PRIMARY KEY,           -- shared with rejections.jsonl
    turn         INTEGER NOT NULL,
    candidate    TEXT NOT NULL,              -- verbatim candidate text
    reason       TEXT,                       -- agent-supplied, optional
    at           TEXT NOT NULL,              -- ISO-8601, mirrors jsonl
    embedding    BLOB NOT NULL               -- f32 vector
);
CREATE INDEX idx_rejection_vectors_turn ON rejection_vectors (turn);

-- sync state (ch. 02); derived
CREATE TABLE file_hashes (
    file_path   TEXT PRIMARY KEY,
    hash        TEXT NOT NULL,
    indexed_at  INTEGER NOT NULL
);
```

Queue reads order by `(enqueue_seq, id)`. The sequence is local SQLite
mechanics, never an external identifier or provenance field. Opening a
database with the original id-ordered queue schema transactionally
rebuilds that ephemeral table and copies pending rows in legacy `rowid`
insertion order; deleting the database instead remains equally safe.

`rejection_vectors` is rebuilt at startup from
`witness/rejections.jsonl` if the table is empty (or a dim probe
against the first stored row disagrees with a fresh embed, indicating
the embedding model changed) — the jsonl is authoritative, the table
is a cache the witness's σ retrieval reads.

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
- **Single writer per file.** The turn loop task writes the turn
  record; the agent task writes moment files (via the `create_moment`
  tool); the witness writes moves, its own receipt logs
  (`glean-log.jsonl`, `connect-log.jsonl`), and the derived
  `rejection_vectors` rows; adapter inbound writes channel logs (one
  writer task per file). The memory system alone writes the other
  database tables. The witness's `[connect]` frames route through the
  turn loop via mpsc so the turn-record's single-writer invariant is
  preserved.
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

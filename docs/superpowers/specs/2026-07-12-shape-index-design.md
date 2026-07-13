# Shape index — design

**Date:** 2026-07-12
**Status:** approved, ready for implementation plan
**Follows:** `docs/explorations/2026-07-10-weaving-shape-typed-flashes.md` (Fable, §2)
**Extends:** `docs/superpowers/specs/2026-07-11-flash-subsystem-design.md` (Spec 1: Bridge stubbed)
**Depends on:** the forthcoming `write_atomic` tool spec for the birth-time gloss trigger. Until that lands, the background worker in §4 is the sole source of shapes for new atomics.

## Purpose

A second embedding namespace indexing what an atomic note *does*
rather than what it is *about*. The witness composes a one-line
"logical skeleton" for each atomic; those glosses are embedded and
stored in a derived `shape_vectors` table. The divergence between
shape-space and text-space is the **Bridge signal** — same move,
different vocabulary — which the flash subsystem (Spec 1) has already
declared and stubbed. This spec makes Bridge real.

The shape index also becomes reusable substrate: the future weaving
practice's type-targeted `same-pattern-as` pass queries shape space,
and this spec exposes the retrieval seam it will use.

## Non-goals

- **Glossing anything but atomic notes.** `loom/` and
  `record/moments/` are indexed for text-space search but not for
  shape. Glosses are about *claims*; narrative and compressions of
  experience are not that shape. Revisit if evidence says otherwise.
- **Multi-shape per note.** Scalar only. If a note makes two moves,
  ship one and revisit. The schema is upgradeable.
- **Weaving.** Spec 4 territory. Bridge here fires from the per-turn
  flash pass; a weaving-time Bridge pass is a later addition that
  reuses the same retrieval seam.
- **Stance scan / Friction.** Spec 3 territory.
- **Agent-facing tools.** No `search_shapes` tool for the agent in
  this spec. Shape space is a witness-side signal for now.
- **The `write_atomic` tool itself.** That tool has its own spec
  (deferred). This spec defines the seam it will call and works
  without it via the background worker.

## Data model — `shape_vectors`

New table in the memory database, sibling to `segments` and
`rejection_vectors` (ch. 02):

```sql
CREATE TABLE IF NOT EXISTS shape_vectors (
    note_id      TEXT PRIMARY KEY,   -- ULID from atomic frontmatter
    file_path    TEXT NOT NULL,      -- for orphan cleanup on rename/delete
    gloss        TEXT NOT NULL,      -- the one-line skeleton
    author       TEXT NOT NULL,      -- 'witness' | 'agent'
    model_id     TEXT NOT NULL,      -- witness model at gloss time; 'agent' if authored
    prompt_hash  TEXT NOT NULL,      -- sha256 of on-shape.md at gloss time; empty for agent
    embedding    BLOB NOT NULL,      -- gloss embedding
    at           TEXT NOT NULL       -- RFC3339 write time
);
CREATE INDEX IF NOT EXISTS idx_shape_vectors_file ON shape_vectors (file_path);
```

**Ground-truth discipline** (ch. 02). The table is derived. Deleting
it costs shapes, never knowledge. The sync service already tracks
`knowledge/` file lifecycle; on delete of a file, its shape row is
removed alongside its segments. On rename (same id, new path), the
`file_path` column updates; the gloss survives.

**Agent override.** An atomic's frontmatter may carry
`shape: "…"`. When present at sync time, that string is embedded and
upserted as an `author='agent'` row (`model_id='agent'`,
`prompt_hash=''`). The agent may always claim authorship of her own
skeleton; the drift-repair worker (§4) never touches agent-authored
rows.

## The duty — `on-shape.md` and the gloss call

Following the wall's convention (ch. 04):
`workspace/witness/on-shape.md` is optional. Missing file → duty
disabled → no witness glossing happens → the shape column stays
empty for witness-owned rows → Bridge stays silent. Agent-authored
rows still populate; the duty file only gates the witness's path.

**Prompt** (seeds ship in `seed/witness/on-shape.md`, verbatim from
exploration §2):

> State the logical skeleton of this claim in one line of 8–20
> words. Use only abstract roles: a system, a signal, a measure, an
> observer, a part, a whole, a constraint, a boundary, a cost. **Do
> not use any domain noun that appears in the note.** Name the move,
> not the subject: what gets mistaken for what, what produces what,
> what fails when what changes, what survives what.
>
> Note body:
> `{note_body}`

Single template variable, single line out.

**Prompt hash** is `sha256(file_contents_bytes)` computed at read
time. The witness caches the (contents, hash) pair; the hash is
recomputed only when the file's mtime changes.

**The gloss call.** New module `crates/river-gateway/src/shape.rs`:

```rust
pub async fn gloss_note(
    witness: &Witness,
    memory: &Memory,
    note_id: &str,
    note_path: &Path,
    body: &str,
) -> anyhow::Result<()>
```

Reads `on-shape.md` (returns `Ok(())` early if missing), calls the
witness model with `{note_body}` substituted, embeds the returned
line via `memory.embedder`, upserts the `shape_vectors` row with
current `(model_id, prompt_hash, author='witness')`. Idempotent by
primary key.

**Call sites**, in priority order:

1. **`write_atomic` tool** (separate spec): after the file is
   durably written and its ULID is known, submits a job to the shape
   worker's queue. Birth-time; the intentional path.
2. **Shape worker** (§4): fills missing / stale rows.
3. **Agent override at sync time**: the sync service reads
   frontmatter for every changed atomic; if `shape:` is present, the
   value is embedded and upserted with `author='agent'`, bypassing
   the model call entirely.

The witness never writes to `knowledge/`. Witness-authored glosses
live only in the derived table.

## The shape worker

A single background task in the gateway drains a FIFO queue of
`(note_id, note_path, reason)` gloss jobs. Small on purpose — the
point is patience, not throughput.

**Job sources**, three of them:

1. **Missing rows.** On startup, walk `knowledge/`, enqueue any
   atomic whose `id` has no `shape_vectors` row. This is the backfill
   campaign for the current 64 atomics.
2. **Drift.** On startup, enqueue any row where
   `(model_id, prompt_hash) ≠ (current_witness_model_id, current_prompt_hash)`.
   Agent-authored rows (`author='agent'`) are exempt.
3. **Live.** `write_atomic` (when it lands) submits directly. Same
   queue, same worker.

**Rate.** Fires only when the turn loop is idle — the same 5-minute
quiet window digestion uses (ch. 02). One gloss per tick, then yields.
Inbound messages preempt instantly; the queue is patient. A restart
mid-campaign is fine — sources 1 and 2 re-enqueue on next startup.

**Cost.** 64 atomics × one cheap-model call ≈ pennies and seconds.
Bridge tolerates missing shapes: a candidate without a shape row
simply doesn't participate in Bridge (Connection/Echo/Return still
work for it via text-space).

**Receipts.** `workspace/witness/shape-log.jsonl`, one line per
gloss:

```json
{"note_id":"...", "author":"witness", "model_id":"...",
 "prompt_hash":"...", "gloss":"...",
 "reason":"missing|drift|write", "at":"..."}
```

Torn-line tolerance matches other JSONL logs (malformed lines skip
with a warning, valid lines load).

## Bridge wiring

Bridge lives in `crates/river-gateway/src/flashes.rs::types::bridge`
(the stub from Spec 1). Its wiring differs from the other flash
types because it cannot reuse the text-sim candidate pool — Bridge
is by definition text-sim *low*.

**Per settled turn, when `flash.types.bridge.enabled`**:

1. **Turn shape.** Call `shape::gloss_turn(witness, transcript)` —
   one extra witness call, using the same `on-shape.md` file with
   the note-body substitution filled by the turn transcript. Returns
   a one-line skeleton for the turn.
2. **Embed** the turn's shape gloss via `memory.embedder`. This
   embedding is Bridge-owned; other flash types don't use it.
3. **Search** `shape_vectors` for top-K shape neighbors via
   `Memory::search_shapes(vec, k)` (new helper). Default `k = 5`,
   its own knob so it doesn't compete with the flash pass's text-sim
   `top_k`. No warmth bump — witness-side retrieval, same discipline
   as `search_no_bump`.
4. **Text-sim filter.** For each shape candidate, look up its
   `segments` and compute cosine against the turn's transcript
   embedding (already computed by the flash pass). Keep only
   candidates where `text_sim ≤ bridge.text_sim_max`.
5. **Predicate on survivors.** `shape_sim ≥ bridge.shape_sim_min ∧
   text_sim ≤ bridge.text_sim_max`. First survivor above threshold
   fires; per-target refractory (same shape as Echo/Return).

**Frame body** (fixed template, Spec 1 convention):

```
[flash: bridge] turn N and [[<target_ref>]] make the same move in different words.

  shape: <turn shape gloss>
  matches: <target shape gloss>

<target body>
```

Both glosses in the frame make the signal legible — the agent can
see *why* Bridge fired.

**Missing signal is silent.** No `on-shape.md`, no witness gloss for
the turn, empty `shape_vectors`, or a candidate lacking a shape row
→ Bridge silently returns `None`. Same tolerance as any other
missing signal.

## Config surface

New per-agent block in `river.json`, sibling to `flash`:

```json
"shape": {
  "enabled": true,
  "worker_idle_seconds": 300
}
```

Two knobs: on/off, and the quiet threshold the worker uses (matches
the digestion cycle's default). Missing block → shape subsystem
entirely disabled; `shape_vectors` stays empty; Bridge stays silent;
contract preserved.

Bridge's own knobs extend the `flash.types` block from Spec 1:

```json
"bridge": {
  "enabled": true,
  "shape_sim_min": 0.70,
  "text_sim_max": 0.40,
  "top_k": 5,
  "min_new_turns_target": 20
}
```

Defaults from exploration §8. Validation: thresholds in
`[0.0, 1.0]`, `top_k > 0`, `min_new_turns_target ≥ 0`.

Config parsing lives in `river-core::config::ShapeConfig` and
`BridgeConfig`. `WitnessConfig::shape: Option<ShapeConfig>`; `None`
disables the subsystem.

## Data and receipts summary

- **New SQLite table:** `shape_vectors` (schema above).
- **New JSONL log:** `workspace/witness/shape-log.jsonl` (one line
  per gloss).
- **New optional prompt file:** `workspace/witness/on-shape.md`
  (missing → duty disabled).

No other file/table changes.

## Code layout

New module `crates/river-gateway/src/shape.rs`:

```
shape.rs
├── pub async fn gloss_note(...)            // upserts a shape row from a note body
├── pub async fn gloss_turn(...)            // one-shot gloss of a transcript for Bridge
├── pub async fn run_worker(...)            // background task; drains the queue on idle
├── pub struct GlossJob { note_id, path, reason }
├── pub struct ShapeLogEntry
└── mod prompt { load, hash, substitute }   // on-shape.md loader with mtime-cached hash
```

Changes to existing files:

- **`memory.rs`.** New `search_shapes(vec, k) -> Vec<(String, f32)>`
  helper (returns `(note_id, cosine)` pairs). New `upsert_shape` and
  `read_shape` helpers used by `shape.rs`. Sync service extended to
  read `shape:` frontmatter on atomic changes and call
  `upsert_shape` with `author='agent'`; also to `DELETE FROM
  shape_vectors WHERE file_path = ?` on atomic deletion (same shape
  as segments cleanup).
- **`witness.rs`.** `Witness` gains `shape: Option<ShapeState>`
  holding worker handle and prompt-hash cache. Startup enqueues the
  missing-rows and drift-repair scans.
- **`flashes.rs`.** The `signals::shape_sim` and `types::bridge`
  stubs from Spec 1 become real. Bridge does its own retrieval via
  `Memory::search_shapes`, not the shared candidate pool.
- **`main.rs`.** Wire the shape worker task into the same
  lifecycle-owned shutdown supervisor coordinated-shutdown work
  established for the witness and memory sync.

Wall docs updated in this spec:

- **`docs/wall/02-memory.md`.** New paragraph after the
  `rejection_vectors` paragraph naming `shape_vectors` as a third
  derived table: witness-authored glosses of atomic notes, source
  for Bridge, disposable and rebuildable, agent-override respected.
- **`docs/wall/04-witness.md`.** Fourth duty: **shape**. Same
  optional-prompt-file convention (`on-shape.md` missing → disabled).
  One-line description; details point to this spec.
- **Contracts block, ch. 04.** One line: *"The witness may author
  glosses of atomic notes into the derived `shape_vectors` table;
  it never writes to `knowledge/`. Agent-authored `shape:`
  frontmatter always overrides the witness's gloss."*
- **Contracts block, ch. 02.** One line: *"Shapes are derived;
  deleting `shape_vectors` costs shapes, never knowledge. The
  worker rebuilds it from `knowledge/` glosses."*

## Prompt files

- **`seed/witness/on-shape.md`** — ships the exploration §2 prompt
  verbatim, single `{note_body}` variable.
- No other prompts. Bridge frames use a fixed template.

## Testing

Unit tests inside `shape.rs`:

- **Prompt loader.** Missing file → duty disabled sentinel. Present
  file → substitution works. Hash stable across reads; recomputed
  after mtime change.
- **`gloss_note`.** Stub witness model returns a fixed line; stub
  embedder returns a fixed vector; assert the row upserts with
  correct `(model_id, prompt_hash, author='witness')`. Second call
  with same inputs is a no-op (idempotent by primary key). Agent
  override path bypasses the model call and stores `author='agent'`.
- **Worker.** Synthetic queue with mixed missing/drift/write jobs;
  assert idle-only firing (given a fake idle signal), preemption on
  a fake inbound event, and per-tick rate limit.
- **Drift detection.** Rows with mismatched `model_id` /
  `prompt_hash` enqueue on startup; agent-authored rows exempt.
- **Rename & delete.** Sync service update path covers both; shape
  row's `file_path` updates on rename; row deleted on file removal.

Unit tests inside `flashes.rs`:

- **Bridge predicate.** Table-driven: shape-sim above threshold,
  text-sim below max → fires. Text-sim above max → no fire.
  Shape-sim below min → no fire. Missing shape row for candidate →
  skipped. Refractory hit on repeat target.
- **Bridge tolerance.** Empty `shape_vectors`, missing
  `on-shape.md`, `gloss_turn` returns error → Bridge returns `None`
  silently, no error propagated to the flash pass.

Integration test at the witness level (stubbed embedder + stubbed
model client): one turn with a knowledge atomic whose shape matches
the turn's shape and whose text does not; assert a
`[flash: bridge] ...` system-role line lands on the record with
both glosses in the frame body.

## Rollout

One deploy step: rebuild, restart.

On startup:

1. `shape_vectors` table is created if absent (schema migration is a
   plain `CREATE TABLE IF NOT EXISTS`).
2. Shape worker scans `knowledge/` and enqueues missing rows (the
   64-atomic backfill for iris).
3. Shape worker scans `shape_vectors` and enqueues drift rows (empty
   set on first deploy).
4. Worker drains at idle; Bridge starts firing as the shape corpus
   populates.
5. `river.json` gains a `shape` block and a `flash.types.bridge`
   block. Missing `shape` block leaves everything off; missing
   `bridge` block defaults Bridge on with §5 defaults.

Iris's live gateway is the only affected workspace; seed configs
gain the default `shape` block and the default `bridge` sub-block.

## Contracts

- **Shapes are derived.** `shape_vectors` is disposable; delete it
  and the worker rebuilds it from `knowledge/`. Ground truth for a
  gloss is either the witness's model output (`author='witness'`)
  or the note's `shape:` frontmatter (`author='agent'`).
- **Divided authorship preserved.** The witness never writes to
  `knowledge/`. Witness-authored glosses live only in the derived
  table. Agent-authored `shape:` frontmatter values are respected
  and never overwritten by the drift-repair worker.
- **Missing signal is silent.** A missing `on-shape.md`, an empty
  `shape_vectors`, a candidate lacking a shape row, or a failed
  `gloss_turn` call → Bridge silently skips. No error, no frame.
- **Backfill is background.** The worker runs on the idle window;
  inbound messages preempt. Startup never blocks on backfill or
  drift repair.
- **Drift repair is bounded.** Rows with mismatched
  `(model_id, prompt_hash)` re-enter the queue at startup only.
  Agent-authored rows are exempt. Existing note glosses are never
  re-glossed inside the turn loop; Bridge's per-turn `gloss_turn`
  call is a separate operation over the transcript, not the
  `shape_vectors` table.
- **Bridge owns its retrieval.** Bridge does not consume the flash
  pass's shared text-sim candidate pool; it retrieves from
  `shape_vectors` directly and text-sim filters afterward.
- **Shape prompt is optional.** Missing `on-shape.md` disables the
  witness's shape authorship entirely; agent-authored rows still
  populate and Bridge still fires on those.

## Open questions

- **Turn-shape cost.** Bridge currently costs one extra witness call
  per settled turn when enabled. If measurement shows this
  meaningful, guard behind "flash pass produced any candidates at
  all" or a text-sim floor. Ship without the guard; measure.
- **Multi-shape per note.** Deferred. Revisit if the shape log
  shows notes that need two skeletons.
- **Weaving reuse.** Spec 4's type-targeted `same-pattern-as` pass
  will reuse `Memory::search_shapes`. This spec exposes the seam;
  weaving will refine what "target selection by shape" looks like
  when it lands.
- **Prompt versioning granularity.** `prompt_hash` is
  `sha256(file_bytes)` — whitespace-sensitive. A no-op reformatting
  of `on-shape.md` will trigger drift repair. Acceptable for now
  (cheap and rare); revisit if the operator wants
  semantically-versioned prompts.

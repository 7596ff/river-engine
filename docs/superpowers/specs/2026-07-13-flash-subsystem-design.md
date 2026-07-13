# Flash subsystem — design (v2)

**Date:** 2026-07-13
**Status:** approved, ready for implementation plan
**Supersedes:** `docs/superpowers/specs/2026-07-11-flash-subsystem-design.md`
**Follows:** `docs/explorations/2026-07-10-weaving-shape-typed-flashes.md` (Fable, §1)
**Absorbs:** the current connect duty (`witness.rs::connect_for`,
`ConnectFrame`, `witness/on-connect.md`, `witness/connect-log.jsonl`).
**Uses:** the shape substrate shipped in
`docs/superpowers/specs/2026-07-12-shape-index-design.md`
(`Memory::search_shapes`, `shape::gloss_turn`).

## Why v2

The 2026-07-11 spec was approved but unimplemented while shape shipped
around it. Three drifts made a rewrite cleaner than an amendment:

1. **Design drift.** Shape's worker + sync-service seam patterns
   proved out; Bridge's substrate is fully ready; σ-retrieval has
   been running in production and its integration is stable.
2. **Some choices were wrong.** The old spec's blanket "at most one
   flash per type per turn" cap read as principled but was arbitrary
   noise-control that fought the design. The digestion + heartbeat
   exclusions inherited from connect ("these turns don't carry
   substance") are wrong — digestion turns hold the candidate text
   and heartbeat turns hold whatever the agent did with unprompted
   time. Both deserve flashes.
3. **Scope was too wide.** Correction's agent-facing frame duplicates
   signal that σ-retrieval already gives the witness; whether that's
   worth the noise is a real design question that deserves its own
   spec. Danger needs a live rejection stream to design cluster
   identity against; shipping the first-cut heuristic in this bundle
   would freeze a bad choice.

## Purpose

The witness gains a **flash pass** on each settled turn — a family of
predicates over a small feature space of signals. The existing
connect duty becomes one type (Connection) in that family. Four types
ship real (Connection, Echo, Return, Bridge); Correction is stubbed
in config; Danger is out of scope.

The flash pass shares the connect path's dispatch shape: witness
computes → `FlashFrame` via mpsc → turn loop appends
`[flash: <type>] ...` as a system-role line to `record/turns.jsonl`.
Same single-writer invariant, same audience-of-one visibility. No
memory-slot ambient in v1; tone in the frame body distinguishes.

## Non-goals

- **Correction, live.** Config accepts `flash.types.correction`;
  the module returns empty. Its own follow-up decides whether the
  agent-facing frame is worth adding to the σ-retrieval that already
  serves the witness.
- **Danger.** Out of scope entirely. No config field, no stub.
- **Memory-slot ambient dispatch.** All flashes are system-role
  lines on the record. If in-flight use shows that's too loud, the
  memory-slot seam (`context.set_memory_slot(...)`, already used by
  knowledge flashes) is the natural next step.
- **New embedding namespaces.** Bridge reuses `shape_vectors`
  (already shipped).
- **A witness call per flash.** Only Connection composes a why via
  the witness model (as connect does today); the other three types
  use fixed frame templates.
- **Removing `WitnessConfig::similar_rejections_*` fields.** Those
  serve on-glean, not the flash pass; they stay.

## The flash pass

Per settled turn, `flashes::flash_pass(&mut Witness, turn)` runs
inline in `witness.rs::run()` where `connect_for` runs today. The
witness holds `flashes: Option<flashes::State>`; `None` when the
`flash` block is absent from config or the block is disabled.

Steps:

1. Read the settled turn's lines from `record/turns.jsonl` (same
   `record::scan_turn_range` call connect uses today).
2. Format the transcript (same `format_transcript` used by connect
   and glean).
3. **Shared candidate pool** for the text-sim types (Connection,
   Echo, Return): `memory.search_no_bump(transcript, flash.top_k)`.
   Default `top_k = 5`. One embed + one scan for the whole pass.
4. **Bridge** does its own retrieval if enabled:
   `shape::gloss_turn(client, prompt, system, transcript)` → embed
   the returned gloss via `memory.embedder` → `memory.search_shapes(
   vec, flash.types.bridge.top_k)`. Missing shape substrate (no
   `on-shape.md`, empty `shape_vectors`, `gloss_turn` errors) →
   Bridge silently skips. This matches the shape spec's "missing
   signal is silent" contract.
5. For each enabled type in a fixed order (Connection, Echo, Return,
   Bridge), evaluate its predicate over its candidate pool. Iterate
   candidates in score order. Every candidate that clears the
   predicate + guards emits a `FlashFrame`.
6. Each `FlashFrame` is sent via `flash_tx`. Send failure logs and
   drops (best-effort, same as connect).
7. After all frames send, append one `FlashLogEntry` per fired flash
   to `witness/flashes.jsonl`, fsync per line. Refractory state
   updates from the successfully-appended entries.

**No turn-type exclusions.** Heartbeat, digestion, and channel turns
all fire flashes when their transcripts produce qualifying
candidates. The connect era's exclusions retire with connect.

**No arbitrary caps.** Multiple flashes per type per turn are fine —
each qualifying candidate fires. Per-target refractory
(Echo/Return/Bridge) prevents flashing the same target repeatedly
across turns.

## Per-type predicates

Each type is a small module inside `flashes::types` with a `Config`
struct, a predicate function, a fixed frame template (Connection
excepted), and per-target refractory state loaded from `flashes.jsonl`
at startup.

### Connection

**Absorbs the current connect duty.** Predicate: `text_sim ≥
threshold`; self-write guard skips candidates whose file was written
by the agent within the last `self_write_window` turns.

No per-target refractory — Connection is about the freshest hit, not
repeats. (Same behavior as connect today.)

**Composition.** Calls the witness model with
`witness/flashes/on-connection.md` (moved from `witness/on-connect.md`,
contents unchanged), substituting `{transcript}`, `{target_path}`,
`{target_excerpt}`. The composed why becomes the frame body:

```
[flash: connection] turn N connects to [[<target_ref>]]: <why>

<target body — full text if atomic note, else first 200 words>
```

Model returning empty or the literal `NOTHING_TO_CONNECT` sentinel →
skip (same as connect today).

**Config defaults**: `enabled: true`, `threshold: 0.65`,
`self_write_window: 5`.

### Echo

Predicate: `text_sim ≥ threshold ∧ warmth(target) ≥ warmth_min` —
the target is currently warm.

**Per-target refractory**: `min_new_turns_target` — the same target
won't echo twice within that many turns.

Fixed template:

```
[flash: echo] turn N echoes [[<target_ref>]] — you were thinking this recently.

<target body>
```

**Config defaults**: `enabled: true`, `threshold: 0.55`,
`warmth_min: 0.3`, `min_new_turns_target: 20`.

### Return

Predicate: `text_sim ≥ threshold ∧ staleness_turns(target) ≥
gap_min_turns` — the target has been cold for a while.

Per-target refractory (same shape as Echo).

Fixed template:

```
[flash: return] turn N returns to [[<target_ref>]] — you haven't thought this in <gap> turns.

<target body>
```

**Config defaults**: `enabled: true`, `threshold: 0.55`,
`gap_min_turns: 200`, `min_new_turns_target: 20`.

### Bridge

**Uses the shape substrate.** Own candidate pool via
`Memory::search_shapes(gloss_vec, flash.types.bridge.top_k)`.

Predicate: `shape_sim ≥ shape_sim_min ∧ text_sim ≤ text_sim_max`.
For each shape candidate, look up its `segments` and compute cosine
against the turn's transcript embedding (already computed by the
flash pass for the shared pool) — the same in-memory data drives
both checks.

Per-target refractory.

Fixed template:

```
[flash: bridge] turn N and [[<target_ref>]] make the same move in different words.

  shape: <turn shape gloss>
  matches: <target shape gloss>

<target body>
```

Both glosses in the frame make the signal legible — the agent can
see *why* Bridge fired and whether it's real.

**Missing signal is silent.** No `on-shape.md`, empty
`shape_vectors`, no shape row for a candidate, or a failed
`gloss_turn` → the type contributes zero frames, no error surfaces
to the flash pass.

**Config defaults** (from shape spec §5): `enabled: true`,
`shape_sim_min: 0.70`, `text_sim_max: 0.40`, `top_k: 5`,
`min_new_turns_target: 20`.

### Correction (stubbed)

`flash.types.correction { enabled: false, threshold: 0.60 }` is
accepted by the config schema so we don't have to bump config later.
The predicate module exists and returns an empty vec. A follow-up
spec decides whether the agent-facing frame is worth adding to the
σ-retrieval that already serves the witness — the false-positive
rate is the real question, and it's answerable only with live data.

## Signals

Pure functions in `flashes::signals`, each takes ambient state and
returns a number:

| signal | source | status |
|---|---|---|
| `text_sim(hit)` | cosine over `segments` (already computed by `search_no_bump` and `search_shapes`) | shipped |
| `warmth(target)` | `activation` table row lookup | shipped |
| `staleness_turns(target, now_turn)` | `now_turn - activation.last_touched_turn` | shipped |
| `shape_sim(hit)` | cosine over `shape_vectors` (already computed by `search_shapes`) | shipped |

No new signals in v1. Rejection-cluster rate (for Danger) and stance
verdict (for Friction) are deferred to their own specs.

## Config surface

New per-agent block in `river.json`:

```json
"flash": {
  "top_k": 5,
  "types": {
    "connection": {
      "enabled": true,
      "threshold": 0.65,
      "self_write_window": 5
    },
    "echo": {
      "enabled": true,
      "threshold": 0.55,
      "warmth_min": 0.3,
      "min_new_turns_target": 20
    },
    "return": {
      "enabled": true,
      "threshold": 0.55,
      "gap_min_turns": 200,
      "min_new_turns_target": 20
    },
    "bridge": {
      "enabled": true,
      "shape_sim_min": 0.70,
      "text_sim_max": 0.40,
      "top_k": 5,
      "min_new_turns_target": 20
    },
    "correction": {
      "enabled": false,
      "threshold": 0.60
    }
  }
}
```

Validation: thresholds in `[0.0, 1.0]`; counts positive. Missing
type block → type-level defaults. Missing `flash` block → subsystem
entirely off (fail-safe for un-migrated configs).

**Removed fields on `WitnessConfig`**: `connect_threshold`,
`connect_min_new_turns`, `connect_self_write_window`. Any config
that still names them fails to parse loudly with an error naming
each removed field and pointing at `flash.types.connection`.

**Preserved fields on `WitnessConfig`**: `similar_rejections_top_k`
and `similar_rejections_threshold`. Those serve the on-glean prompt's
`{similar_rejections}` block; they are not flash-pass concerns.

## Data and receipts

**Receipt log**: `witness/flashes.jsonl` (renamed from
`witness/connect-log.jsonl`). One entry per fired flash:

```json
{
  "type": "connection|echo|return|bridge",
  "turn": <u64>,
  "channel": "<channel>",
  "target_ref": "<wikilink>",
  "target_path": "<workspace-relative>",
  "score": <f32>,
  "at": "<RFC3339>"
}
```

Bridge extras: `"turn_shape": "<gloss>"`, `"target_shape": "<gloss>"`
so the log records what the shape signal was matching (debuggable
by hand).

Torn-line tolerance: malformed lines skip with a warning; valid
lines load. Same discipline as `glean-log.jsonl`, `shape-log.jsonl`,
`connect-log.jsonl`.

Startup recovery scans a bounded tail (last 1000 entries) and
rebuilds per-type refractory state: per-target last-fire turn map
for Echo, Return, and Bridge. Connection has no refractory to
rebuild (deliberately).

**Legacy `connect-log.jsonl` migration**. On startup, if
`witness/flashes.jsonl` does not exist but `witness/connect-log.jsonl`
does, the module reads the old file, writes each entry to
`flashes.jsonl` with `"type": "connection"` and any missing fields
defaulted, then removes the old file. Idempotent — re-runs find
`flashes.jsonl` already present and skip.

**No new SQLite tables.** Signals draw from existing tables:
`segments` (text_sim), `activation` (warmth, staleness_turns),
`shape_vectors` (shape_sim).

## Code layout

New module `crates/river-gateway/src/flashes.rs`:

```
flashes.rs
├── pub struct FlashFrame { turn, channel, flash_type, target_ref,
│                           target_path, body,
│                           bridge_extras: Option<BridgeExtras> }
├── pub enum FlashType    // Connection | Echo | Return | Bridge | Correction
├── pub struct FlashLogEntry
├── pub struct BridgeExtras { turn_shape, target_shape }
├── pub struct State      // per-target refractory + config
├── pub async fn flash_pass(witness: &mut Witness, turn: u64)
│                         -> anyhow::Result<()>
├── mod signals { text_sim, warmth, staleness_turns, shape_sim }
├── mod types   { connection, echo, return_, bridge, correction }
└── mod log     { load_and_recover, append, migrate_legacy_connect_log }
```

Changes to existing files:

- **`witness.rs`**. Seven connect fields collapse into
  `flashes: Option<flashes::State>`. `connect_for` deletes;
  `flash_pass` replaces it in `run()`. `NOTHING_TO_CONNECT`,
  `CONNECT_SCAN_K`, `recover_last_connect_through`,
  `append_connect_log`, `ConnectLogEntry`, `WitnessBuilder::with_connect`
  all delete (their generalizations live in `flashes`).
  `WitnessBuilder::with_flashes(sender, flash_config)` replaces
  `with_connect`.
- **`turn.rs`**. `ConnectFrame` → `FlashFrame` with a
  `flash_type: FlashType` field. `connect_frames` field →
  `flash_frames`. `drain_connect_frames` → `drain_flash_frames`.
  `build_connect_frame_body` → `build_flash_frame_body`, dispatches
  by type (Connection uses today's composed-why body shape; the
  others use their fixed templates).
- **`main.rs`**. `(connect_tx, connect_rx)` mpsc pair renames to
  `(flash_tx, flash_rx)`. `WitnessBuilder::with_connect(...)` →
  `with_flashes(sender, flash_config)`. `TurnLoop::new` takes
  `Some(flash_rx)`. The shape sender wiring from Phase 8 of the
  shape spec is preserved; Bridge reads from `memory` directly, no
  new pipes.

## Wall doc updates in this spec

- **`docs/wall/04-witness.md`**. The "Connecting" subsection
  generalizes to "Flashes" — one flash pass per settled turn, four
  types (Connection absorbs the current connect duty; Echo, Return,
  Bridge each get one paragraph). Correction is named as a stub.
  Danger is not mentioned.
- **`docs/wall/10-data.md`**. `witness/connect-log.jsonl` renames to
  `witness/flashes.jsonl`; new receipt schema documented; legacy
  file's one-time migration recorded.
- **Contracts block, ch. 04**. Contract lines listed in
  §Contracts below.

## Prompt files

- `workspace/witness/flashes/on-connection.md` — moved from
  `witness/on-connect.md`, contents unchanged. Only compose-why
  prompt shipped.
- No `on-echo.md`, `on-return.md`, `on-bridge.md`,
  `on-correction.md` in this spec — their frames use fixed
  templates.
- The convention is preserved: an `on-<type>.md` in
  `witness/flashes/` can be added later to override any type's
  fixed template with a witness-composed variant. Missing file =
  fixed template.

## Testing

Unit tests inside `flashes.rs`:

- **Signals module** (pure): table-driven tests for `text_sim`,
  `warmth`, `staleness_turns`, `shape_sim` given fake ambient
  state.
- **Type predicates**: one table-driven test per type. Threshold
  edges (just above, just below), refractory hits (Echo, Return,
  Bridge), self-write guard (Connection).
- **Multiple flashes per turn**: pool of 3 candidates all above
  Echo threshold → 3 frames emitted; per-target refractory only
  prevents re-fires on repeated targets across turns.
- **No turn-type exclusions**: heartbeat and digestion turns fire
  flashes when candidates qualify (contrast with connect's current
  behavior).
- **Bridge tolerance**: empty `shape_vectors`, missing
  `on-shape.md`, `gloss_turn` returns error → Bridge contributes
  zero frames silently. No error propagates to `flash_pass`.
- **Log recovery**: synthetic `flashes.jsonl` with mixed types →
  per-target refractory maps rebuild correctly.
- **Legacy migration**: synthetic `connect-log.jsonl` present
  without `flashes.jsonl` → renamed and populated with
  `type: "connection"`; re-run is a no-op.
- **Config parse**: a `river.json` with any `connect_*` field on
  `witness` fails loudly with an error naming the removed field.

Integration test at the witness level (stubbed embedder + stubbed
model client, mirroring today's connect integration): one turn
end-to-end. Asserts Connection fires when a note clears threshold,
Echo fires alongside if the target is warm, Bridge fires when the
shape substrate matches and text-sim is low — all three land as
`[flash: <type>]` system-role lines on the record via the mpsc
pipe.

## Rollout

One deploy step: rebuild, restart.

On startup:

1. `witness/connect-log.jsonl` → `witness/flashes.jsonl` migration
   runs (idempotent).
2. `witness/on-connect.md` → `witness/flashes/on-connection.md`
   rename. The deploy step does the file move; the module reads
   from the new path only.
3. `river.json` must have a `flash` block; the old `connect_*`
   fields are gone. Iris.json edits by hand in the same commit as
   the code — one workspace, five minutes.
4. Next settled turn fires flashes with the new prefix. Historical
   `[connect]` lines in the record stay untouched (agents read
   their own history and both prefixes are legible).

Iris's live gateway is the only affected workspace; seed configs
gain a default `flash` block matching the defaults in this spec.

## Contracts

- **Flash pass on every settled turn.** No exclusions —
  heartbeat, digestion, and channel turns all fire flashes when
  their transcripts produce qualifying candidates.
- **No arbitrary per-turn cap.** Each type may fire multiple times
  per turn (once per qualifying candidate). Per-target refractory
  (Echo, Return, Bridge) prevents flashing the same target
  repeatedly across turns.
- **Single-writer preserved.** All flashes route through the
  `flash_tx` mpsc pipe; the turn loop is the only writer of
  `turns.jsonl`.
- **Divided authorship preserved.** No flash writes to `knowledge/`;
  flash frames appear only as system-role record lines and receipts
  in `witness/flashes.jsonl`.
- **Missing signal is silent.** Bridge without `shape_vectors`,
  without `on-shape.md`, or with a failed `gloss_turn` returns zero
  frames silently. No error surfaces to the flash pass.
- **Fail-safe config.** Missing `flash` block disables the
  subsystem entirely — an un-migrated agent runs with no flashes
  rather than crashing.
- **Loud migration.** Any config still naming the removed
  `connect_*` fields fails at parse with an error identifying the
  fields and pointing at `flash.types.connection`.
- **Refractory is type-owned.** Each type's refractory state is
  independent; state loads from the tail of `flashes.jsonl` at
  startup. Connection has no refractory by design.

## Open questions

- **Correction agent-facing frame.** Deferred to its own spec.
  The follow-up must argue whether the frame is worth the noise
  given σ-retrieval already gives the witness this signal, and
  should decide with real data (rejection rates from a live
  workspace) rather than upfront.
- **Danger.** Deferred entirely. Needs a live rejection stream
  to design cluster-identity heuristics against; freezing a
  first-cut in this bundle would set the wrong precedent.
- **Composed-why for Echo/Return/Bridge.** Fixed templates ship
  in v1; if in-flight use shows the fixed language feels hollow,
  optional `on-<type>.md` prompt overrides land in a small
  follow-up (the convention is already in place).
- **Memory-slot ambient dispatch.** All types fire as system-role
  lines in v1. If the always-on-record volume becomes noisy,
  Fable's memory-slot mechanism (already used by knowledge
  flashes via `context.set_memory_slot`) is the natural next
  step for the four "ambient" types (Echo, Return, Bridge,
  Correction if it lands).

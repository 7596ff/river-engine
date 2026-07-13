# Flash subsystem — design

**Date:** 2026-07-11
**Status:** SUPERSEDED by `docs/superpowers/specs/2026-07-13-flash-subsystem-design.md` (v2)
**Follows:** `docs/explorations/2026-07-10-weaving-shape-typed-flashes.md` (Fable, §1)
**Absorbs:** the current connect duty (`witness.rs::connect_for`,
`ConnectFrame`, `connect-log.jsonl`, `on-connect.md`)

> **Superseded.** This spec was approved but not implemented while the
> shape spec (2026-07-12) shipped around it. The v2 rewrite at
> `docs/superpowers/specs/2026-07-13-flash-subsystem-design.md` is
> smaller (four working types + one stub, no Danger), removes
> exclusions and per-turn caps that turned out to be wrong, and
> integrates against the shape substrate that now exists. Read v2
> instead; v1 is preserved for provenance.

## Purpose

The witness gains a third duty in generalized form: a **flash pass** on
each settled turn. Where connect answers one question ("does anything
in the workspace connect to this turn?"), the flash pass answers a
family of them, each a predicate over a small feature space of
signals. The existing connect duty becomes one type of flash
(Connection) in the family.

Five types ship in this spec, all buildable from signals that already
exist in the harness: Connection, Echo, Return, Correction, Danger.
Two types (Bridge, Friction) are declared in config but stubbed —
they light up when Spec 2 (shape index) and Spec 3 (stance scan)
land.

## Non-goals

- Bridge, Friction implementation (deferred to their signal specs).
- Memory-slot ambient dispatch. Fable's design distinguishes ambient
  types (memory slot) from interruptive types (system-role lines).
  Spec 1 ships everything as system-role lines; tone distinguishes.
  Memory-slot dispatch is a follow-up if system-role becomes too loud.
- New embedding namespaces. Shape gets its own table in Spec 2.
- Removal or rewrite of the existing `{similar_rejections}` slot in
  `on-glean.md` — the witness keeps its Bayesian prior over past
  rejections; Correction adds a parallel agent-facing surfacing.
- A witness model call for every flash. Only Connection composes a
  why via the witness model (as connect does today); the other four
  types use fixed frame templates.

## The flash pass

Per settled turn (same trigger connect uses today), the witness runs
one flash pass:

1. Skip if the turn is a digestion or heartbeat turn (same exclusions
   connect uses today).
2. Skip if there is no `flash` block in config or the block is
   disabled.
3. Embed the turn's transcript (single embed reused across types).
4. Retrieve top-K knowledge neighbors via
   `memory::search_no_bump(transcript, flash.top_k)` — the shared
   candidate pool. Default `top_k = 5`.
5. For each type enabled in config, evaluate its predicate:
   - Iterate candidates in score order.
   - Apply the type's guards (refractory, self-write, per-target
     bookkeeping).
   - Emit at most one flash per type per turn.
6. Additionally, evaluate **Danger** — a windowed rate over
   `rejection_vectors`, no retrieval — and emit at most one Danger
   frame per turn per cluster.
7. Each fired flash produces a `FlashFrame` sent via the mpsc pipe;
   the turn loop writes a `[flash: <type>] ...` system-role line on
   the turn and (if the turn is still in HOT) synthesizes the same
   line into the live context window.

Correction runs on a **different clock**: it fires on the glean pass,
per witness-proposed candidate, when the candidate's rejection-sim
against past rejections clears `flash.types.correction.threshold`.
Not part of the flash pass loop; hooks the existing σ-retrieval path
in `witness.rs`.

## Signals

Signals are pure functions in a `signals` submodule of `flashes.rs`.
Each takes the ambient state (memory handle, activation snapshot,
turn embedding, candidate) and returns a plain number.

| signal | source | status |
|---|---|---|
| `text_sim(candidate)` | cosine over `segments` embedding (already computed by `search_no_bump`) | exists |
| `warmth(target)` | `activation` table row lookup | exists |
| `staleness_turns(target, now_turn)` | `now_turn - activation.last_touched_turn` | exists |
| `rejection_sim(candidate)` | cosine over `rejection_vectors` (already computed by `top_similar_rejections`) | exists |
| `danger_clusters(window_hours, cluster_cosine, count)` | windowed scan over `rejection_vectors` at (now - window), grouped by cosine ≥ cluster_cosine, filtered to clusters of size ≥ count | new logic, existing table |
| `shape_sim(candidate)` | placeholder; unimplemented until Spec 2 | deferred |

## Types

Each type is a small module inside `flashes::types` with:

- A `Config` struct (deserialized from the type's block in
  `flash.types.<name>`).
- A predicate function `fire(candidate, signals, state, config) ->
  Option<FlashFrame>` — returns `Some` if the flash fires, `None`
  otherwise.
- A fixed template for the frame body (Connection excepted).
- Its own refractory state loaded from `flashes.jsonl` at startup.

### Connection

**Predicate.** `text_sim ≥ threshold`, self-write guard (skip
candidates matching a knowledge note the agent wrote in the last
`self_write_window` turns), refractory `min_new_turns` since the last
Connection fire.

**Composition.** Calls the witness model with `on-connection.md`
(renamed from `on-connect.md`), substituting `{transcript}`,
`{target_path}`, `{target_excerpt}`. The composed why becomes the
frame's body line.

**Frame body.** Same shape as today's connect frame:
```
[flash: connection] turn N connects to [[<target_ref>]]: <why>

<target body — full text if atomic note, else first 200 words>
```

**Config defaults.** `enabled: true`, `threshold: 0.65`,
`min_new_turns: 5`, `self_write_window: 5`.

### Echo

**Predicate.** `text_sim ≥ threshold ∧ warmth(target) ≥ warmth_min`
— the target is currently warm.

**Refractory.** Per-target: the same target won't echo twice within
`min_new_turns_target` turns.

**Frame body.** Fixed template:
```
[flash: echo] turn N echoes [[<target_ref>]] — you were thinking this recently.

<target body>
```

**Config defaults.** `enabled: true`, `threshold: 0.55`,
`warmth_min: 0.3`, `min_new_turns_target: 20`.

### Return

**Predicate.** `text_sim ≥ threshold ∧ staleness_turns(target) ≥
gap_min_turns` — the target has been cold for a while.

**Refractory.** Per-target, same shape as Echo.

**Frame body.** Fixed template:
```
[flash: return] turn N returns to [[<target_ref>]] — you haven't thought this in <gap> turns.

<target body>
```

**Config defaults.** `enabled: true`, `threshold: 0.55`,
`gap_min_turns: 200`, `min_new_turns_target: 20`.

### Correction

**Trigger.** Not the flash pass. Fires on the glean pass, per
witness-proposed candidate, alongside the existing σ-retrieval that
populates `{similar_rejections}`.

**Predicate.** `rejection_sim(candidate) ≥ threshold` for at least
one past rejection. The winning past rejection determines the frame
body.

**Refractory.** Per-candidate: the same `candidate_id` won't emit a
Correction more than once (already deduplicated in the σ-retrieval
path).

**Frame body.** Fixed template:
```
[flash: correction] turn N — you turned this away before (turn <past_turn>): <past_reason>

<candidate text>
```

**Config defaults.** `enabled: true`, `threshold: 0.60`.

The `{similar_rejections}` template substitution in `on-glean.md`
stays untouched — the witness still gets its Bayesian prior; the
flash adds a parallel agent-facing surfacing.

### Danger

**Trigger.** Runs during the flash pass but consults no candidate
pool. Scans `rejection_vectors` for the last `window_hours` and
clusters entries by cosine ≥ `cluster_cosine`. A cluster of size ≥
`count` fires one Danger frame.

**Refractory.** Per-cluster, `window_hours`: a cluster that fires
doesn't re-fire until the window slides off its earliest member.
Cluster identity is the set of `candidate_id`s; two clusters are
"the same" if they share ≥ ceil(count / 2) members.

**Frame body.** Fixed template:
```
[flash: danger] turn N — <count> rejections in <window> hours cluster around a shape you keep turning away:

  • <exemplar 1 candidate text, truncated to 120 chars>
  • <exemplar 2 candidate text, truncated to 120 chars>
  • <exemplar 3 candidate text, truncated to 120 chars>
```

No composition; the frame reports, does not adjudicate. If Correction
fires on the same material, both frames land — Correction on the
glean turn, Danger on its own trigger; the engine never suppresses
either in favor of the other.

**Config defaults.** `enabled: true`, `window_hours: 72`,
`cluster_cosine: 0.70`, `count: 4`.

### Bridge and Friction (stubbed)

`flash.types.bridge` and `flash.types.friction` blocks are accepted
by the config schema (so we don't have to bump config later) but
default `enabled: false`. Their predicate modules exist as stubs
returning `None`. Spec 2 (shape index) implements Bridge; Spec 3
(stance scan) implements Friction.

## Config surface

New per-agent block in `river.json`:

```json
"flash": {
  "top_k": 5,
  "types": {
    "connection": {
      "enabled": true, "threshold": 0.65,
      "min_new_turns": 5, "self_write_window": 5
    },
    "echo": {
      "enabled": true, "threshold": 0.55,
      "warmth_min": 0.3, "min_new_turns_target": 20
    },
    "return": {
      "enabled": true, "threshold": 0.55,
      "gap_min_turns": 200, "min_new_turns_target": 20
    },
    "correction": {
      "enabled": true, "threshold": 0.60
    },
    "danger": {
      "enabled": true, "window_hours": 72,
      "cluster_cosine": 0.70, "count": 4
    }
  }
}
```

Config parsing lives in `river-core::config::FlashConfig`. Validation:
thresholds in `[0.0, 1.0]`, counts positive, windows positive.
`WitnessConfig::flash: Option<FlashConfig>`; `None` disables the
subsystem entirely (fail-safe for un-migrated configs).

The current `WitnessConfig` fields are **removed** — no more
`on_connect`, `connect_threshold`, `connect_min_new_turns`,
`connect_self_write_window`, `similar_rejections_top_k`,
`similar_rejections_threshold`, `connect_log_path`, `connect_sender`.
Their behavior migrates into `flash.types.connection.*` and
`flash.types.correction.threshold`. An un-migrated `river.json`
fails to parse loudly.

Missing type block → type-level defaults (above). Missing `flash`
block → entire subsystem off.

## Data and receipts

Receipt log: `witness/flashes.jsonl` (renamed from
`witness/connect-log.jsonl`). One entry per fired flash:

```json
{
  "type": "connection|echo|return|correction|danger",
  "turn": <u64>,
  "channel": "<channel>",
  "target_ref": "<wikilink target, if applicable>",
  "target_path": "<workspace path, if applicable>",
  "score": <f32>,
  "at": "<RFC3339>"
}
```

Type-specific extra fields:

- **Danger**: `"cluster": { "size": N, "exemplar_candidate_ids": [...] }`
- **Correction**: `"rejection_ref": "<candidate_id of the past rejection>"`

Torn-line tolerance: same as existing JSONL logs — malformed lines
skip with a warning, valid lines load.

Startup recovery scans the tail (bounded, e.g., last 1000 entries)
and rebuilds per-type refractory state: last-fire turn for
Connection; per-target last-fire turn map for Echo and Return; per-
cluster last-fire window for Danger. Correction has no refractory
state to rebuild (deduplication is per-candidate at the moment of
σ-retrieval).

**No new SQLite tables.** All signals draw from existing tables
(`segments`, `activation`, `rejection_vectors`).

**Historical `connect-log.jsonl` migration.** On startup, if
`witness/flashes.jsonl` does not exist but `witness/connect-log.jsonl`
does, the module reads the old file, writes each entry to
`flashes.jsonl` with `"type": "connection"` and any other missing
fields defaulted (score = 0.0 if absent), then removes the old file.
One-time; idempotent (re-runs find `flashes.jsonl` already present
and skip).

## Code layout

New module `crates/river-gateway/src/flashes.rs`:

```
flashes.rs
├── pub struct FlashFrame            // replaces ConnectFrame in turn.rs
├── pub enum FlashType               // Connection | Echo | Return | Correction | Danger
├── pub struct FlashLogEntry         // replaces ConnectLogEntry
├── pub struct State                 // per-type refractory + config
├── pub async fn flash_pass(
│       witness: &mut Witness,
│       turn: u64,
│   ) -> anyhow::Result<()>          // called from witness.rs::run
├── pub fn on_glean_correction(
│       state: &mut State,
│       candidate: &Candidate,
│       past: &[SimilarRejection],
│   ) -> Option<FlashFrame>          // hooked into the glean σ path
├── mod signals { text_sim, warmth, staleness_turns,
│                 rejection_sim, danger_clusters }
├── mod types { connection, echo, return_, correction, danger }
└── mod log { load_and_recover, append, migrate_legacy_connect_log }
```

Changes to existing files:

- **`witness.rs`**. The seven connect fields collapse into
  `flashes: Option<flashes::State>`. `connect_for` deletes; the flash
  pass replaces it in `run()`. The glean σ-retrieval path calls
  `flashes::on_glean_correction` alongside `format_similar_rejections`.
  `NOTHING_TO_CONNECT`, `CONNECT_SCAN_K`, `recover_last_connect_through`,
  `append_connect_log` all delete (their generalizations live in
  `flashes::log`).
- **`turn.rs`**. `ConnectFrame` → `FlashFrame` with a
  `flash_type: FlashType` field. `connect_frames` field →
  `flash_frames`. `drain_connect_frames` → `drain_flash_frames`.
  `build_connect_frame_body` → `build_flash_frame_body`, which
  dispatches by type — Connection uses today's composed-why body
  shape; the other four use their fixed templates.
- **`main.rs`**. `(connect_tx, connect_rx)` mpsc pair renames to
  `(flash_tx, flash_rx)`. `WitnessBuilder::with_connect(...)` →
  `with_flashes(sender, flash_config)`. `TurnLoop::new` takes
  `Some(flash_rx)`.

Wall docs update in Spec 1:

- **`docs/wall/04-witness.md`**. The current "Connecting" subsection
  (duty three) generalizes to "Flashes" — one flash pass per settled
  turn, five types shipped, Bridge/Friction reserved. The Correction
  behavior across duties (witness prior + agent flash) is stated.
- **`docs/wall/10-data.md`**. `witness/connect-log.jsonl` renames to
  `witness/flashes.jsonl`; new receipt schema documented; legacy
  file's one-time migration recorded.
- **Contracts block, ch. 04**. One line: *"On each settled turn the
  witness runs the flash pass; every fired flash routes through the
  same mpsc as the turn loop's other frame writes, preserving the
  single-writer invariant on `turns.jsonl`."*

## Prompt files

- `workspace/witness/flashes/on-connection.md` (moved from
  `witness/on-connect.md`, contents unchanged). The only compose-why
  prompt shipped in Spec 1.
- No `on-echo.md`, `on-return.md`, `on-correction.md`, `on-danger.md`
  in this spec — their frames use fixed templates.
- Prompt convention preserved: an on-glean.md-shaped optional
  `on-<type>.md` file in the `flashes/` directory can be added later
  to override any type's fixed template with a witness-composed
  variant. Missing file = fixed template.

## Testing

Unit tests live inside `flashes.rs`:

- **Signals module.** Pure functions, table-driven tests: `text_sim`,
  `warmth`, `staleness_turns`, `rejection_sim` given fake ambient
  state; `danger_clusters` given synthetic streams (single cluster
  above count, single cluster below count, two overlapping clusters,
  refractory-suppressed).
- **Type predicates.** One table-driven test per type. Rows cover:
  threshold edge (just above / just below), refractory hit, self-
  write guard, disabled flag, per-target refractory.
- **Correction on glean.** The σ-retrieval hook fires a Correction
  frame when a candidate exceeds threshold against a synthetic
  rejection; `{similar_rejections}` substitution still happens
  (parity — witness's prior preserved).
- **Danger clustering.** Synthetic rejection stream, one firing per
  cluster with correct exemplars, refractory suppresses re-fire
  within the window, cluster identity by shared-member majority.
- **Log recovery.** Synthetic `flashes.jsonl` with mixed types,
  per-type refractory state rebuilds correctly.
- **Legacy migration.** Synthetic `connect-log.jsonl` present without
  `flashes.jsonl`; startup renames and populates `type: connection`
  on each entry; running again is a no-op.

Integration test at the witness level (using stubbed embedder and
stubbed model client, mirroring today's connect integration test):
one turn end-to-end, asserts a Connection fires when a knowledge
note clears threshold, an Echo fires alongside if the target is
warm, and both land as `[flash: <type>]` system-role lines on the
record via the mpsc pipe.

## Rollout

One deploy step: rebuild, restart.

On startup:

1. `witness/connect-log.jsonl` → `witness/flashes.jsonl` migration
   runs (idempotent).
2. `witness/on-connect.md` → `witness/flashes/on-connection.md`
   rename (manual — the deploy step does it; the module reads from
   the new path only).
3. `river.json` must have a `flash` block; the old connect knobs are
   gone. iris's config gets rewritten by hand (or, if we ship it, by
   a `river-cli migrate flash-config` helper — my lean is to skip
   the helper; it's a five-minute manual edit for one workspace).
4. Next settled turn fires flashes with the new prefix; historical
   `[connect]` lines in the record stay untouched.

Iris's live gateway is the only affected workspace; seed configs
gain a default `flash` block matching the defaults in this spec.

## Contracts

- **One flash pass per settled turn.** Digestion and heartbeat turns
  are excluded from the pass, matching connect's exclusions today.
- **At most one flash per type per turn** (Danger: at most one per
  turn per cluster).
- **Single-writer preserved.** All flashes route through the mpsc
  pipe; the turn loop is the only writer of `turns.jsonl`.
- **Divided authorship preserved.** No flash writes to `knowledge/`;
  flash frames appear only as system-role record lines and receipt
  entries in `witness/flashes.jsonl`.
- **Correction has two audiences, one signal.** The witness still
  gets `{similar_rejections}` in `on-glean`; the agent additionally
  gets a `[flash: correction]` line when a candidate would clear the
  threshold. Same threshold, same signal.
- **Danger adjudicates nothing.** Frame reports count, window, and
  exemplars only. Danger and Correction may both fire on the same
  material.
- **Fail-safe config.** Missing `flash` block disables the subsystem
  entirely — an un-migrated agent runs with no flashes rather than
  crashing.
- **Refractory is type-owned.** Each type's refractory state is
  independent; there is no global "one flash per turn" cap.

## Open questions

- **Composed-why for Echo/Return/Correction/Danger.** Fixed templates
  ship in Spec 1; if in-flight use shows the fixed language feels
  hollow, adding optional `on-<type>.md` prompt overrides is a small
  follow-up.
- **Memory-slot ambient dispatch.** All types fire as system-role
  lines in Spec 1. If the always-on-record volume becomes noisy,
  Fable's memory-slot mechanism is the natural next step.
- **Danger cluster identity.** The "shared-member majority" rule is
  a first cut; a proper cluster identity across time may need
  centroid tracking. Revisit if the log shows the same cluster
  firing repeatedly.

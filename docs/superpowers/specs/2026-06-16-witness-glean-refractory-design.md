# Witness Glean Refractory — Design

Status: approved 2026-06-16. The wall (ch. 04) specifies flat per-turn
gleaning probability plus a guaranteed end-of-session pass; nothing
prevents the witness from queueing successive candidates over heavily
overlapping source windows. A live report from iris-strix
(2026-06-16): 9 candidates in one stretch, 8 of them noise from the
same narrow band of material (a channel_read debug session plus three
restarts). The fixable thing is not "relevance" but **deduplication
by source proximity**: silence between conversations should produce
zero or one candidate, not nine.

## Scope

A structural refractory period on witness gleaning:

- After a candidate is queued at `up_to_turn = T`, no further glean
  fires until the agent reaches turn `T + N`, where `N` is configured
  per agent (default 12 = 2 × `GLEAN_WINDOW_TURNS`).
- Refractory is a **pre-model gate**: model is not called, no
  candidate is generated, only a debug log line is emitted.
- The gate applies to **both** wake paths — per-turn dice and the
  guaranteed end-of-session shutdown pass.

Out of scope: any semantic inspection of candidate content (this is
config-level, not model-level); wall-clock cooldowns (turn-distance
is the chosen signal); any change to digestion-turn handling or move
duty.

## Config

A new optional block on the agent's config (river-core `AgentConfig`):

```json
"witness": {
  "glean_min_new_turns": 12
}
```

- `glean_min_new_turns` — minimum turns of forward movement required
  between queued candidates. Default 12.
- `witness` block is optional; absent block = defaults.
  `deny_unknown_fields` applies, matching the rest of the config.
- Naming: the knob lives under a `witness` sub-block (not flat on
  `AgentConfig`) so future witness-side knobs share one home.

## State model

`Witness` gains in-memory:

- `last_glean_through: Option<u64>` — the `up_to_turn` of the most
  recently queued candidate.
- `glean_min_new_turns: u64` — the configured threshold.
- `glean_log_path: PathBuf` — `{workspace}/witness/glean-log.jsonl`.

### Persistence: `workspace/witness/glean-log.jsonl`

Append-only, one entry per queued candidate:

```json
{"id":"01JXP...","turn":47,"at":"2026-06-16T03:14:22Z"}
```

- `id` — engine ULID; identical to the row id used in
  `extraction_queue`, so cross-referencing is trivial.
- `turn` — the `up_to_turn` value the glean covered.
- `at` — ISO 8601 in UTC.

**Why a workspace JSONL file (not a SQLite table):**

- Inspectability. The agent can grep its own dedup decisions when
  the gate's behaviour feels off — same idiom as the channels' JSONL
  logs and `moves.jsonl`.
- Survival across data_dir disposal (ch. 10). When the SQLite cache
  is wiped, the queue is gone but `last_glean_through` is recovered
  from the log; the gate stays armed.
- Append-only matches the "record is the truth, derived caches
  rebuild" principle.

### Startup recovery

`Witness::load`:

1. Read `glean-log.jsonl` if present; skip malformed lines with a
   logged warning (same shape as `channels::scan`).
2. Tail entry's `turn` → `last_glean_through`. Missing file or empty
   log → `None`.

## The refractory check

Inside `glean(up_to_turn)`, after the digestion-turn early return
and before the model call:

```rust
if let Some(last) = self.last_glean_through {
    if up_to_turn.saturating_sub(last) < self.glean_min_new_turns {
        tracing::debug!(
            turn = up_to_turn,
            last_glean_through = last,
            min_new = self.glean_min_new_turns,
            "glean: skipped (refractory)"
        );
        return Ok(());
    }
}
```

The wake loop's dice roll still happens; the gate just makes a
within-refractory glean a no-op. No model call, no candidate.

## On successful enqueue

When the model returns a non-empty, non-sentinel candidate:

1. `let id = memory.enqueue_candidate(candidate)?` — the existing
   API already generates and returns the row ULID.
2. Append `{id, turn: up_to_turn, at: now()}` to
   `glean-log.jsonl`; fsync.
3. Update `self.last_glean_through = Some(up_to_turn)`.

**Ordering rule:** enqueue first, then log. A torn log line never
points at a missing queue row. The reverse would risk dedup state
out of sync with a phantom candidate.

## Failure modes & contracts

- **Pre-model gate.** Refractory fires before the model call; no
  tokens spent on within-refractory attempts.
- **Both wake paths gated.** Per-turn dice and shutdown-pass share
  the refractory. Wall ch. 04's "guaranteed end-of-session pass"
  reads as "the pass runs," not "a candidate is queued."
- **Digestion-turn filter takes precedence.** The existing
  digestion-turn early return runs before the refractory check; a
  digestion-turn glean is still skipped for its existing reason.
- **Log write follows enqueue.** A torn log line cannot describe a
  phantom queue row.
- **DB disposal** (ch. 10) leaves the log intact; gate recovers.
- **Workspace hand-edit.** Deleting `glean-log.jsonl` resets the
  gate to `None`; the next glean fires unblocked. Matches the
  "ground truth is the file" idiom.
- **No calibration drift.** The gate is a pure structural rule; no
  estimators, no semantics.

## Wall amendments

- **Ch. 04** — add a "Refractory" paragraph to *Duty two: gleaning*
  describing the gate; amend the **Glean cadence** contract:
  > Flat per-turn probability + guaranteed end-of-session pass, both
  > subject to a turn-distance refractory between queued candidates
  > (default 12, configurable per agent).
- **Ch. 10** — list `witness/glean-log.jsonl` among the
  workspace-resident derived files (alongside `record/moves.jsonl`
  and the channels' JSONL).

## Testing

Unit tests in `witness.rs`:

- Refractory blocks a glean within N turns of the last queued
  candidate (model not called).
- Refractory releases after exactly N new turns.
- First glean of an agent's life fires unblocked.
- Shutdown-pass glean is gated identically.
- `last_glean_through` recovers from the log file across two
  `Witness::load` calls in the same workspace.
- Missing log file → `None` → next glean fires unblocked.
- Torn line in the log is skipped, not fatal; the tail-before-the-
  torn-line still recovers correctly.
- Log entry writes only after the queue insert succeeds — verified
  with a mock that fails the enqueue mid-flight; expect no log line
  for the failed candidate.

## Out of scope (v1)

- **Semantic dedup** (checking what the candidate cites). Iris ruled
  this out: structural rule only.
- **Wall-clock cooldowns.** Turn-distance is the chosen signal.
- **Per-channel refractory.** The gate is agent-global; a busy
  conversation on one channel naturally suppresses gleans about
  another, which is acceptable. Revisit if cross-channel evidence
  shows real loss.
- **Adaptive thresholds.** N is a config knob, tuned by hand. No
  auto-tuning logic.

# Settle tool

**Status:** design, 2026-07-14
**Author:** cass + Claude Opus 4.7
**Depends on:** wall ch. 01 (turn cycle), ch. 07 (tools)

## Motivation

Today a turn ends when the model returns a response with no tool calls,
and the next heartbeat wake is a fixed `heartbeat_minutes` sleep after
each settle. That has two frictions:

1. **Ambiguous end-of-turn.** The agent has no first-class way to say
   "I'm done." It has to hope the model produces an empty tool-calls
   array. Well-trained models sometimes emit an extra "OK" call or a
   redundant read just to have *something* to do.

2. **No agent-side cadence knob.** The heartbeat interval is a config
   constant. An agent that wants to disappear for two hours to let a
   long-running human conversation breathe — or wake more frequently
   through a busy stretch — has nowhere to put that intent.

`settle` addresses both.

## Tool surface

Name: `settle`. Registered in `Registry::core()`; bumps
`DEFAULT_TOOLS` by 1 (currently 14).

**Arguments:**

- `next_heartbeat: Option<u64>` — minutes. Optional.

**Semantics:**

- `settle()` (no arg) — set `next_heartbeat_at = now + config_default`
  (i.e., the existing `agent.heartbeat_minutes`, currently 45).
- `settle(N)` — clamp `N` to `[1, 480]` and set
  `next_heartbeat_at = now + clamped(N)`.

Either form ends the current turn (see "Batch behavior" below).

**Bounds:**

```rust
const SETTLE_FLOOR_MINUTES: u64 = 1;
const SETTLE_CEILING_MINUTES: u64 = 480; // 8h
```

Clamp behavior: silent clamp to the bound, but the tool result JSON
surfaces the requested and clamped values so the model can notice.

**Tool result JSON:**

```json
{
  "next_wake_at": "2026-07-14T15:30:00-05:00",
  "seconds_until": 5400
}
```

When clamping happened, two extra fields appear:

```json
{
  "next_wake_at": "2026-07-14T23:00:00-05:00",
  "seconds_until": 28800,
  "requested_minutes": 1000,
  "clamped_to_minutes": 480
}
```

The model will not consult this result — the turn ends after settle
runs — but the JSON is appended to the record for observability and
for future readers.

## Turn-loop mechanics

Add one field on `TurnLoop`:

```rust
next_heartbeat_at: std::time::Instant,
```

**Initialization (cold start).** In `TurnLoop::new()`:

```rust
next_heartbeat_at: std::time::Instant::now() + heartbeat,
```

Where `heartbeat: Duration` is the existing config-default field. No
persistence across gateway restart — the deadline is re-initialized on
boot.

**Wake loop change** (`turn.rs`, in the `select!`):

```rust
// before:
_ = tokio::time::sleep(self.heartbeat) => Wake::Heartbeat,
// after:
_ = tokio::time::sleep_until(self.next_heartbeat_at.into()) => Wake::Heartbeat,
```

**End-of-turn recompute** (in the settle path of `TurnLoop::turn()`):

1. Drain the settle intent (see "The intent slot" below).
2. If a settle intent exists, apply the last one (the one that actually
   ran in the batch) and recompute `next_heartbeat_at`.
3. If no intent → **leave `next_heartbeat_at` alone**. Natural settle
   preserves the existing deadline. This is the deadline model:
   non-settle wakes and natural turn-ends don't reset the countdown.

**Non-heartbeat wakes** (channel, digestion, notify, quiet-recheck):
never touch `next_heartbeat_at`. If they wake the agent early, the next
`sleep_until` continues toward the same deadline once that turn's
settle completes.

## Batch behavior and the intent slot

The tool call runs inside a tool dispatch loop; it can't reach into the
turn loop directly. Route the intent through a slot on `ToolContext`,
following the same pattern as `shape_queue`.

```rust
enum SettleIntent {
    Bare,                // settle() — recompute to now + default
    NextHeartbeat(u64),  // settle(N), N already clamped
}

// in ToolContext:
settle_intent: Arc<Mutex<Option<SettleIntent>>>,
```

Ownership: the `TurnLoop` owns the `Arc<Mutex<Option<SettleIntent>>>`
and clones it into `ToolContext` at each dispatch — same lifetime
pattern as `shape_queue`. `SettleTool::run` writes into the slot and
returns `Ok` with the result JSON. It does not itself end the turn.

**Dispatch loop change** (`TurnLoop::turn()`): after each model
response, tool calls execute in order (existing behavior). Add a
post-batch check: if `settle_intent` is `Some`, the turn ends after the
current batch resolves — do not issue another model call this turn.
The settle path then applies the intent and clears the slot.

**Consequences:**

- **Batch ordering is preserved.** `[write_atomic(...), settle(60)]`
  runs write_atomic first, then settle sets intent, then the turn ends.
- **Last-writer-wins in a batch.** Two settle calls in one batch: the
  later one overwrites the slot. Log a warning at info level; don't
  error. The model shouldn't do this, but we won't punish it.
- **Deterministic turn end.** No reliance on the model returning zero
  tool calls when it wants to be done.

**Interaction with existing turn-end paths.**

- **Iteration ceiling hit** (wall ch. 01, "hitting it ends the turn
  through the normal settle path"): if `settle_intent` is set, honor
  it. Otherwise natural rules apply.
- **Shutdown mid-turn**: same. Settle intent honored if present.
- **Model call failure**: same.

## Config

No new config keys. The existing `agent.heartbeat_minutes` in
`river-core/src/config.rs` remains the source of truth for the default
heartbeat. The floor and ceiling live in the settle tool module as
constants — they're guard rails, not user-tunable.

## Seed

Add a short teaching paragraph to `seed/AGENTS.md` on `settle`:

- It ends the current turn deterministically.
- It's optional — a response with no tool calls still ends the turn
  naturally.
- `next_heartbeat` (minutes) is your knob for when you want to wake
  next. If you don't pass it, the default cadence applies. If you do,
  it sets a deadline; non-settle wakes in the meantime don't reset it.

## Wall updates

- **`docs/wall/01-turn-cycle.md`**: extend the "how a turn ends"
  paragraph to include `settle`. Note the deadline model in one line:
  "Explicit `settle` recomputes `next_heartbeat_at`; other wakes and
  natural turn-ends preserve it."
- **`docs/wall/07-tools.md`**: add `settle` entry with args (the
  optional `next_heartbeat`), semantics, and result shape.
- **Contract block on ch. 01**: add one line —
  "Explicit `settle` recomputes `next_heartbeat_at`; other wakes
  preserve it."

## Tests

Every test in this list is designed to **hunt** a specific regression,
not to pass. If a test can't fail on a plausible drift of the code,
it's dead weight and shouldn't be written.

1. **`settle_bare_ends_turn_and_uses_default`** — call `settle()`;
   assert the turn ends and `next_heartbeat_at ≈ now + default`.
   Hunts: bare-settle silently preserving an old deadline instead of
   recomputing.
2. **`settle_with_arg_sets_deadline`** — call `settle(120)`; assert
   `next_heartbeat_at ≈ now + 120min`.
   Hunts: unit confusion (seconds vs minutes), stale-state bugs.
3. **`settle_clamps_below_floor`** — `settle(0)` → 1min; result JSON
   contains `clamped_to_minutes: 1` and `requested_minutes: 0`.
   Hunts: clamp missing, clamp silent (no result surfacing).
4. **`settle_clamps_above_ceiling`** — `settle(1000)` → 480min; result
   JSON contains `clamped_to_minutes: 480, requested_minutes: 1000`.
   Hunts: ceiling missing, off-by-one at the ceiling.
5. **`settle_runs_after_batch_peers`** — batch of
   `[write_atomic(...), settle(60)]`; assert write_atomic's side
   effects happened, then the turn ended, deadline is `now + 60min`.
   Hunts: settle short-circuiting the batch and dropping the peer.
6. **`natural_settle_preserves_deadline`** — call `settle(120)`, wake
   the loop by a channel message before 120min elapses, let the second
   turn end naturally (no settle call); assert `next_heartbeat_at`
   equals the original deadline.
   Hunts: natural settle resetting the deadline (the whole point of
   the deadline model).
7. **`settle_on_intermediate_turn_recomputes`** — call `settle(120)`,
   wake by channel at simulated t+20min, call `settle(60)` on that
   turn; assert new deadline = t+20+60.
   Hunts: settle-intent being ignored on non-heartbeat turns.
8. **`settle_multiple_in_batch_last_wins`** — batch of
   `[settle(60), settle(30)]`; assert deadline = now + 30min; assert a
   warning was logged.
   Hunts: first-wins ordering, missing warning.
9. **`cold_start_deadline_from_config`** — new `TurnLoop`; assert
   `next_heartbeat_at ≈ startup + config_default` before any turn
   runs.
   Hunts: default not applied at construction, panic on
   uninitialized deadline.
10. **`iteration_ceiling_honors_settle_intent`** — configure a low
    iteration ceiling, have the last iteration's batch include
    `settle(30)`; assert deadline = now + 30min after the ceiling
    forces the turn to end.
    Hunts: ceiling path bypassing settle-intent drain.
11. **`shutdown_mid_turn_honors_settle_intent`** — send shutdown while
    a turn is in flight after a settle intent has been set; assert the
    turn's settle applies the intent (deadline is set correctly for
    when the gateway comes back, even though we're about to shut
    down).
    Hunts: shutdown path bypassing intent drain.

## Kanban

Add `settle tool` under **In Progress** on `docs/Kanban.md` at
implementation start; move to **Implemented** with a note when it
lands.

## Non-goals

Called out explicitly so the implementation doesn't drift:

- **No persistence** of `next_heartbeat_at` across gateway restart.
- **No memory/witness/context slot changes.** Settle is purely a
  turn-loop mechanism.
- **No settle-log jsonl.** Considered and rejected. Observability
  lives in the tool-result JSON on the transcript.
- **No wake-cancel API.** The deadline is mutable only by another
  settle call.
- **No batch rejection or error on multi-settle.** Last-writer-wins
  with a warning is enough.

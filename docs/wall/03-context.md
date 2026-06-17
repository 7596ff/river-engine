# 03 — Context

The context is a **persistent object**, not a per-turn artifact. It is
built once when the agent starts (or switches channels), and messages are
appended in place as the conversation proceeds. It is rebuilt only by
**compaction**, and compaction only ever drops what the witness has
already compressed. The agent never observes any of this machinery — it
converses; the context object handles the rest.

## Assembly order

Top to bottom, the context reads: *who I am → what has happened → what
memory offers → what is happening now.*

```
1. SYSTEM       identity files (AGENTS.md + IDENTITY.md + RULES.md,
                ch. 08) joined with separators, plus environment:
                current time (workspace-configured timezone)
2. ARC          the witness's moves — the life's arc, whichever
                channel each turn faced — oldest→newest, as one
                system message headed "[Conversation arc]"
3. MEMORY SLOT  what the memory system offers this turn: flashes with
                their 1-hop neighbors, the warmest notes, retrieved
                results — token-budget-bounded (ch. 02)
4. HOT          the conversation messages, in order, each tagged with
                its turn number (ch. 01)
```

The memory slot is a *slot*: context assembly defines where memory
content goes and how much room it gets; the memory system decides what
fills it. Either side can evolve without the other.

## Growth and compaction

Messages append until the estimated total reaches the **compaction
threshold** (default 80% of the context limit). Then, before the next
model call:

1. Re-read the system prompt from disk (identity edits take effect here
   and at channel switches — never mid-stretch).
2. Read the **witness cursor**: the contiguous compression frontier of
   the moves file (ch. 10) — the tail when the file is gapless. If the
   witness has never run — no moves file, or an empty one — the cursor
   is 0.
3. Drop all messages belonging to turns **at or below the cursor** —
   whole turns only, never a partial turn. These turns are represented
   in the arc; dropping them loses nothing.
4. If fewer than **min_messages** (default 50) remain, backfill whole
   turns from below the cursor, newest first, stopping if the next turn
   would push past the threshold. The floor is best-effort.
5. Reload the arc: moves newest-first from the record until the
   **fill target** (default 40% of limit) is reached, then presented
   oldest-first. Old moves fall off here — they remain in the record.
   Moves whose turn is currently in hot are skipped: the full turn at
   high resolution already represents it, and the compressed summary
   would only duplicate. Skipping them frees the arc budget for older
   moves the model has no other way to see.
6. Refresh the memory slot.
7. **Never re-trigger.** If the result still exceeds the threshold (the
   witness is far behind), accept it and continue; the next compaction
   comes when accumulation crosses the threshold again.
8. **Lag warning.** If the post-compaction total exceeds the midpoint of
   fill target and threshold (default 60%) *and* the agent is ≥ 10 turns
   ahead of the cursor, append a system message telling the agent its
   compression is behind, by how much, and that it may want to respond
   more briefly or say so. The agent can act on it or ignore it.

Session start is the same algorithm with the record file as the message
source: scan `record/turns.jsonl` backward, collecting whole turns that
**touch** the channel (any line tagged with it) above the cursor,
backfill whole turns to the floor, load the arc from the moves file,
go. A channel switch (deferred to the next turn start, so a turn's tool
calls are never split across contexts) rebuilds the same way for the
new channel — and because the record is one stream, an exchange
conducted about this channel from elsewhere is not lost to it: the
whole turn rides in with the switch.

**The lossless guarantee**, stated once and bindingly: no message that
the witness has not compressed into a move is ever dropped from context.
If the witness falls behind, the context degrades by *crowding* (less
room for arc and memory), never by forgetting.

## Token estimation

Estimates are heuristic and self-correcting. Base: `len/4 + 4` per
message (content plus tool-call payloads). After every model response,
calibrate against the prompt token count the model reports:

```
ratio ← 0.7 · ratio + 0.3 · (reported / estimated)      (start: 1.0)
```

Skip the update when either side of the division is zero. All estimates
multiply by the current ratio. The weighted average smooths
content-type swings (code vs prose) and converges within a few turns.

## Session resume

A restart is not a death. Between sessions, the ephemeral pieces of
the context — the current channel, the estimator's calibration ratio,
the memory slot's active flashes, the quiet-gate timer — would
otherwise reset, producing a "little death" in which the agent wakes
on `local_main` with cold estimates and an empty slot regardless of
what it was doing before. `workspace/session.json` carries these
across restarts.

- **Written at every settle** via atomic tmp + fsync + rename, so a
  live snapshot is at most one turn behind reality and a killed
  gateway mid-write never leaves a torn file.
- **Loaded once at startup.** Missing or malformed file → fall back
  to derivation: the channel is the tail of `record/turns.jsonl`
  (where iris was actually talking), other fields reset to defaults.
- **Hot and arc still rebuild from the record + moves** — those are
  ground truth in the workspace already. Session resume only carries
  the in-memory state that has no other home.
- `quiet_seconds` is the elapsed wall-clock between the last
  significant event and the snapshot. On resume the timer continues
  as if that much silence had already passed — extended downtime is
  extended silence, and a candidate that was about to fire fires.

## Configuration

Four knobs, nothing per-layer:

| knob | default |
|---|---|
| `limit` | 128,000 tokens |
| `compaction_threshold` | 0.80 |
| `fill_target` | 0.40 |
| `min_messages` | 50 |

Derived, not configured: lag warning at the midpoint of fill target and
threshold; turn-lag threshold of 10.

## Contracts

- **Lossless.** Only turns at or below the witness cursor are ever
  dropped. Cursor 0 (witness never ran) means nothing is droppable.
- **Turn-atomic.** All messages of a turn drop or stay together; a tool
  call is never separated from its result.
- **No re-trigger.** Compaction runs at most once per turn, and its
  output is accepted even if over threshold.
- **Persistent object.** No per-turn rebuild. Rebuilds happen only at
  session start, channel switch, and compaction.
- **System prompt freshness.** Identity files are re-read at session
  start, channel switch, and compaction — never mid-stretch.
- **Slot discipline.** Memory content appears only in the memory slot,
  between arc and hot. Memory may leave it empty; assembly never blocks
  on memory.
- **Arc–hot disjoint.** A turn appears in at most one layer at a
  time: as a full turn in hot, or as a one-line move in the arc,
  never both. When backfill pulls a compressed turn back into hot,
  its move is suppressed from the arc for that assembly.
- **Session resume is honest.** `workspace/session.json` is written
  atomically at every settle and read once at startup. Missing or
  malformed → fall back to derivation; the record tail names the
  channel. Hot and arc are never persisted in the snapshot — they
  rebuild from `record/turns.jsonl` and `record/moves.jsonl`.
- **Calibration** uses reported prompt tokens only, with the 0.7/0.3
  weighted average and zero-skip.

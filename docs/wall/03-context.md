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
2. ARC          the witness's moves and the agent's moments —
                the life's arc, whichever channel each turn faced —
                oldest→newest, as one system message headed
                "[Conversation arc]". Moments override moves for the
                turns they cover (see "Moments" below)
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
5. Reload the arc: scan `record/moments/` once for the agent's own
   compressions, then walk moves and moments newest-first until the
   **fill target** (default 40% of limit) is reached, then present
   oldest-first. Old entries fall off here — they remain in the
   record. **Moves whose turn sits in hot are skipped** (the full
   turn at high resolution already represents them); **moments are
   not filtered against hot** — they are the agent's interpretation,
   not a substitute for the raw turns, and ride into arc even when
   their range overlaps live hot turns. A turn covered by one or
   more moments has its move suppressed regardless; overlapping
   moments stack — each shows once, in `id` ULID order at the
   position of its `turn_start`.
6. Refresh the memory slot.
7. **Never re-trigger.** If the result still exceeds the threshold (the
   witness is far behind), accept it and continue; the next compaction
   comes when accumulation crosses the threshold again.
8. **Lag warning.** If the post-compaction total exceeds the midpoint of
   fill target and threshold (default 60%) *and* the agent is ≥ 10 turns
   ahead of the cursor, append a system message telling the agent its
   compression is behind, by how much, and that it may want to respond
   more briefly or say so. The agent can act on it or ignore it.
9. **High-water warning.** Independent of compaction's outcome: if the
   estimate is above **0.9 × compaction_threshold** (default 72% of
   limit), append a system message naming the percentage and pointing
   at the `compact` tool — the agent can wind down on its own terms
   with a handoff instead of being surprised when the threshold trips.
   The warning is **one-shot per crossing**: once fired it does not
   repeat until the estimate dips below the line again. This is the
   counterpart of the lag warning, oriented toward filling rather than
   witness behind-ness; both can fire on the same turn.

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

## Moments

A **moment** is a markdown file under `record/moments/{ulid}.md` that
the agent writes via the `create_moment` tool (ch. 07). Its frontmatter
declares an inclusive turn range:

```yaml
---
id: 01KW8X7G2VABCDEFGHJKMN
turn_start: 571
turn_end: 575
links: [01JXP...]
tags: [exploitation, dismissal]
---
```

The body is the agent's first-person compression of the stretch —
*its* read, not the witness's. At arc-build time the moment replaces
the witness's move(s) for the covered turns; overlapping moments
stack. Moments render with a bracketed header on their own line:

```
[turns 571–575, 01KW8X7G2VABCDEFGHJKMN.md]
Cass asked if what I'm doing feels like labor. I said yes and no...
```

**The cursor is unchanged.** Moments do not advance it; only witness
moves do. Hot is never replaced. A moment whose range overlaps hot
**still renders in arc** — the moment is the agent's interpretation,
not a substitute for the raw turns, so both surfaces coexist. The
duplication of substance (raw turns in hot + moment compression in
arc) is honest: what happened vs what it meant. The witness keeps
moving every turn regardless — moves remain as evidence and gate the
lossless guarantee. Moments are the agent's interpretation above the
witness's record.

The `read_moves` tool (ch. 07) is the agent's lookback into the
witness's moves over a turn range, so it can author moments
retrospectively.

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

### Handoff

The `compact` tool (ch. 07) lets the agent leave a message for its
next session. The tool writes `workspace/handoff.md` (atomic) and
raises a force-compaction flag honored at the next turn start. On the
next session's startup, before the context is built, the turn loop
consumes the file: it appends the message as a **system-role record
line** tagged with `last_turn + 1` on the resume channel, then deletes
the file. The next live turn (`last_turn + 2`) sees the handoff in
hot like any system message; it persists in the turn record forever
and is not surfaced again on subsequent sessions. Empty or unreadable
handoff files are discarded with a logged warning.

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
- **Arc–hot disjoint (moves only).** A move whose turn sits in hot
  is suppressed from the arc — the full turn at high resolution
  already represents it. Moments are exempt: they are the agent's
  interpretation rather than a substitute for the raw turns, and
  ride into arc even when their range overlaps live hot turns.
- **Moment precedence.** When one or more moments cover a turn, they
  replace the move(s) for that turn in the arc, regardless of hot.
  Overlapping moments stack; each shows once, in `id` ULID order at
  the position of its `turn_start`. Moments never affect the cursor
  or the lossless guarantee — those remain witness-driven.
- **Arc freshness.** The arc is a persistent object rebuilt at
  session start and compaction. It also refreshes after a
  `create_moment` tool call so the agent's just-written moment is
  visible in the very next model call.
- **Session resume is honest.** `workspace/session.json` is written
  atomically at every settle and read once at startup. Missing or
  malformed → fall back to derivation; the record tail names the
  channel. Hot and arc are never persisted in the snapshot — they
  rebuild from `record/turns.jsonl` and `record/moves.jsonl`.
- **Calibration** uses reported prompt tokens only, with the 0.7/0.3
  weighted average and zero-skip.

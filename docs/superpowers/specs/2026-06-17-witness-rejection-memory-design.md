# Witness Rejection Memory + Queue Cap — Design

Status: approved 2026-06-17. Follows up the 2026-06-16 refractory work
(`docs/superpowers/specs/2026-06-16-witness-glean-refractory-design.md`).

## Background

The refractory fixed the cascade — successive gleans no longer re-mine
the same source window. But a fresh report from iris-strix
(2026-06-17): after a restart, the agent woke to **eight separate
`[digestion]` turns** across a quiet night, rejecting each one in
turn. The eight candidates were a mix of pre-restart queue residue
plus post-restart gleans that got through the gate. Two failure
modes she named:

- **Mining the witness's own design conversation.** When the agent
  works on the witness's machinery, the next glean extracts "patterns"
  from that meta-conversation. The witness sees its own debugging as
  knowledge.
- **Fixation on warm passages.** Goodnights, brief affectionate
  exchanges. The witness keeps re-surfacing them across passes
  because nothing tells it the agent already said no.

iris's diagnosis: the witness exercises in-context judgment on each
glean call but is **stateless across calls**. It has no awareness of
queue contents, no memory of what the agent accepted or rejected, no
view of what notes were written. Every glean is a fresh judgment with
no learning.

The fix has two parts, decided in the 2026-06-17 brainstorm:

1. **Rejection memory** — give the witness a persistent record of
   past rejections so it can learn what doesn't land.
2. **Queue depth cap** — bound the damage when judgment fails. A
   hard ceiling at enqueue time keeps a productive session from
   filling the next quiet stretch with eight weak candidates.

## Out of scope

- Prompt tuning beyond the minimal addition needed to surface the new
  `{recent_rejections}` template variable. The shipped `on-glean.md`
  changes name the new section; finer-grained prompt language belongs
  to the operator, not the engine.
- Semantic dedup at enqueue time (embedding-based "this is similar to
  what was just queued"). The structural rules already chosen are
  enough; revisit if evidence shows they aren't.
- Acceptance memory. iris suggested rejection only; the agent's note
  is the positive signal and survives in the workspace. No need for
  a parallel acceptance log.

## Reject mechanism: `reject_candidate` tool

A new core tool that the agent calls during a digestion turn to mark
the current candidate as no-go.

```
reject_candidate(reason?: string)
```

- The candidate is resolved implicitly from turn context — whichever
  candidate was the inbound of the current digestion turn. No
  candidate-id parameter.
- `reason` is optional but encouraged. When present it goes into
  `rejections.jsonl` verbatim and the witness reads it as "here's
  why this didn't land."
- Calling outside a digestion turn returns a tool error — same
  error-as-result machinery as the rest of the tools.
- Calling more than once in the same digestion turn appends each
  call. Idempotent in effect; the witness reads all entries.
- Joins `DEFAULT_TOOLS`. The default profile grows from 9 to 10.

**Why a tool, not a heuristic.** Rejection is a positive act. An
implicit signal ("the agent didn't write a knowledge note this
turn → infer rejection") would confuse "rejected the candidate" with
"interrupted by inbound," "wrote a note about something else," "went
to bed," and many others — the witness would learn the wrong lesson.
The wall already frames it: "the agent will write its own note in
its own words — or reject it. That division is permanent."

### `DigestionInfo` on `ToolContext`

The turn loop populates a new optional field when entering a
`Wake::Digestion` turn:

```rust
pub struct DigestionInfo {
    pub candidate_id: String,
    pub candidate_text: String,
    pub turn: u64,
}
```

`None` on all other wake kinds, so `reject_candidate` can error fast
when called outside its valid window.

To make this work, `Memory::pop_candidate` returns
`(String /* id */, String /* text */)` instead of just the text — a
small signature change. (Callers: only the turn loop; existing tests
that use `pop_candidate` adapt trivially.)

## Storage: `workspace/witness/rejections.jsonl`

Append-only, one entry per `reject_candidate` call:

```json
{
  "candidate_id": "01JXP...",
  "candidate": "<full text of the rejected candidate>",
  "reason": "warm moment but no extractable claim — this was just goodnight",
  "turn": 731,
  "at": "2026-06-17T03:14:22Z"
}
```

- `candidate_id` cross-references the queue row id and the
  `glean-log.jsonl` entry that introduced it.
- `candidate` is the full text — the witness reads this file alone,
  with no need to join against the queue (the row will typically have
  popped by the time of rejection).
- `reason` is omitted from the entry when the agent called the tool
  with no argument.
- `turn` is the digestion turn the rejection happened in.
- `at` is ISO 8601 UTC, same format as `glean-log.jsonl`.

**Why a workspace JSONL (not data_dir SQLite):** matches the
inspectability and disposability rules of `glean-log.jsonl` (per the
2026-06-16 spec). iris can grep her own rejections. The file
survives data_dir disposal. Hand-deletion resets the rejection
memory, same idiom as deleting other workspace logs.

**Torn-line tolerance.** Same shape as channels.rs and the existing
glean-log recovery: malformed lines are skipped with a warning,
never fatal.

## How the witness reads rejections

A new template variable `{recent_rejections}` in `on-glean.md`. The
engine substitutes the last **N** rejection entries into the prompt,
where N is configurable (default 5).

**Rendered format** when there is at least one entry:

```
[your prior gleans the agent rejected]
turn 612: "Sustained exchange isn't always extraction-worthy" — reason: warm moment, no claim
turn 623: "The pattern of 'enqueue then log' is Logging Causality" — reason: meta-mining
turn 731: "That's not nothing" carries weight — reason: just a goodnight
```

- Candidate text is truncated to a one-line preview (~80 chars).
- Reason appears verbatim if present, omitted otherwise.
- Entries are oldest-first (chronological).

**Rendered when the list is empty:** the variable substitutes to the
empty string. No header, no placeholder. The prompt reads naturally
on day-one.

**Why a moving window, not all-time history:** unbounded growth
would crowd the actual `{recent_record}` material. N is a lesson
horizon — the witness learns from yesterday's misses, not from a
year ago.

## Queue depth cap

A hard ceiling at enqueue time. Witness's path through
`Memory::enqueue_candidate`:

1. `let depth = memory.queue_depth()?;`
2. If `max_queue_depth > 0 && depth >= max_queue_depth`, the engine
   drops the candidate:
   - `tracing::warn!` line carrying the candidate text and turn for
     postmortem grep.
   - **No append to `glean-log.jsonl`.**
   - **`last_glean_through` stays untouched** — refractory state
     isn't burned by a drop, so the next eligible turn still glean.
3. Otherwise, enqueue normally and continue with the existing
   glean-log + state update flow.

The drop is silent to the witness's model — the chat call already
returned successfully; the engine just doesn't queue what came back.

**Zero disables the cap** — same convention as
`glean_min_new_turns: 0` disabling the refractory.

**Default 5.** Matches iris's "0-1 candidates per quiet stretch"
target with headroom for a session that produces several
genuinely-different candidates before sleep. Aggressive caps (2-3)
risk dropping good candidates on productive streaks; generous caps
(10+) re-introduce the eight-wakes problem.

## Config

```json
"witness": {
  "glean_min_new_turns": 12,
  "max_queue_depth": 5,
  "recent_rejections_window": 5
}
```

- `glean_min_new_turns` — existing; default 12 (= 2 × glean window).
- `max_queue_depth` — new; default 5; zero disables.
- `recent_rejections_window` — new; default 5; number of recent
  rejections rendered into `{recent_rejections}`.

All three live under the `witness` sub-block with
`deny_unknown_fields`. Block remains optional; absent block = all
defaults.

## Surface impact

| file | change |
|---|---|
| `crates/river-core/src/config.rs` | two new fields on `WitnessConfig` |
| `crates/river-gateway/src/tools.rs` | `RejectCandidateTool`; `ToolContext.digestion`; `DEFAULT_TOOLS` grows to 10 |
| `crates/river-gateway/src/turn.rs` | populate `DigestionInfo` on `Wake::Digestion`; pass `(id, text)` from `pop_candidate` |
| `crates/river-gateway/src/memory.rs` | `pop_candidate` returns `(id, text)`; queue-depth check stays in the witness's enqueue path |
| `crates/river-gateway/src/witness.rs` | read `rejections.jsonl`, render `{recent_rejections}`; enforce queue cap |
| `seed/witness/on-glean.md` | mention `{recent_rejections}` section in the seed prompt |
| `docs/wall/04-witness.md` | *Rejection memory* paragraph; amend on-glean.md template doc |
| `docs/wall/07-tools.md` | add `reject_candidate` row to the registry |
| `docs/wall/02-memory.md` | note the queue cap |
| `docs/wall/10-data.md` | list `witness/rejections.jsonl` |
| `docs/decisions.md` | log the decision |

## Failure modes & contracts

- **`reject_candidate` outside a digestion turn** → tool error result.
- **`reject_candidate` called more than once in a digestion turn** →
  each call appends; the witness reads them all.
- **Queue at cap when a great candidate arrives** → dropped. Same
  topic in a later glean window with queue freed will surface again.
- **Rejections file hand-deleted** → witness reverts to memoryless
  state; gleans without rejection context. Matches the
  "ground-truth-is-the-file" idiom.
- **Torn line in `rejections.jsonl`** → skipped with a warning, same
  as the existing glean-log recovery.
- **DB disposed (ch. 10)** → queue gone, but rejection memory stays
  in the workspace; the witness learns across DB resets.
- **No semantic dedup.** Two distinct passes can each return
  meaningfully different candidates that the agent rejects; the
  rejection log records both, and the witness reads both.

## Testing

Unit tests in `tools.rs`:

- `reject_candidate` appends a correctly-shaped entry to
  `rejections.jsonl` (text, reason, turn, timestamp).
- `reject_candidate` errors when called outside a digestion turn.
- `reject_candidate` with no reason omits the field.
- Multiple `reject_candidate` calls in one digestion turn produce
  multiple entries.

Unit tests in `witness.rs`:

- Glean prompt includes `{recent_rejections}` with the last N entries
  when the file has entries.
- Glean prompt substitutes empty string when the file is missing.
- Queue cap drops enqueues when at `max_queue_depth`;
  `last_glean_through` stays untouched on a dropped enqueue.
- Queue cap of zero disables the cap.
- `recent_rejections_window` recovers across `Witness::load`:
  missing file → empty list, nonzero file → tail N entries.
- Torn line in `rejections.jsonl` is skipped, not fatal.

Unit tests in `memory.rs`:

- `pop_candidate` returns `(id, text)`; FIFO ordering preserved.
- `queue_depth` reflects enqueues and pops accurately.

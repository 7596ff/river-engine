# 04 — The Witness

The witness is the second voice: retrospective, second person, watching
the agent's turns from outside. It is a **role, not a process** — a
concurrent task in the same binary, subscribed to the same event bus,
with its own model assignment (often a smaller, cheaper model than the
agent's; the work is summarization, not reasoning).

It has four duties: **moves**, **gleaning**, **connecting**, and
**shape**. Compression of the record, the harvest of its margins,
the retrieval of already-written knowledge the current turn resonates
with, and the composition of one-line skeletons of atomic notes for
the derived `shape_vectors` table (ch. 02). It writes to the moves
file, the extraction queue, its own receipt logs
(`glean-log.jsonl`, `connect-log.jsonl`, `shape-log.jsonl`, and the
derived `rejection_vectors`/`shape_vectors` tables, ch. 10). Connect
frames it produces are routed through the turn loop so the
single-writer invariant on `turns.jsonl` (ch. 10) is preserved. It
never writes to the agent's knowledge, never speaks on channels,
never touches tools.

## Prompt-driven, entirely

The witness's behavior is markdown in `workspace/witness/`. The runtime
is a thin dispatcher: receive an event, load the prompt, substitute
variables, call the model, handle the structured output.

```
workspace/witness/
  identity.md       system prompt for every witness model call — REQUIRED
  on-turn.md        produces a move (template vars: {transcript},
                    {turn_number}) — optional
  on-glean.md       produces extraction candidates (template vars:
                    {recent_record}, {recent_rejections},
                    {similar_rejections}) — optional
  on-connect.md     composes a one-sentence connection why (template
                    vars: {transcript}, {target_path},
                    {target_excerpt}) — optional
  on-shape.md       composes a one-line logical skeleton of an atomic
                    (template var: {note_body}) — optional; missing
                    file disables the shape duty and leaves the
                    derived shape_vectors empty for witness-authored
                    rows (agent-authored `shape:` frontmatter rows
                    still populate)
```

`identity.md` is required: **if it is missing, the gateway fails at
startup**, with the same severity as a missing agent identity file. This
is not pedantry — forgetting-safety depends on the witness. Compaction
can only drop what the witness has compressed (ch. 03), so a harness
running without its witness is a harness whose context fills and pins.
The witness's liveness is a startup invariant, not a feature flag.

The event prompt files are optional: a missing file disables that duty,
silently and legibly (logged once at startup). This is the tuning
surface — the human shapes the witness by editing prose, not code.

The identity seed shipped with the engine (ch. 08) sets the voice:
second person, always — "you did this," never "the agent did this."
A witness, not a judge. It names patterns, notices what was dropped,
compresses honestly — including failures and dead ends the agent might
prefer to forget. Its compressions are the agent's long-term memory of
its own life; what it skips is lost; that responsibility is the job.

## Duty one: moves

On every `TurnComplete { turn_number }`:

1. Read the turn's messages from the turn record
   (`record/turns.jsonl`, ch. 10) — a tail scan for the turn
   number. The witness never trusts a self-summary from the agent;
   it reads what actually happened.
2. Format a transcript; substitute into `on-turn.md`; call the model
   with `identity.md` as system prompt.
3. Append the response as a **move** line to the moves file
   (`record/moves.jsonl`): turn number plus a 1–2 sentence structural
   summary capturing shape (question, request, correction, task,
   failure, tangent) and substance (what it was about).
4. If the model call fails, append a **fallback move** built
   mechanically from the roles and tool names involved — the turn is
   never lost from the arc. Log the failure.

One move per turn. Moves are the unit of safe forgetting and the lines
of the conversation arc. They accumulate in the record forever; the
context's arc budget decides how many ride along (ch. 03).

The witness has no awareness of **moments** (ch. 03 / ch. 07) — the
agent's own compressions that override moves in the arc. Moments live
in `record/moments/`, parallel to but independent of `moves.jsonl`.
The witness keeps moving every turn regardless; the moment is the
agent's interpretation above the witness's evidence, and arc-build is
what gives moments precedence.

## Duty two: gleaning

After any turn, with flat probability (default 0.25), plus one
guaranteed pass at session end: the witness reviews the agent's recent
activity — the last few turns of messages and moves, *and any notes
the agent wrote in its workspace since the last pass* (its loom, if it
keeps one) — against `on-glean.md`, and writes **extraction
candidates** into the queue (ch. 02). The agent's own narrative notes
are often where the best candidates live: the agent's telling of what
happened carries claims its raw transcript does not. Each candidate is prose addressed to the agent: what is worth
thinking about again, citations of the agent's actual words, suggested
typed links. The flat rate is deliberate: the agent cannot predict
which turns will be gleaned, and the unpredictability is structurally
healthy. The end-of-session pass catches what the dice missed.

Gleaning is the anti-enclosure right made operational (ch. 02). The
witness's retrospective distance is the point: it sees what the agent
walked past *because* it was not the one walking.

**Rejection memory.** The witness reads
`workspace/witness/rejections.jsonl` and surfaces the last N entries
(N configurable, default 5) as the `{recent_rejections}` block in
`on-glean.md`. Each entry records the candidate text, an optional
agent-supplied reason, the turn the rejection happened in, and a
timestamp — written by the agent's `reject_candidate` tool (ch. 07)
during the digestion turn that asked the question. Without this
signal the witness re-surfaces patterns the agent already turned
away; with it, rejections become learning, not noise. The file is
append-only and lives in the workspace alongside `glean-log.jsonl`,
so the gate survives data_dir disposal and hand-deletion resets the
memory the same way deleting any workspace log does.

**Semantic retrieval over rejections.** Recency is a shallow signal
— it catches "you just turned this away" but misses shape-recurrences
from beyond the window. So the witness also embeds each rejection at
write time into a derived `rejection_vectors` SQLite table (ch. 10),
rebuildable from `rejections.jsonl` on startup. Before every glean it
embeds the current window text and cosine-scans the vectors, surfacing
the top-K semantically-similar past rejections as the
`{similar_rejections}` block in `on-glean.md`. This catches a
rejection from months back whose substance resembles the current turn,
in vocabulary or not. Threshold-gated (default cosine 0.60); zero-K
fully disables the read path (write-side embedding continues so a
later re-enable is instant). Recent and similar blocks dedup by
candidate id, recent winning — a rejection already in
`{recent_rejections}` is dropped from `{similar_rejections}` because
recency carries context similarity doesn't. This is the σ role
(retrieval, distinct from the π-shaped Proposer that writes glean
candidates) staged before any prompt-revision loop: the operator
authors `on-glean.md` and the engine never rewrites it.

**Queue depth cap.** At enqueue time the witness checks the queue's
current depth; at-or-above the configured cap (`max_queue_depth`,
default 5; zero disables) the candidate is dropped with a warning.
A drop does not consume refractory state — `last_glean_through`
stays where it was — so a quieter moment lets the next eligible
glean still fire. The cap bounds the worst case (a productive
session filling the next quiet stretch with weak candidates) without
silencing the witness when it has real signal.

**Refractory.** After a candidate is queued at turn T, no further
glean fires until the agent has reached turn `T + N`, where `N` is the
configured threshold (default 12 = 2 × the 6-turn glean window). The
gate is pre-model: when within refractory the witness does not even
call its model. Both wake paths — per-turn dice and the end-of-session
pass — honor the gate; "guaranteed end-of-session pass" names a *pass*
that runs, not a candidate that must be queued. The threshold is a
structural rule, not a semantic judgment: a stretch of activity that
keeps mining the same narrow band of material produces zero or one
candidate, not many, regardless of what the model would have said.
`last_glean_through` persists across restarts in
`workspace/witness/glean-log.jsonl` (one append-only entry per queued
candidate), so the gate survives even when the data_dir cache is wiped.

The witness does not glean over its own gleanings. Digestion turns
(ch. 02) — the system frames carrying a candidate plus the agent's
response to it — are excluded from the glean window, and the dice are
not rolled when the wake turn is itself a digestion. Without this
filter the witness extracts knowledge claims about the machinery of
digestion, which the next quiet trigger fires as a digestion, which
the witness extracts a more-abstract claim about, and so on; the
abstraction climbs without bound. A digestion turn carries no
world-information — its only inbound is a prior gleaning — so it is
not material for compression. The quiet gate on the agent side resets
on every digestion for the same reason: the queue must not collapse
into a sequence of back-to-back digestions the moment the silence
threshold is first crossed.

## Duty three: connecting

After every settled turn (like moves), the witness embeds the turn's
transcript and cosine-scans the workspace's vector index — the same
`segments` table that backs the agent's `search` tool (ch. 02), read
through a **no-bump** variant so per-turn retrieval does not pump
warmth into notes the agent never sees. If the top hit clears
`connect_threshold` (default 0.65) and passes the **self-connection
guard** (skip hits whose file the agent wrote to within the last N
turns; default 5), the witness composes a one-sentence why via
`on-connect.md` and posts a `ConnectFrame` to the turn loop.

The turn loop writes a `[connect]` system-role line to the referenced
turn's record and, if the turn is still in HOT, synthesises the same
line into the live window so the model sees it on its very next call.
The frame's body carries the target note's content: full file if the
target is an **atomic note** (path under `knowledge/` **and** YAML
frontmatter with an `id`), otherwise the first 200 words verbatim
with an ellipsis on truncation.

Refractory (`connect_min_new_turns`, default 6) bounds how often
frames land; the threshold is the trigger — no probability dice.
Turns that don't clear it produce no model call at all. Zero
threshold disables the duty entirely. `last_connect_through`
persists across restarts in `workspace/witness/connect-log.jsonl`
(one entry per fired frame), same discipline as glean-log.

The result is a σ role pointed at the knowledge web: the witness
noticing what already-written note the current turn resonates with,
offered inline as a record line for the agent to act on or ignore.
Digestion and heartbeat turns are excluded from connect the same way
they are from gleaning (their transcripts are not the kind of
substance connect should search against).

**No queue, no reject.** Unlike gleaning, connect surfaces without
asking for a decision. The agent may write a link, extend the target
note, or ignore the frame entirely; there is no structural machinery
to accept or reject. The connection is a fact-about-the-turn, added
to the record and left for the agent to act on or not. If the same
low-quality connection keeps surfacing, that is a σ problem for a
future measurement pass — not something the engine tracks today.

## Contracts

- **Four duties.** Moves, gleaning, connecting, and shape. The witness
  writes to the moves file, the extraction queue, its own receipt
  logs, and (as author=Witness only) rows in the derived
  `shape_vectors` table. Connect frames it produces are routed
  through the turn loop; the witness never writes `turns.jsonl`
  directly. It never writes knowledge, never speaks, never executes
  tools.
- **Shape authorship is divided.** The witness may author glosses of
  atomic notes into the derived `shape_vectors` table; it never
  writes to `knowledge/`. Agent-authored `shape:` frontmatter always
  overrides the witness's gloss and is never overwritten by the
  drift-repair worker.
- **Identity required.** Missing `workspace/witness/identity.md` fails
  gateway startup with an error naming the file. Missing event prompts
  disable their duty and log once.
- **Witness reads the record.** Move generation reads the turn's
  messages from the record file; it never consumes agent-produced
  summaries.
- **The transcript carries the speech.** Speech is a tool (ch. 01),
  so the agent's actual words live in speak-call arguments; the
  witness's transcript surfaces them as first-class speech ("you
  spoke: ..."), and other tool calls carry a truncated argument peek.
  The witness cannot compress what it cannot see.
- **A turn is never lost.** Model failure during move generation
  produces a mechanical fallback move, not a gap.
- **Move shape.** One move line per turn: turn number + summary text,
  appended to `record/moves.jsonl`. The cursor used by compaction is
  the contiguous frontier of the file's turn numbers (ch. 10), never
  stored elsewhere.
- **The record is the truth; moves are derived.** On every wake the
  witness scans for turns in the record with no move line — not just
  forward from the tail — so a hand-edited moves file (a deleted
  line) regenerates on the next settled turn.
- **Glean cadence.** Flat per-turn probability (default 0.25,
  configurable) + guaranteed end-of-session pass, both subject to a
  turn-distance refractory between queued candidates (default 12,
  configurable per agent; zero disables). The pass runs; the
  refractory decides whether a candidate is queued.
- **Rejection memory is in the workspace.** Rejections persist in
  `workspace/witness/rejections.jsonl`, written by the agent's
  `reject_candidate` tool. The witness reads the last N entries
  (configurable) before each glean and renders them into the prompt's
  `{recent_rejections}` slot. The witness's memory of what didn't
  land survives data_dir disposal and is inspectable by the agent.
- **Queue depth is bounded.** The witness drops enqueues at-or-above
  `max_queue_depth` (default 5, configurable; zero disables). A drop
  does not consume refractory state.
- **No gleaning over digestion.** Digestion turns are skipped by the
  dice and stripped from the glean window. The witness never compresses
  the machinery of its own past compressions. The quiet gate on the
  agent side resets on every digestion, so candidates wait a full quiet
  interval between firings regardless of queue depth.
- **No gleaning over heartbeats.** Heartbeat wakes (ch. 01) are skipped
  by the dice — firing a glean from the autonomy floor turns quiet time
  into more inbound. Unlike digestion, heartbeats remain *in* the glean
  window: the loom work the agent does during quiet stretches is prime
  material, so a later real turn or the end-of-session pass still
  harvests it.
- **σ retrieval over rejections.** Each rejection is embedded at write
  time into the derived `rejection_vectors` table (ch. 10); before
  every glean the witness surfaces the top-K semantically-similar past
  rejections into the `{similar_rejections}` prompt slot, deduped by
  candidate id against `{recent_rejections}`. Threshold-gated; zero-K
  disables the read path but the write-side embedding continues so
  re-enable is instant.
- **Connect fires per settled turn.** The threshold is the trigger
  (default 0.65 cosine); the refractory (default 6 turns) bounds
  frame frequency. Zero threshold disables the duty entirely. No
  probability dice — turns that don't clear it never call the model.
- **Connect surfaces; the agent acts.** No queue, no reject. A
  `[connect]` frame lands as a system-role record line on the
  referenced turn and (when the turn is still in HOT) is synthesised
  into the live window. The agent decides what to do with it during
  ordinary turns. Body policy: full file if atomic, else first 200
  words.
- **Single-writer preserved for the turn record.** Connect frames
  route through the turn loop's mpsc; the turn loop's `TurnRecord`
  remains the sole writer of `turns.jsonl`. The witness holds only
  the sender.
- **Connect log recovers refractory.**
  `workspace/witness/connect-log.jsonl` — one line per fired frame
  (turn, target_ref, at) — recovers `last_connect_through` across
  restarts, same discipline as glean-log.
- **No connect over digestion or heartbeats.** Same exclusions as
  gleaning, for the same reasons: those turns' inbound is not the
  substance connect should search against.
- **Second person.** The shipped identity seed writes the witness as
  "you"; the voice is part of the design, not a style preference.

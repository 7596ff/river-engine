# 04 — The Witness

The witness is the second voice: retrospective, second person, watching
the agent's turns from outside. It is a **role, not a process** — a
concurrent task in the same binary, subscribed to the same event bus,
with its own model assignment (often a smaller, cheaper model than the
agent's; the work is summarization, not reasoning).

It has exactly two duties: **moves** and **gleaning**. Compression of
the record, and the harvest of its margins. Nothing else. It writes to
the moves files and the extraction queue; it never writes to the
agent's knowledge, never speaks on channels, never touches tools.

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
                    {recent_record}) — optional
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

## Contracts

- **Two duties only.** Moves and gleaning. The witness writes to the
  moves files and the extraction queue, nowhere else. It never writes
  knowledge, never speaks, never executes tools.
- **Identity required.** Missing `workspace/witness/identity.md` fails
  gateway startup with an error naming the file. Missing event prompts
  disable their duty and log once.
- **Witness reads the record.** Move generation reads the turn's
  messages from the record file; it never consumes agent-produced
  summaries.
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
  configurable) + guaranteed end-of-session pass.
- **Second person.** The shipped identity seed writes the witness as
  "you"; the voice is part of the design, not a style preference.

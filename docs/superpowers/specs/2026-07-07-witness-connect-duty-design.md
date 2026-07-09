# Witness Connect Duty — Design

Status: draft 2026-07-07. Sibling of the 2026-07-07 σ-retrieval work
(`docs/superpowers/specs/2026-07-07-witness-similar-rejection-retrieval-design.md`).
Reading iris's close reading of the MetaSkill-Evolve paper
(`~/stream/notes/papers/metaskill-evolve-close-reading.md`) with her
brought this idea up: the witness has a strong Proposer (glean); it
does not have a Retriever pointed at the workspace. This spec adds one.

## Background

The witness has two duties today:

- **move** (per settled turn, always): compress the turn into one line.
- **glean** (per quiet window, probabilistic): propose a candidate
  atomic note, queued for the agent to digest.

Both duties are *outward*: they produce compressions or new candidate
claims. Neither points *inward* at the knowledge web the agent has
already built. When a turn resonates with a past note, nothing in the
loop surfaces the resonance — the flash system does an ambient
version of this (warm notes cross the threshold and surface), but
warmth accumulates over time and doesn't respond to the specific
content of a single turn.

iris's framing (from the phase-1 spec conversation): σ alone carries
weight in the paper's ablations. Retrieval-only, no proposer, no
prompt revision. Applied here it names a new duty: **connect** — the
witness reads a settled turn, semantic-searches the workspace, and if
the closest note is close enough, surfaces the connection with a
one-sentence why.

The agent decides what to do with the surface (write a link, extend
the note, ignore it). Nothing structural is written except the
surface itself.

## Goal

Give the witness a σ role pointed at the workspace: after each
settled turn, find the workspace note most semantically similar to
the turn's transcript, and if it clears a threshold, surface the
connection inline in the record so the agent sees it while the turn
is still fresh in HOT.

## Non-goals

- **No queue.** The connect duty writes surface, not candidates to
  digest. Iris's call: "we just need to surface the connection."
- **No dedicated wake.** No `Wake::Connection` turn type; no
  digestion-style framing that pulls the agent's attention.
- **No rejection log.** The surfacing is the whole action; there is
  nothing structural to reject. The agent ignores what doesn't land.
- **No graph traversal.** Phase 1 is pure semantic top-K — same
  discipline as the phase-1 σ-retrieval spec. If evidence shows
  graph-hop expansion or activation weighting would help, that is a
  follow-up.
- **No prompt revision.** `on-connect.md` is authored by the operator
  and held fixed. σ-only staging, same as the rejection-retrieval
  spec.
- **No cross-agent surfacing.** Each agent's workspace is its own.

## Concept

Once per settled turn, the witness embeds the transcript, cosine-
scans the existing `segments` table (same index that backs
`Memory::search`), and takes the top hit. If cosine < threshold, or
if a refractory / self-connection guard blocks, nothing happens.

Otherwise the witness makes a small model call: given
`(transcript, target_path, target_excerpt)`, emit one sentence that
names the connection. If the model returns the `nothing to connect`
sentinel, nothing happens.

Otherwise the witness posts a `ConnectFrame { turn, target_ref, why }`
to the turn loop via mpsc. The turn loop writes a `RecordRole::System`
line to the referenced turn's record with a `[connect]` marker, and —
if the turn is still in the live HOT window — synthesises the same
line into HOT so the agent sees it on their very next model call.

The record line is torn-line-tolerant like every other record line,
compaction eats it with the turn it annotates, and the arc's move for
the turn (if any) is uninvolved.

## Duty flow

Per settled turn N, run after the move duty in the same catch-up
scan:

1. Skip if turn N is a digestion or heartbeat turn (same exclusion as
   glean uses — the agent's inbound on those turns isn't the kind of
   material connect should search against).
2. Skip if `on-connect.md` is absent (duty disabled — same fallback
   pattern as `on-turn.md` and `on-glean.md`).
3. Refractory: if `up_to_turn - last_connect_through < connect_min_new_turns`,
   skip. Recovered from `witness/connect-log.jsonl` on load, same
   idiom as glean's refractory.
4. Load the turn's transcript (the same
   `format_transcript(&lines)` the move duty produces).
5. Embed the transcript. On embed failure: log warn, skip.
6. Cosine-scan `segments`. Take the top hit. If cosine <
   `connect_threshold`, skip.
7. **Self-connection guard:** if the hit's file was written by the
   agent within the last `connect_self_write_window` turns (checked
   against the file's most recent `on_write` bump), skip and take the
   next hit; if no hit clears both threshold and guard within the top
   K scanned, skip entirely. `K` is small (5) — this is a filter, not
   a search widening.
8. Render the compose-why prompt from `on-connect.md`, substituting
   `{transcript}`, `{target_path}`, `{target_excerpt}`.
9. Witness model call. On failure or empty: log warn, skip.
10. On sentinel (`nothing to connect`, case-insensitive): skip.
11. Post `ConnectFrame { turn: N, target_ref, why }` to the turn
    loop's connect channel.
12. Append a receipt to `witness/connect-log.jsonl`:
    `{turn: N, target_ref, at}`.

## Turn-loop injection

The turn loop owns an mpsc receiver for `ConnectFrame`s. Between
turns (or at the boundary of the next wake — the placement matters
because we want the frame visible immediately), it drains pending
frames and, for each:

1. Load the target's file body (workspace-relative from `target_ref`).
2. Compose the frame body:
   - **Format:** `[connect] turn N connects to [[target_ref]]: {why}\n\n{body}`
   - **Body policy:** full file text if the target is an *atomic* note
     (workspace path under `knowledge/` **and** has YAML frontmatter
     with an `id`); otherwise, the first 200 words of the file
     verbatim, with an ellipsis if truncated.
3. Append via `TurnRecord::append_full` as `RecordRole::System`
   attached to turn `N` (the referenced turn, not the current one).
4. If turn N is still in the live HOT window, synthesise the same
   line into HOT as an in-memory `ChatMessage` at the natural position
   for that turn's system frames, so the very next model call sees
   it.

If turn N has since compacted out of HOT (either because the arc has
folded it, or because `min_messages` pushed it out), the record line
is still appended — the connection is a fact about turn N even if the
agent didn't see it live. The HOT synthesis is the part that gets
skipped.

**Single-writer preserved.** Only the turn loop writes to
`turns.jsonl`; the witness writes to `connect-log.jsonl`. The mpsc
channel is the seam.

## Storage

- **New file:** `witness/on-connect.md`. Operator-authored prompt.
  Substitution slots: `{transcript}`, `{target_path}`,
  `{target_excerpt}`. Missing → duty disabled with a one-line info log
  (same pattern as `on-turn.md`, `on-glean.md`).
- **New file:** `witness/connect-log.jsonl`. Append-only.
  Line shape: `{turn, target_ref, at}` (ISO-8601). Recovers refractory
  state across restarts; torn lines skipped with a warn.
- **No new SQLite table.** Pure semantic search reuses `segments`.

## Config

Additions to `WitnessConfig` in `river-core::config`:

```rust
/// Cosine similarity floor for the connect duty's top-hit gate.
/// The threshold IS the trigger — no probability dice, no dice per
/// turn. Rows below this are not surfaced. Zero disables the read
/// path (witness still catches up moves; no connect calls issued).
/// Default 0.65.
pub connect_threshold: f32,

/// Refractory between fired connects, measured in turns of forward
/// movement. Prevents the connect duty from firing on every turn of
/// a topic-locked stretch and burying the record in [connect]
/// frames. Zero disables. Default 6 (matches glean's window).
pub connect_min_new_turns: u64,

/// Look-back window for the self-connection guard. If the top-hit's
/// file was written by the agent within this many turns, skip the
/// hit and try the next one (up to K=5 scanned). Zero disables the
/// guard. Default 5.
pub connect_self_write_window: u64,
```

All optional in `river.json`; defaults bind here.
`connect_threshold = 0.0` fully disables the duty at the read path.

## Prompt shape

`witness/on-connect.md` is operator-authored. The engine guarantees
these substitutions and nothing else:

| variable | contents |
|---|---|
| `{transcript}` | The same `format_transcript` output the move duty consumes |
| `{target_path}` | Workspace-relative path of the top-hit's file |
| `{target_excerpt}` | The matched segment text (from `segments.text`) |

Operator conventions the engine assumes but does not enforce:
- The prompt asks for one sentence.
- The prompt names the `nothing to connect` sentinel so the model has
  a natural non-answer.

## Rendered frame format

Appended as `RecordRole::System` to the record for turn N:

```
[connect] turn N connects to [[target_ref]]: {why}

{body}
```

- `target_ref` — the target's frontmatter id if present, else the
  filename stem (last component, `.md` stripped) — same resolution as
  the existing wikilink handling in `memory.rs`.
- `{why}` — the witness's one-sentence output, trimmed.
- `{body}` — the target's full file text if it's an atomic note
  (path under `knowledge/` **and** YAML frontmatter with an `id`),
  otherwise the first 200 words verbatim with a trailing `…` if
  truncated. "Words" counted by ASCII whitespace splits, matching
  what a human eyeballing the file would count. Verbatim: no
  model-generated summarisation on the body — that would add a
  second per-turn call the duty otherwise avoids.

Atomic definition is deliberately strict: only notes under
`knowledge/` with a frontmatter id count. Loom notes, moment files,
and other workspace `.md` files get truncated bodies to keep the
frame from swallowing HOT.

## Failure modes

| Failure | Behavior |
|---|---|
| No memory system configured | Duty disabled at load; no calls issued. |
| `on-connect.md` absent | Duty disabled at load with info log. |
| Embed of transcript fails | Log warn; skip this turn's connect. |
| No hit clears threshold | Silent skip. |
| Every clearing hit is self-written within the window | Silent skip. |
| Witness model call fails | Log warn; skip. |
| Model returns empty / sentinel | Skip. |
| Target file deleted between search and frame write | Log warn; drop the frame. |
| Turn N already compacted out of HOT | Record line still appended; HOT synthesis skipped. |
| Torn line in `connect-log.jsonl` | Skipped on load with warn; refractory recovers to the tail before the tear (same as glean-log). |
| Restart mid-catch-up | Refractory log makes the duty idempotent per turn — a re-fire on the same turn is gated by the log. |

## Contracts

- **The turn record is single-writer.** The witness never writes
  `turns.jsonl` directly; it routes through the turn loop's mpsc.
- **The threshold is the trigger.** No probability dice. Turns that
  don't clear the threshold produce no calls and no frames.
- **The receipt is written last.** As with glean's log discipline:
  the receipt lands only after the frame post succeeds, so a torn
  log line cannot describe a phantom frame.
- **No queue means no accept/reject.** The agent's actions on a
  surfaced connection (writing a link, extending a note, ignoring)
  happen through existing tools during regular turns; the engine
  does not track them.
- **Config-off is total.** `connect_threshold = 0.0` fully disables
  the duty at the read path (no embed, no scan, no model call).

## Open questions

1. **Frame placement in a settled turn's record.** Turn N's record
   already has User/Tool/Assistant/System lines in written order. The
   connect frame is appended after all of them — the tail of turn N.
   Alternatives: interleave near the User inbound, or write as a new
   line ID that sorts after the Assistant's final response. Tail
   placement is simplest and reads naturally; revisit if it looks
   wrong in HOT.
2. **HOT synthesis timing.** "The very next model call" is what we
   want. Concretely: the turn loop's mpsc drain runs at wake, before
   context assembly for the next turn. That matches the intent for
   the common case (the agent is about to be prompted anyway); if the
   agent is *not* about to wake, the frame sits in the record and
   surfaces on the next rebuild.
3. **Sentinel wording.** `nothing to connect` mirrors glean's
   `nothing to glean`. Both are conventions between the operator's
   prompt and the engine's sentinel check.
4. **Move-connect interaction.** Should a connect frame influence
   the witness's move for the same turn? Simplest: no — the move duty
   runs first, produces its summary from the transcript alone, and
   the connect frame lands after. The move for turn N doesn't
   reference the connection; whatever the agent does about the
   connection lands in later turns' moves naturally.
5. **Threshold default.** `0.65` matches the flash system's
   `semantic_threshold`. May need tuning; expected to be the first
   knob turned in the field.

## Follow-ups (not this spec)

- **Connect-rate instrumentation.** Analog to phase-2 σ measurement:
  track how often connect fires vs. how often the agent references
  the target in subsequent turns. The `P̂` analog for connection
  quality. Read-only telemetry.
- **Graph-hop expansion.** If pure semantic top-K under-fires,
  extend the retrieval with N-hop typed-link + wikilink walks from
  the semantic anchor. Ranks targets by
  `anchor_score × decay^hops`. The paper's fuller σ.
- **Activation weighting.** Multiply cosine scores by current
  activation so warm notes surface first. Cheap addition on top of
  semantic top-K.
- **Move enrichment.** If the connect frame surfaces during turn N's
  HOT window and the agent acts on it, the move for turn N could
  reference both the transcript and the connection. Only worth
  doing if evidence shows the connect frames materially change what
  turns "meant."

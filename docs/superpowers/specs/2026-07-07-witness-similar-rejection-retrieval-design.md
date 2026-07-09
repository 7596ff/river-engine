# Witness Similar-Rejection Retrieval — Design

Status: draft 2026-07-07. Extends the 2026-06-17 rejection memory work
(`docs/superpowers/specs/2026-06-17-witness-rejection-memory-design.md`),
whose "Out of scope" note deferred semantic dedup — "revisit if
evidence shows [structural rules] aren't [enough]."

## Background

The witness already has a recent-rejections rendering: the last N
entries from `witness/rejections.jsonl` are dropped into the on-glean
prompt via the `{recent_rejections}` template slot. Recency is a
useful signal but a shallow one. Two failure modes it does not catch:

- **Reoccurring shape, different words.** The agent rejected "warm
  goodnight" three weeks ago as "not a claim." A similar exchange
  today is well past the recent window, and the witness has no cue.
- **Similar-in-content, distant-in-time.** Rejections about the
  witness's own machinery, meta-gleans, warm passages — categories
  that keep coming back — vanish from `{recent_rejections}` after N
  turns and rehydrate into a fresh candidate.

Reading the MetaSkill-Evolve paper (arXiv:2607.05297) with iris made
the shape of the fix explicit. The paper's Retriever `σ` surfaces
inspirations semantically matched to the current failure. Iris flagged
that `σ` alone carries weight in the paper's ablations (§4.3) and
proposed staging it before any prompt-revision work: get retrieval
right first, then decide whether to revise the operator.

This spec is that first stage. Retrieval only; no prompt revision.
The `on-glean.md` prompt stays as-is except for one new template
slot. No sovereignty question arises because no operator is rewritten.

## Goal

Before the witness proposes a glean candidate, surface the past
rejections most semantically similar to the current glean window.
Render them into the prompt so the witness has a cue when it is
about to re-propose something the agent has already turned away —
even if the earlier rejection is outside the recent window.

## Non-goals

- **Prompt revision.** `on-glean.md` is not rewritten. Only the
  `{similar_rejections}` slot is added.
- **Auto-skip.** Retrieval surfaces; the model still judges. No code
  path decides "similar enough → drop" on its own.
- **Accepted-note retrieval.** iris's original point still holds: the
  workspace itself is the positive signal. Only rejections are indexed
  here.
- **Cross-agent retrieval.** Each agent's rejections stay in that
  agent's memory. No shared archive.
- **Meta-productivity metric.** Rejection-rate instrumentation (the
  natural `P̂` analog for gleaning) is a follow-up spec. This one
  changes retrieval only; the measurement comes next.

## Concept

Rejections are embedded at write time and stored in a new SQLite table
alongside the existing vector segments. At glean time, before the
model call, the witness embeds the current glean window text, cosine-
compares it to the rejection vectors, takes the top-K above a cosine
threshold, and renders them into the prompt's new
`{similar_rejections}` slot.

`{recent_rejections}` stays. Recency and similarity answer different
questions:

- `{recent_rejections}`: "what did I recently turn away, in any
  domain" — style, tone, dispositional cue.
- `{similar_rejections}`: "what did I turn away that resembles what
  I'm looking at right now" — pattern-match cue.

## Storage

One new table in the existing per-agent memory database:

```sql
CREATE TABLE rejection_vectors (
  candidate_id TEXT PRIMARY KEY,     -- ULID from rejections.jsonl
  turn         INTEGER NOT NULL,
  candidate    TEXT NOT NULL,        -- verbatim candidate text
  reason       TEXT,                 -- agent-supplied reason, if any
  at           TEXT NOT NULL,        -- ISO-8601, mirrors the jsonl entry
  embedding    BLOB NOT NULL         -- packed f32, dim = embedding model's
);
CREATE INDEX rejection_vectors_turn_idx ON rejection_vectors(turn);
```

`candidate_id` is the same ULID the reject-candidate tool already
writes into `rejections.jsonl`, so the jsonl file and the vector table
share a key. The jsonl stays authoritative (ground-truth-vs-derived,
wall ch. 10); the vector table is derived and rebuildable.

**Rebuild rule.** A fresh `memory.db` reconstructs `rejection_vectors`
by scanning `witness/rejections.jsonl` and embedding each entry once.
This runs at startup only if the table is empty or its row count is
strictly less than the jsonl's non-torn line count. Torn lines are
skipped with a warning (same tolerance as elsewhere).

## Write path

`reject_candidate` (in `tools.rs`) already appends one line to
`witness/rejections.jsonl`. It now also enqueues an embed-and-insert
job through the memory system.

- The tool returns as soon as the jsonl append + fsync succeed
  (unchanged). The vector insert is best-effort and happens on the
  memory system's async worker.
- On embed failure: log a warning, do not retry inline. The rejection
  remains in the jsonl and will be picked up by the next startup
  rebuild pass or by an on-demand recovery scan.
- On duplicate `candidate_id` at insert time: log and no-op. This
  guards against the rebuild racing with a live write.

**Why write-time embedding.** Doing this at glean-time would add
latency to every glean and re-embed the same rejection dozens of
times. Write is once-per-rejection; reads are per-glean.

## Read path

Inside `Witness::glean`, after the window is assembled and before the
model call:

1. Compute the query text: the same string that will be substituted
   into `{recent_record}` (the recent-turn transcript plus, if
   present, the recent moves block).
2. Skip retrieval entirely when the memory system is `None` (no
   embedding model configured) — the slot renders empty.
3. Embed the query text (one call).
4. Cosine-compare against every row of `rejection_vectors`. Full-scan
   is fine at expected volumes (hundreds to low thousands of rows over
   an agent's lifetime); revisit if evidence forces it.
5. Take the top-`similar_rejections_top_k` rows with cosine similarity
   ≥ `similar_rejections_threshold`. Ties broken by newer `turn` first.
6. Render into `{similar_rejections}` (format below).
7. On embed failure at read time: log a warning, render empty. Never
   fail the glean.

**Overlap with `{recent_rejections}`.** A rejection may show up in
both slots. Dedup is by `candidate_id`: if a rejection is already in
the recent list, it is skipped in the similar list. Recent wins,
because it carries "you saw this recently" context the similar slot
doesn't.

## Prompt integration

Two additions to `witness/on-glean.md`:

- One new template variable `{similar_rejections}` (empty string when
  the slot has nothing).
- The operator's freeform prose introducing the section — not our
  business; the operator writes it. The engine only guarantees the
  substitution.

Rendered block shape when non-empty:

```
[your prior gleans, semantically similar to what you're looking at now]
turn 42 (sim 0.81): "warm goodnight" — reason: not a claim
turn 118 (sim 0.74): "the pattern of enqueue-before-log" — reason: meta-mining
...
```

Empty rendering: empty string, same convention as
`{recent_rejections}`, so the operator's surrounding label can sit
alone on day one without looking broken.

## Config

Additions to `WitnessConfig` in `river-core::config`:

```rust
/// Top-K semantically similar past rejections to render into the
/// witness's on-glean.md `{similar_rejections}` slot. Zero disables
/// the slot. Default 5.
pub similar_rejections_top_k: usize,

/// Cosine similarity floor for similar-rejection retrieval; rows
/// below this are not considered. Default 0.60.
pub similar_rejections_threshold: f32,
```

Both are optional in `river.json`; defaults bind here. Setting
`similar_rejections_top_k = 0` fully disables the read path — no
embed of the query text, no scan. The write-path embedding continues
so a later re-enable has data.

## Failure modes

| Failure | Behavior |
|---|---|
| No embedding model configured | Slot renders empty; write path skipped. |
| Query-side embed fails | Log warn; slot renders empty; glean proceeds. |
| Write-side embed fails | Log warn; jsonl entry unaffected; recover on next startup scan. |
| Vector table torn / db missing | Rebuild from jsonl at startup; missing db already triggers full memory rebuild (wall ch. 10). |
| Torn jsonl line during rebuild | Skipped with warning; that rejection is unretrievable until manually restored. |
| Duplicate insert | Log and no-op. |
| `rejection_vectors.embedding` dim mismatch (embedding model changed) | Log warn; discard mismatching rows and re-embed from jsonl. |

## Contracts

- **Rejections file is authoritative.** `witness/rejections.jsonl`
  stays the ground truth. The vector table is derived and always
  rebuildable from it.
- **Read path is best-effort.** Retrieval failure never blocks a
  glean. A failing embedder degrades to the pre-spec behavior:
  `{recent_rejections}` only.
- **No operator revision.** This spec adds no code path that
  modifies `witness/on-glean.md` or any other prompt file.
- **Ground judges deploys.** No new automation acts on the retrieval
  output. Every action taken because of a similar-rejection cue is
  taken by the model, in-context, per glean.
- **Config-off is total.** `similar_rejections_top_k = 0` disables
  the entire read path with no downstream effects on the rest of the
  witness.

## Open questions

1. **Query granularity.** The query text as specified is the whole
   `{recent_record}` block. An alternative: embed each turn line
   separately and take the max-similarity per rejection. Whole-window
   is simpler and matches how the witness reads; per-line might
   surface more sharply. Start with whole-window.
2. **Threshold default.** 0.60 is a guess. If it under-fires, drop to
   0.50. If it over-fires with generic language matching, raise. The
   knob exists for exactly this tuning.
3. **Rebuild-at-startup cost.** Embedding an entire lifetime of
   rejections at first launch could be slow. If it becomes an issue,
   move the rebuild to a background task that populates rows as it
   goes; retrieval simply misses the un-embedded ones until then.
4. **Reason field in similarity.** Currently the embedding is over
   the candidate text only. Including the reason ("meta-mining") might
   help retrieval find category-matches. Try candidate-only first; if
   it misses meta-category rejections, concatenate reason after a
   separator and re-embed.

## Follow-ups (not this spec)

- **Rejection-rate instrumentation.** The stage-2 measurement: track
  rejection rate over a rolling window as the `P̂` analog for glean
  productivity. Read-only; no action taken on it.
- **Operator revision loop.** Stage 3, only if stage 2 shows a signal
  worth acting on. A slow loop that drafts revisions to
  `witness/on-glean.md`; drafts land in a review path, ground
  approves before deploy. External-judge invariant preserved.
- **Silence gate for σ.** A σ-side analog of the refractory: if
  retrieval surfaces a rejection with cosine > 0.95 within the last
  N turns (iris's proposal: 3), skip the glean entirely rather than
  proposing a near-duplicate the agent will reject again. Called out
  as a non-goal for this spec because auto-skip is a code path that
  acts on the retrieval output, and the σ-only staging deliberately
  avoids that. Natural after stage-2 measurement shows which
  rejections are high-similarity repeats — the threshold and window
  can be picked from data instead of guessed. Thresholds live in
  `WitnessConfig`, disabled by default.

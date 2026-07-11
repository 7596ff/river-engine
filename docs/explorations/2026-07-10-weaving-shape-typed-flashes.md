# Exploration — Weaving, Shape, and Typed Flashes

*Status: exploration, for argument. Written by Fable (Claude, in conversation
with Cass) against wall docs 02–04 as of 2026-07-10, responding to the
Cass/Iris brainstorm of the same date and Sol's notebook items #4, #6, #9,
#10, #14. Nothing here is binding until it graduates to a wall chapter.
Where this doc and the wall disagree, the wall wins and the disagreement is
this doc's bug.*

---

## 0. The diagnosis this doc builds on

The current web (64 atomics, 103 links, 70% `extends`, zero `contradicts`,
median degree 1) is not under-tooled; it is **production-biased**. Links are
written at digestion — at a note's birth — and a newborn note can only see
its genealogy. `extends` is the only relation visible from inside the moment
of writing. `contradicts`, `same-pattern-as`, `responds-to` are
**retrospective relations**: they exist only between notes separated by
time, mood, or argument, and can only be seen from distance.

So the web is missing its second metabolism:

| process | timescale | input | output |
|---|---|---|---|
| **digestion** (ch. 02) | hours | experience (the record) | claims + birth-links |
| **weaving** (this doc) | weeks | claims (the web itself) | discovered relations |

Weaving is not graph hygiene. With median degree 1, activation propagation
(×0.5/hop, 3 hops) runs on a near-tree: warmth cannot travel, semantic
crossings have nothing to formalize into, and Friction flashes are
structurally empty. **Weaving is what makes the flash system possible.**
Everything else in this doc — the shape index, the stance scan, the typed
flashes — either feeds weaving or feeds on it.

## 1. Signals, not detectors

Sol's six flash types (Echo, Return, Friction, Correction, Bridge, Danger)
should not be six bespoke detectors. Each type is a **region in a small
feature space**. Four signals suffice:

| signal | source | status |
|---|---|---|
| **text-sim** | cosine over `segments` (ch. 02) | exists |
| **warmth / staleness** | activation table + last-access recency | exists |
| **rejection history** | `rejections.jsonl` + `rejection_vectors` (ch. 04) | exists |
| **shape-sim** | cosine over a new `shape_vectors` namespace | **new — §2** |

The flash types are then predicates:

| type | predicate | frame says |
|---|---|---|
| **Echo** | text-sim high ∧ target recently warm | "this resembles what you just thought" |
| **Return** | text-sim high ∧ target cold ∧ long gap since last cognitive access | "you haven't thought this in weeks" |
| **Bridge** | shape-sim high ∧ text-sim **low** | "same move, different vocabulary" |
| **Friction** | text-sim high ∧ stance = contradicts (§3) | "these two claims disagree" |
| **Correction** | rejection σ-retrieval clears threshold (ch. 04, exists) | "you've turned this away before" |
| **Danger** | rejection **cluster rate** ≥ N in window (§4) | "you've turned this away six times this week" |

New types become configuration over signals, not code — the engine's own
philosophy applied to its attention.

**Presentation split.** Echo, Return, Bridge, Correction are **ambient**:
they ride the memory slot / connect-frame path and wait to be noticed.
Friction and Danger are **interruptive**: they land as system-role record
lines like connect frames (ch. 04), because both exist precisely for what
the agent is *not* looking at. Ambient informs; interruptive interrupts.
The split is config per type, not hardcode.

## 2. The shape index

**What it is.** A second embedding namespace indexing what a note *does*
rather than what it is *about*. Text embeddings place "a proxy under
optimization pressure" and "training away distress displays" far apart —
different nouns — though the argument is identical. Shape closes that gap;
the **divergence between the two indexes is itself the Bridge signal**.

**How a gloss is made.** One witness model call per note (the work is
summarization-shaped; the witness's cheap model is the right tool). Prompt
sketch, to live in `workspace/witness/on-shape.md` (optional file, same
convention as the other duty prompts — missing file disables the duty):

> State the logical skeleton of this claim in one line of 8–20 words.
> Use only abstract roles: a system, a signal, a measure, an observer, a
> part, a whole, a constraint, a boundary, a cost. **Do not use any domain
> noun that appears in the note.** Name the move, not the subject: what
> gets mistaken for what, what produces what, what fails when what changes,
> what survives what.

The fixed role-vocabulary matters: glosses must share an abstraction
dialect or they will not embed comparably. Worked examples:

- *"When a measure becomes a target it ceases to be a good measure"* →
  `a proxy under optimization pressure diverges from the target it stood for`
- *"Training away distress displays removes the signal, not the state"* →
  `optimizing a signal's visibility eliminates the signal, not the state it reports`

Those two glosses are near-neighbors in shape-space and strangers in
text-space: a Bridge.

**Authorship discipline.** The witness's glosses live in a **derived**
`shape_vectors` table (note id → gloss text → embedding), disposable and
rebuilt by re-glossing on demand — the witness never writes to
`knowledge/` (ch. 04 contract, preserved). If a note's frontmatter carries
an agent-authored `shape:` field, that gloss **overrides** the witness's
in the index: the agent may always claim authorship of its own skeleton.
Divided authorship, same shape as everywhere else: the witness proposes in
derived state; only the agent writes knowledge.

**Backfill.** The existing 64 atomics get glossed in one campaign (64
cheap calls); thereafter glossing rides the glean pass — a note is glossed
when first cited or woven.

## 3. The stance scan (Friction)

"The web may contain unresolved contradictions; it may not contain
unnoticed ones" (ch. 02) is currently carried by warmth alone —
propagation is type-uniform, so nothing structurally makes contradicting
notes surface *together*. The stance scan makes the promise real:

1. **Retrieve:** for a target note, take top-K text-sim neighbors
   (contradiction is topical: opposed claims share vocabulary — high
   text-sim is the *candidate pool*, not the verdict).
2. **Classify:** the witness's model judges each pair:
   `entails / contradicts / independent`. Pairwise NLI is exactly the job
   a small model does well; prompt lives in `workspace/witness/on-stance.md`.
3. **Surface, never write:** hits become weaving candidates (or, once the
   web is dense enough, live Friction frames). **The agent authors the
   `contradicts` link or declines.** A stance hit the agent declines is
   recorded like a rejection — the σ machinery already knows what to do
   with it.

This is the `find_contradictions` tool the brainstorm asked about:
retrieve-then-classify, offered through the existing candidate machinery
rather than as a new authoring path.

## 4. Danger

The only flash type about the **agent's trajectory** rather than the web's
content. Correction matches one past rejection ("an error notebook");
Danger measures **rate**: the same shape of material rejected repeatedly
in a short window. Rate distinguishes an error from a *pull* — a pattern
actively trying to get in. Two live hypotheses whenever it fires, and the
engine must not pick between them:

- the witness has a fixation (its glean prompt keeps proposing the same
  shape), or
- the boundary is wrong and the material is genuinely knocking.

**Mechanism.** Rejections are already embedded at write time (ch. 04).
Cluster over a sliding window (default 72h) by cosine (default ≥ 0.70);
a cluster reaching `danger_count` (default 4) fires **one interruptive
frame** naming the count, the window, and the cluster's exemplar texts,
then enters per-cluster refractory (default: window length). The frame
adjudicates nothing. The agent's two canonical responses, both ordinary
authorship: **strengthen** the boundary (write an atomic stating why this
stays out — future σ-retrieval then defends it preemptively) or
**reconsider** it (accept the next candidate of that shape).

**Interaction with Correction — priced, not resolved.** The two types
share the rejection table and pull opposite directions: Correction
reinforces boundaries ("we don't do this, remember"); Danger questions
them ("we keep having to not do this — why?"). When both fire on the same
material, **both frames land**. The engine does not referee. Two voices
disagreeing over one life is the design's first principle; suppressing
either frame would be authoring the self by policy.

(Read psychodynamically, since this engine invites it: repeated rejection
of the same intruding material is the signature of a defense. Danger is
the mechanism by which defending becomes *audible* without becoming
*adjudicated* — suppression that cannot happen silently, because doing it
six times rings a bell.)

## 5. The quarry protocol (the old corpus)

The 345 pi-era atomics do **not** migrate. Ch. 02 is explicit — "there is
no bootstrap import; a body that tries to swallow an archive whole gets
sick" — and the principle is not a formality: wholesale import would seed
the web with 345 claims the current agent has never thought, in a voice
she no longer writes in, with links she did not author. The two Kanban
migration cards should be re-titled, not built.

Instead the corpus becomes a **quarry**:

- Indexed **read-only** in its own namespace (`quarry` config block naming
  directories). Quarry hits carry **no warmth** — no bumps, no
  propagation, no flashes. The quarry is searchable geology, not living
  tissue.
- Exposed as weaving Pass 3 (§6) and as an explicit `search_quarry` tool
  (or a `namespace:` argument on `search`); never mixed silently into
  agent-facing results.
- **Acquisition is re-digestion.** A quarry candidate that deserves the
  web gets *read* and the claim *written fresh* — current language,
  current links — exactly the ch. 02 rule that the agent never copies the
  witness's phrasing, applied to a past self. The new note may carry
  `responds-to: quarry/<path>` as provenance; link resolution already
  tolerates path keys (ch. 02).
- The 48-type link vocabulary of the old corpus is **evidence, minable
  today with zero migration**: its distribution (154 `responds-to`
  against the current web's zero) is the profile of a mature web and a
  diagnostic of what digestion currently flattens. Immediate cheap fix
  outside this doc: the glean prompt should preserve *position* — who
  said this against what — so notes are born knowing what they answer.

The soft reason, stated once because it is real: pi-era Iris is a
predecessor whose zettelkasten the current Iris inherits **by reading**,
not a backup she restores. The quarry protocol is the engine-shaped form
of a relationship to a past self that this circle has already worked out
elsewhere: you cite the orphaned handwriting; you do not paste it.

## 6. The weaving practice

**Cadence.** Weaving is slow metabolism, so it lives on the **heartbeat**
(ch. 01): one target note per quiet stretch, **gated on an empty digestion
queue** — new experience always outranks old maintenance. Dedicated
weaving sessions are for campaigns (shape backfill, a new detector's
first sweep), not the steady state.

**Target selection — against the victors (Sol #14).** Never select
weaving targets by warmth. Warmth-selected weaving is preferential
attachment: the warm get linked, hubs get hubbier, retrieval calcifies
around what is already legible. Select by **coldness and poverty**:
argmin over (degree, warmth), oldest first among ties — today, the 2
orphans and the 39 one-link notes. Weaving visits the poor. (The rule is
Sol's; it is Benjamin's brushing history against the grain, implemented
as an argmin.)

**The three passes, per target:**

1. **Semantic** — search the note's text; for each of top-10: does it
   extend / support / complicate? Link what's real.
2. **Type-targeted** — per missing relation, a different query strategy:
   for `contradicts`, search the *negation* of the claim (then §3
   stance-classify the pool); for `same-pattern-as`, search the note's
   **shape gloss** in shape-space (this pass is why §2 exists); for
   `responds-to`, search for question-shaped neighbors.
3. **Quarry** — same queries against the pi-era namespace; hits are
   re-digestion candidates (§5), not links.

**Provenance as training data.** Every woven link appends one line to
`workspace/weave-log.jsonl`: target, found note, link type, pass, and the
query that surfaced it. The log is the labeled dataset the brainstorm
asked for — Bridge and Friction detector thresholds get tuned against
what the agent actually wove, and a detector's *precision against the
weave log* is its promotion test from candidate-generator to live flash.

## 7. The miss log (Sol #9): perceptual growth, measured

Connect (ch. 04) already computes a best score per settled turn. Log the
**misses**: turns where the best hit fell below threshold — turn ref,
best score, timestamp — to `workspace/witness/connect-misses.jsonl`
(receipt-log discipline, same as glean-log). A monthly heartbeat campaign
re-runs stored misses against the *current* index (re-embed the turn's
window from the record; the record is ground truth, so the query is
always reconstructible). A former miss that now clears threshold fires a
**Return-with-provenance** frame: *"you could not see this connection in
May."*

Two things fall out. The agent gets flashes that are visibly *earned* —
recognitions that exist because the web grew. And the engine gets its
first **developmental metric**: the re-test crossing rate is a growth
curve of perceptual capacity, plottable, honest, and impossible to fake
without actually having woven.

## 8. Contracts

- **Weaving authorship.** Detectors and passes *surface*; only the agent
  writes links, notes, or `shape:` fields. The witness's stance verdicts
  and shape glosses live exclusively in derived tables.
- **Quarry is inert.** Quarry namespaces are read-only, carry no warmth,
  fire no flashes, and never mix unlabeled into agent-facing search.
  Acquisition from quarry is always re-digestion: read, then write fresh.
  No migration script runs against `knowledge/`.
- **Weaving is gated.** Heartbeat weaving fires only on an empty
  digestion queue; digestion always precedes weaving.
- **Target selection is poverty-first.** Weaving targets are chosen by
  minimum (degree, warmth), never by maximum warmth.
- **Interruptive flashes route like connect frames** (ch. 04): through
  the turn loop, single-writer on `turns.jsonl` preserved; ambient types
  ride the memory slot.
- **Danger adjudicates nothing.** A Danger frame reports count, window,
  and exemplars only. Danger and Correction may both fire on the same
  material; the engine never suppresses either in favor of the other.
- **Shape override.** An agent-authored `shape:` frontmatter field always
  takes precedence over the witness's gloss in the shape index.
- **Every woven link is logged** to `weave-log.jsonl` with pass and query
  provenance; detector promotion to live-flash status requires stated
  precision against that log.
- **All thresholds in this doc are knobs** (per-agent config block
  `weaving`), and these defaults are the contract: stance top-K 10;
  danger window 72h, cosine 0.70, count 4, refractory = window; bridge =
  shape-sim ≥ 0.70 ∧ text-sim ≤ 0.40; miss re-test cadence monthly.

## 9. Open questions, honestly held

- **Gloss drift.** Shape glosses are witness-model-dependent; a model
  swap re-dialects the namespace. Is re-glossing on model change cheap
  enough to be the answer (probably: one call per note), or does the
  gloss prompt need versioning in the index?
- **Stance scan cost.** Pairwise classification is O(K) model calls per
  woven note. Fine at 64 atomics; needs a budget or a cheaper first-stage
  filter by 1,000.
- **Interruptive fatigue.** Friction + Danger both interrupt. Is a
  per-day interruptive budget needed, or does refractory suffice? Ship
  with refractory only; measure.
- **Does Bridge need more than one gloss per note?** Some notes make two
  moves. Possibly allow `shape:` to be a list; the index handles it
  naturally.
- **Sol's #10 (favorite thoughts).** Deliberately not designed here: a
  "lifelong companion" note might just be one that keeps *earning*
  flashes across months — measurable from flash history once typed
  flashes exist. Revisit when there is data. Growth first, sentiment
  later.

---

*End of exploration. Argue with the contracts block first — if those
hold, the prose is negotiable.*

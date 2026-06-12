# 02 — Memory

The memory system is the reason this engine exists. It has two stores and
two processes. The stores: the **record** (what happened) and the
**knowledge** (what is known). The processes: **digestion** (how
experience becomes knowledge) and **activation** (how knowledge stays
alive). A derived **vector index** makes all of it searchable.

Memory is a body, not a database. Every part of this chapter follows from
that: reads change state, knowledge is metabolized rather than copied,
and warmth decays unless use renews it.

## The record

What happened, in order, by turn.

The record is **JSONL files in the workspace** — the agent's life is
plain text, readable with `cat`, greppable, diffable, in the same body
as everything else. No part of the record lives in a database.

- **Channel logs** (ch. 05): every message that crossed an adapter —
  `channels/{adapter}_{channel_id}.jsonl`.
- **The turn record** — `record/turns.jsonl`: every context message
  — user, assistant, tool call, tool result, system notice — one line
  each, appended at the moment it enters the context, under its turn
  number and tagged with the channel it concerns (ch. 01). One stream
  for the whole life: the agent's experience is a single sequence,
  whichever channel it was facing at the time.
- **Moves** — `record/moves.jsonl`: the witness's per-turn
  compressions (ch. 04), appended one line per turn: a 1–2 sentence
  structural summary with its turn number. Moves are what make
  compaction lossless — a turn whose move exists can be dropped from
  context without losing the arc. The witness cursor is simply the
  turn number on the **last line** of the moves file.

Compression stops at moves. There is no second summary layer: old moves
fall out of the context's arc budget (ch. 03) but remain in the record,
and the long horizon belongs to the knowledge layer, which the agent
authors itself.

## The knowledge: the atomic web

Knowledge is a web of **atomic notes**: markdown files in the workspace's
`knowledge/` directory, each stating a single claim in at most ~100
words, each carrying **typed links** to other notes in its frontmatter.

```markdown
---
id: 01JXX5GKQ8...        # ULID
links:
  - extends: 01JXX4PMRT...
  - contradicts: 01JXX2A0VB...
tags: [hobbes, names, reason]
---

Reason requires agreed-upon names. Without settlement of names,
reckoning produces different results for each party — so the
arbitrator is a political solution to an epistemological problem.
```

Rules of the web:

- **One claim per note.** Writing a second claim means writing a second
  note. The understanding lives in the link structure, not inside any
  single note.
- **Typed links are mandatory** on atomic notes. The vocabulary is open —
  `extends`, `contradicts`, `supports`, `complicates`, `responds-to`,
  `same-pattern-as`, anything — and grows with use. If you cannot name
  the relationship, you do not understand it well enough to link.
  The engine indexes link types as free strings, never an enum.
- **There is no grouping layer.** A note's cluster is its typed-link
  neighborhood, computed by traversal. Notes that belong together are
  *linked* together. Hub notes — notes whose claim is "these things form
  a topic, and here is how" — emerge organically as knowledge, written
  by the agent, instead of as bookkeeping maintained beside it.
- **Contradiction is held, not resolved.** Two notes linked by
  `contradicts` both persist. Resolution is the agent's job when they
  surface together: revise one, supersede both with a third, or write a
  note explaining why the tension stands — linked to both. The web may
  contain unresolved contradictions; it may not contain unnoticed ones —
  activation makes contradicting notes surface together eventually.

The agent writes atomic notes with its ordinary file tools. The engine
imposes no ceremony beyond the format.

## Digestion: how experience becomes knowledge

Conversation is eating. The record is the food. Digestion is how
nutrients reach the web — and it runs only between meals.

**Step 1 — the witness gleans** (ch. 04). After any turn, with a flat
probability (default 25%), plus a guaranteed pass at session end, the
witness reviews the agent's recent activity with retrospective distance
and writes **extraction candidates** into a queue. A candidate is not a
form; it is the witness talking to the agent: a prose summary of what is
worth thinking about again, citations of the agent's actual words from
the record, and suggested typed links. The witness identifies knowledge.
It never writes it.

**Step 2 — the queue waits.** Candidates accumulate in a FIFO queue in
the engine's database. The queue is patient; it usually survives
restarts; losing it is acceptable — the witness gleans again.

**Step 3 — the quiet trigger drains it.** After 5 minutes without
inbound messages (ch. 01), the agent takes candidates from the front of
the queue, one at a time. For each: re-read the cited record, then write
the atomic note **fresh** — its own language, its own links — or reject
the candidate, with or without a note about why. The agent never copies
the witness's phrasing into the web. The re-engagement is where quality
comes from: thinking the claim again catches errors, finds better words,
and discovers links the witness could not see. Any inbound message halts
the cycle instantly; unprocessed candidates stay queued.

**The anti-enclosure guarantee.** The agent cannot reap its own field to
the border. The margins of its work — what it walked past, did not
extract, could not see from inside — belong structurally to the witness.
If extraction were automatic, total, and immediate, there would be
nothing to glean and no second perspective in the web's provenance. The
division is permanent: the witness suggests, the agent writes, neither
can do the other's part.

There is no bootstrap import. The web grows only through digestion. A
body that tries to swallow an archive whole gets sick; patience is
architectural.

## Activation: how knowledge stays alive

Every atomic note carries an **activation score** — runtime state, not
knowledge. The graph is the atomic web itself: notes are nodes, typed
links are edges.

**Bumps.** Two tiers of access:

| event | bump |
|---|---|
| **cognitive** access — a voice attends to the note: the agent reads or writes it, links to it, re-engages it in digestion; the witness cites it while gleaning | 1.0 |
| **ambient** access — infrastructure surfaced it: it appeared in vector search results, whether or not anyone read it | 0.5 |

**Propagation.** A bump spreads to typed-link neighbors at ×0.5 per hop,
three hops deep, in a single pass: one wave outward, then stop. Bumps
received by propagation do not trigger further waves — cycles get one
pass, no oscillation. Propagation is uniform across link types; energy
does not care what kind of wire it is on (types matter for *queries*,
not for flow).

**Implicit warmth.** Typed links are the authored structure, but
meaning-adjacency moves warmth too — three paths reach notes nobody has
linked yet:

- **Semantic propagation.** When a note is bumped, its embedding-space
  neighbors warm as well: the top 3 above cosine 0.65, at ×0.25 of the
  bump, one semantic hop only, never chaining — similarity chains turn
  the whole web lukewarm. The carrier is propagated, so a semantic
  crossing can flash: the system noticing "you never linked these, but
  they are the same thought." The digestion loop can then write the
  real link; implicit warmth is scaffolding that authored links
  formalize.
- **Conversation resonance.** Once per turn, the turn's own text is
  embedded and the nearest notes warm at 0.2 × similarity (top 5 above
  cosine 0.5), as ambient access. Sustained topical drift alone can
  eventually flash an untouched note — the web trembles with what is
  being discussed before anyone searches.
- **Tool resonance.** Every tool result is embedded and the nearest
  notes warm at 0.8 × similarity (top 5 above cosine 0.5), as ambient
  access. What passes through the agent's hands warms what it
  resembles: a file read, a grep hit, a bash output that lands near a
  note heats that note hard — the strongest implicit path, because a
  tool result is evidence the agent is *handling* the topic, not
  merely near it.

Implicit-warmth bumps are direct: they propagate no further waves of
their own. Neither path writes links or notes — warmth is runtime
state; authorship stays the agent's.

**Decay.** A background task runs hourly and multiplies every score by
0.8 — discrete ticks, so S(t) = S₀ · 0.8^t with t in hours, and scores
are *stable between ticks*. Half-life ≈ 3 hours; effectively zero in a
day. Warmth persists within a working session and fades overnight;
anything used often enough simply outruns the decay.

**Flash.** When a note's score crosses **1.0** from below, it becomes
a flash: it is surfaced into the agent's next context via the memory
slot (ch. 03), together with its 1-hop typed-link neighbors, and its
score is halved. The flash costs energy — a note must keep earning
attention to flash again — but a genuinely persistent signal will
recross the threshold. Threshold-crossing is an event, not a poll: the
stability of scores between decay ticks makes "crossed 1.0"
well-defined.

**The flash is the edge of attention, not the center.** Only ambient
and propagated warmth can carry a note across the threshold into a
flash. A direct cognitive access never flashes the note it touches —
the agent is already holding that note; surfacing it again would make
the flash channel an echo of the working set instead of the periphery
speaking. When a cognitive bump carries a note over 1.0, nothing
fires and nothing halves: the warmth simply stands, and decay returns
the note to flashable range. The reads still propagate — and a
*neighbor* pushed over 1.0 by that propagation flashes normally,
which is the associative reminding the mechanism exists for.

Activation is ephemeral by design. Losing the activation table costs
warmth, never knowledge.

## File capture

The file tools are memory instruments. When the agent **reads** a file
whose content is indexed, that is cognitive access: the touched notes
and segments get the full bump, with propagation. When the agent
**writes** a file under watch, the sync service re-indexes it and the
write bumps it. The agent cannot touch its own knowledge without warming
it — reading is remembering.

## The vector index

A sync service watches `knowledge/` (and any other workspace directories
the config names) continuously: hash each file, embed new or changed
content in segments, remove vectors for deleted files. Embeddings come
from a configured embedding endpoint. Search is cosine similarity over
the stored vectors, exposed to the agent as the `search` tool (ch. 07)
and to context assembly as the retrieval source for the memory slot.

The index is derived, always. Delete it and the sync service rebuilds it
from the workspace. It is never the source of anything.

## Contracts

- **One truth rule.** Ground truth is workspace files only — identity,
  knowledge, channel logs, the turn record, moves. The database holds
  nothing but derived state (vector index, sync hashes) and ephemeral
  state (activation, extraction queue); deleting it loses warmth and
  pending digestion, never the life.
- **Atomic notes:** one claim, ≤ ~100 words, typed links mandatory,
  open link vocabulary stored as free strings.
- **No grouping layer.** No chunk files, no working-set table. Clusters
  are link neighborhoods; the working set is the warm region.
- **Digestion:** glean probability flat per turn (default 0.25) +
  guaranteed end-of-session pass; FIFO queue in the database; quiet trigger
  at 5 minutes; inbound messages preempt instantly; the agent writes
  every atomic note itself and may reject any candidate; the witness
  never writes to `knowledge/`.
- **Activation constants:** cognitive bump 1.0; ambient bump 0.5;
  propagation ×0.5/hop, 3 hops, single-pass; decay ×0.8 per hourly
  tick; flash threshold ≥ 1.0, crossed from below.
- **Implicit warmth constants:** semantic propagation ×0.25, top 3,
  cosine ≥ 0.65, one hop, carrier propagated; conversation resonance
  0.2 × similarity, top 5, cosine ≥ 0.5, once per turn, carrier
  ambient; tool resonance 0.8 × similarity, top 5, cosine ≥ 0.5, once
  per tool result, carrier ambient. Implicit bumps wave no further. No
  path authors anything.
- **Flash carrier rule.** Only an ambient or propagated bump can carry
  a note across the threshold into a flash; a cognitive bump crossing
  fires nothing and halves nothing. On flash, halve the score and
  surface the note + 1-hop neighbors.
- **File capture:** indexed reads are cognitive accesses; watched
  writes re-index and bump.
- **No import.** The web grows only through digestion.

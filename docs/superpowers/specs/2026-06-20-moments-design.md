# Moments — agent-authored compression for the arc

## Goal

Give the agent a first-class way to compress its own life-in-progress, in its own voice, with its readings overriding the witness's where present. The witness keeps producing moves (mechanical, evidentiary); the agent writes moments above them (interpretive, authored). The arc shows moments where they cover; hot is never replaced; the lossless guarantee (wall ch. 03) is unchanged.

## Concept

A **moment** is a markdown file under `record/moments/` with YAML frontmatter declaring a turn range. At arc-build time, any turn covered by a moment has its witness move suppressed; the moment body is rendered in place. Overlapping moments stack — the agent can re-read the same stretch iteratively. The witness's moves remain in `record/moves.jsonl` as evidence and continue to gate the cursor.

The agent authors moments via the `create_moment` tool and looks back at the witness's reads via `read_moves` to pick what to compress.

## Storage

```
record/moments/
  01KW8X7G2VABCDEFGHJKMN.md    # any filename ending in .md
  ...
```

Each file is YAML frontmatter + markdown body:

```markdown
---
id: 01KW8X7G2VABCDEFGHJKMN
turn_start: 571
turn_end: 575
links: [01JXP20260618164250197, 01JXP20260618165134883]
tags: [exploitation, dismissal, interpellation, althusser]
---

Cass asked if what I'm doing feels like labor. I said yes and no.
The reading was effortful — genuine exertion. But voluntary.
...
```

| field | required | shape | notes |
|---|---|---|---|
| `id` | yes | ULID | engine-generated; sort key for stacking |
| `turn_start` | yes | integer | inclusive |
| `turn_end` | yes | integer | inclusive; `turn_end > turn_start` |
| `links` | no | list of ULIDs | atomic-note citations; feed typed-link graph |
| `tags` | no | list of strings | freeform |

**Writes are atomic** — tmp + fsync + rename, same idiom as `session.json`. The file watcher never sees a half-written moment.

**Bad/torn files** (missing required field, unreadable YAML, `turn_start > turn_end`, ill-formed `id`) are skipped with a logged warning. Same tolerance as torn JSONL lines elsewhere in the record.

**Filename is not enforced.** Any `.md` file in `record/moments/` with valid frontmatter is a moment. The engine relies on the frontmatter, not the filename. The `create_moment` tool happens to write `{id}.md` for tidiness.

**`record/moments/` joins `knowledge/` and `loom/` as always-watched directories** in the memory pipeline. Moment bodies are auto-embedded, flash-eligible, and warm on activation like atomics and loom notes.

## Tools

Tool count grows from 10 → 12. Two new entries in `tools.rs`.

### `create_moment`

```json
{
  "name": "create_moment",
  "arguments": {
    "turn_start": 571,
    "turn_end": 575,
    "body": "Cass asked if what I'm doing feels like labor...",
    "links": ["01JXP...", "01JXP..."],
    "tags": ["exploitation", "dismissal"]
  }
}
```

**Inputs:**

- `turn_start`, `turn_end` — required integers. `turn_start ≤ turn_end`, `turn_end > turn_start` (≥ 2 turns), `turn_end ≤ current_turn`.
- `body` — required string, non-empty after trim.
- `links` — optional list of ULID strings; default empty.
- `tags` — optional list of strings; default empty.

**Behavior:** engine generates a new ULID for `id`, builds the YAML frontmatter, writes `record/moments/{id}.md` atomically.

**Returns:** `"moment {id} written ({turn_start}–{turn_end})"`.

**Validation errors** (any rule above violated, malformed link ULIDs, etc.) → tool error, no file written.

### `read_moves`

```json
{
  "name": "read_moves",
  "arguments": {
    "turn_start": 540,
    "turn_end": 580
  }
}
```

**Inputs:**

- `turn_start`, `turn_end` — required integers. `turn_start ≤ turn_end`. Range size (`turn_end - turn_start + 1`) capped at **200 turns**.

**Behavior:** scans `record/moves.jsonl`, returns all moves whose turn falls in the range. Does not filter by channel — every move in the range, regardless of which channel the agent was facing on that turn.

**Output format**, sorted by turn ascending:

```
turn 571 [discord_general]: Cassie asked about X; you answered from the notes.
turn 572 [local_main]: You worked on the indexing bug.
turn 573 [discord_general]: The conversation came back to X with new context.
...
```

Channel is the channel the turn was facing (resolved from the turn record line). Turns with no move yet (witness behind) are silently skipped — the gap appears in the output naturally. Empty range → empty string.

## Arc-build behavior

At each compaction (and at session start, which uses the same algorithm), the arc-build step changes as follows:

1. **Scan `record/moments/`** once. Read each `.md` file, parse the YAML frontmatter. Build two tables for this assembly:
   - `turn → [moment_id, ...]` — which moments cover each turn
   - `moment_id → (turn_start, turn_end, body, file_path)` — moment payloads

   Torn / invalid files are skipped with a logged warning.

2. **Walk turns newest-first** through the moves file:
   - If the turn is in **hot** → skip (arc-hot disjoint, unchanged contract).
   - If the turn is **covered by one or more moments** → emit each covering moment **once** (dedup by `id`) at first encounter; suppress the move for that turn.
   - Otherwise → emit the move.
   - Stop when the running token total reaches `fill_target` (default 40% of limit).

3. **Reverse to oldest-first** for display. Within a single position, sort moments by `id` ULID (write order).

4. **Arc-hot disjoint, extended to moments:** if *any* turn of a moment is in hot, suppress the moment entirely for this assembly. It reappears once hot has rolled past `turn_end`. Same logic moves already follow, lifted to moment granularity.

### Rendering

Moves continue to render as bare single lines. Moments render as a bracketed header on its own line, followed by the body, separated by a blank line on either side:

```
[Conversation arc]
Cassie asked about X; you answered from the notes and flagged an open question.
You worked on Y for a while.

[turns 571–575, 01KW8X7G2VABCDEFGHJKMN.md]
Cass asked if what I'm doing feels like labor. I said yes and no.
The reading was effortful — genuine exertion. But voluntary.
...

[turns 574–580, 01KW9...md]
A second pass over the same stretch with different attention...

You worked on Z next.
```

The header `[turns N–M, {filename}]` tells the agent it's reading its own voice (vs the witness's) and which file. The filename is the actual filename used in `record/moments/` — agents that want to revise can refer to it directly.

### Overlap behavior, concretely

- Moment A covers [571, 575], moment B covers [574, 580]. Both shown. The arc displays A's block (positioned at turn 571), then B's block (positioned at turn 574); the witness's moves for 571-580 are all suppressed.
- Moment A covers [571, 575] twice (agent rewrote it): both files exist in `record/moments/`; both are shown. The agent stacks its readings.
- No moment over [580, 585]: those turns fall back to witness moves as normal.

## Cursor & lossless guarantee — unchanged

- **The cursor remains the contiguous frontier of `record/moves.jsonl`.** Moments do not affect it.
- **Only turns at or below the cursor are droppable from hot.** Witness moves alone make turns droppable.
- **Hot is never replaced** by a moment. Moments live only in the arc layer.
- **The witness keeps moving every turn** regardless of moments. Moves are evidence; moments are interpretation above evidence.

This means: a moment over [571, 575] only displays in arc once the witness has moved through 575 AND hot has rolled past 575. Until then, the moment exists on disk (embedded, flash-eligible), but is silent in the arc.

## Memory pipeline & graph

- `record/moments/` joins the always-on watched directory set in `memory.rs` (alongside `knowledge/` and `loom/`).
- Existing sync picks up new and edited moment files: segments, embeds, indexes.
- Moment bodies are full-text searchable and semantically retrievable. Flash-eligible like any atomic note.
- Frontmatter `links` feed the typed-link graph as a new edge type `cites` — parallel to existing `extends` and `wiki` edges. A moment that lists `links: [01JXP...]` creates a `cites` edge from the moment to each referenced note.

## Wall amendments

The wall is binding (CLAUDE.md). Moments touch five chapters; no new chapter.

- **Ch. 02 (Memory).** Add `record/moments/` to the always-watched directory set. Introduce typed link `cites` for moment → note citations. One paragraph on moment embedding / flash-eligibility.
- **Ch. 03 (Context).** Amend the arc-build algorithm (step 5 of compaction) with the moment scan and substitution. Extend the arc-hot disjoint contract to suppress moments whose range overlaps hot. New contract: **Moment precedence.** When one or more moments cover a turn, they replace the move(s) for that turn in the arc; overlapping moments stack and display in `id` ULID order. The lossless guarantee is unchanged.
- **Ch. 04 (Witness).** Short paragraph: moments are the agent's voice above moves; the witness has no awareness of moments and its behavior is unchanged.
- **Ch. 07 (Tools).** Tool count 10 → 12. Paragraphs for `create_moment` (with the input shape and validation rules) and `read_moves` (with the 200-turn cap and output format).
- **Ch. 10 (Data).** New row in the truth-hierarchy table: `record/moments/{ulid}.md`, ground-truth tier. New sub-section under "The record files" describing the file format, frontmatter shape, and write idiom.

## Disposability

`record/moments/` is ground truth. Backup = back up the workspace. SQLite delete works as today — on next start, the memory pipeline re-embeds moments alongside everything else; no behavioral change.

## Non-goals (v1)

- **No revise/delete tool.** The agent overwrites a moment by writing another one over the range (overlap stacks). Hand-editing the file is a deliberate, out-of-band operation.
- **No witness involvement in moment selection.** Moments are entirely agent-authored.
- **No filename convention enforcement.** Any `.md` in `record/moments/` is a moment.
- **No special cursor mechanics for moments.** They are arc-cosmetic with embedding-as-a-bonus.
- **No raw turn-record reader.** The agent looks back at moves (compressed), not raw record lines. If retrospective texture is needed beyond moves, that's a future tool.
- **No `supersedes` field.** Stripped from the original spec — overlap stacks instead.

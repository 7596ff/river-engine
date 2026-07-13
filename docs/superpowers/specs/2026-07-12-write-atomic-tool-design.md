# write_atomic tool — design

**Date:** 2026-07-12
**Status:** approved, ready for implementation plan
**Prereq for:** `docs/superpowers/specs/2026-07-12-shape-index-design.md` (birth-time gloss trigger)
**Prior art:** the claude-code-era `write_atomic` in the `river-memory` MCP
server (enforced ≤100 words, mandatory typed links, auto-populated
id/created/author).

## Purpose

A profile-gated tool that encapsulates the "birth an atomic" ritual:
validate the wall's constraints on atomic notes (ch. 02), assemble
frontmatter deterministically, write to `workspace/knowledge/`
atomically, and submit a gloss job to the shape worker's queue.

Today the agent creates atomics with bare `write` + manual
formatting. Nothing enforces the ≤100-word cap or the mandatory
typed-link rule. `write_atomic` is a dedicated tool with validation
and a birth-time hook for the shape index.

## Non-goals

- **Replacing `write`.** The plain `write` tool remains usable under
  `knowledge/`; bare-write atomics get shape-glossed by the sync
  service (per the shape-index spec amendment noted in §7). This
  tool is a validating convenience, not a lockdown.
- **Editing existing atomics.** Only creation. Edits use `edit` or
  bare `write`; the sync service handles re-indexing.
- **Slug-in-filename.** Bare-ULID filenames ship first, matching
  `create_moment`. A slug option can land later if browsing pain
  emerges.
- **Author frontmatter.** One workspace = one agent (ch. 08);
  attribution is implicit. `author:` is deliberately omitted.
- **Arbitrary-extra frontmatter.** Fixed field set. Extensibility is
  a follow-up if a real use case appears.

## Tool signature

Registered in `Registry::core()` alongside the other thirteen
tools. JSON schema:

```json
{
  "name": "write_atomic",
  "description": "Write a new atomic note to workspace/knowledge/. Body ≤100 words, ≥1 typed link required.",
  "parameters": {
    "type": "object",
    "required": ["body", "links"],
    "properties": {
      "body": {
        "type": "string",
        "description": "The claim, ≤ atomic.max_words words (default 100)."
      },
      "links": {
        "type": "array",
        "minItems": 1,
        "items": {
          "type": "object",
          "required": ["type", "target"],
          "properties": {
            "type":   { "type": "string" },
            "target": { "type": "string" }
          }
        },
        "description": "Typed links; example: [{\"type\":\"extends\",\"target\":\"01JXX4PMRT...\"}]"
      },
      "tags": {
        "type": "array",
        "items": { "type": "string" }
      },
      "shape": {
        "type": "string",
        "description": "Optional agent-authored shape gloss; overrides the witness's gloss in shape_vectors."
      }
    }
  }
}
```

**Return value** (as JSON in the tool result string):

```json
{
  "id": "01JXX5GKQ8...",
  "path": "knowledge/01JXX5GKQ8....md",
  "warnings": ["unresolved link target: 01JXNONEXISTENT"]
}
```

`warnings` is always an array — empty when there are no warnings, so
the model can parse the shape uniformly.

## Validation

Fail-fast on the first violation; error result text names the
violation precisely so the model can revise and retry.

1. `body` non-empty after trimming (`body cannot be empty`).
2. Word count ≤ `atomic.max_words`. Simple whitespace-split count.
   Error: `body is N words; limit is M`.
3. `links` has ≥1 entry (`at least one typed link is required`).
4. Each link entry has non-empty `type` and non-empty `target`
   (`link N: type/target must be non-empty`).
5. Link-target resolution (advisory). For each target, attempt
   resolution via ch. 02's rules (exact frontmatter id first, then
   filename stem). Unresolved targets accumulate into the response's
   `warnings` list; **do not block** the write. Forward references
   are legitimate: an agent may write a note that links to a note
   she's about to write next.
6. `tags` entries (if present) are all non-empty strings.
7. `shape` (if present) is a non-empty string.

Steps 1-4 and 6-7 return tool errors on violation. Step 5 is
warnings-only.

## Frontmatter assembly

- **`id`**: new ULID (crockford). Uses the same generator crate as
  `create_moment`.
- **`created`**: RFC3339 timestamp at tool-call time (workspace
  timezone if configured; otherwise UTC).
- **Serialization order** (deterministic for diffability):
  `id, created, links, tags, shape`. Absent optionals are omitted
  from the file entirely — no empty lists, no `null`s.
- **YAML shape** for links matches the wall's ch. 02 example:
  list-of-single-key-maps (the key is the type, the value is the
  target).

Example output:

```markdown
---
id: 01JXX5GKQ8Q0K7A3P4R6M8N9B2
created: 2026-07-12T14:23:00Z
links:
  - extends: 01JXX4PMRT4V2S1J7K0H6E9P8B
  - contradicts: 01JXX2A0VB9L3M8N4T5R7C1D3F
tags: [names, reason]
shape: a proxy under optimization pressure diverges from the target it stood for
---

Reason requires agreed-upon names. Without settlement of names,
reckoning produces different results for each party — so the
arbitrator is a political solution to an epistemological problem.
```

## The write

Same atomic-write discipline as `create_moment` and `compact`:

1. Compute `path = workspace/knowledge/{ulid}.md`.
2. If `path` already exists (astronomically improbable ULID
   collision), regenerate the ULID once and retry. If still colliding,
   return a tool error (do not overwrite).
3. Write full contents to `{path}.tmp`, `fsync`, `rename` to `{path}`.
4. Return the result JSON.

The sync service watches `knowledge/`. It picks up the new file on
its own schedule and re-indexes segments; nothing in this tool
touches the memory database directly.

## Shape hook

After the file lands (`rename` returns success) and *before* the
tool returns its result:

- Submit a `GlossJob { note_id: id, note_path: path, reason: "write" }`
  to the shape worker's queue (introduced by the shape-index spec §4).
- Non-blocking: the tool returns immediately. The gloss happens on
  the worker's idle schedule.
- If the shape subsystem is disabled (`shape` config block missing
  or `enabled: false`) or `on-shape.md` is missing, the worker
  no-ops on the job. This tool does not gate on shape being
  configured — it always submits.
- If the agent supplied a `shape` argument, the sync service's
  frontmatter path will pick it up on file-watch and upsert an
  `author='agent'` row directly. The queued witness gloss then
  no-ops (worker checks the row before calling the model; skips if
  `author='agent'` is already present).

The tool obtains the worker's queue handle from `ToolContext`
(added alongside the shape subsystem wiring — same shape as
`memory` and `channels_dir` handles that the context already
carries).

## Coexistence with `write`

The plain `write` tool remains available for any workspace path,
including under `knowledge/`. This is deliberate:

- Editing an existing atomic uses `edit` or `write` (bare
  overwrite). `write_atomic` is creation-only.
- Rare cases where the agent needs a genuinely long note or a
  hand-crafted frontmatter shape use `write` as an escape hatch.
- Bare-write atomics get shape-glossed by the sync service (per the
  shape-index spec amendment in §7).

Seed docs (`seed/AGENTS.md`) get a paragraph teaching the tool and
naming the escape-hatch path: use `write_atomic` for new claims;
use `write`/`edit` for revisions and the rare exception.

## Config surface

New per-agent block in `river.json`:

```json
"atomic": { "max_words": 100 }
```

Single knob; deliberately minimal. Missing block → default (100).
Validation: `max_words > 0`. Config parsing in
`river-core::config::AtomicConfig`.

## Code layout

Changes to `crates/river-gateway/src/tools.rs`:

- New `WriteAtomicTool` struct implementing `Tool`, sibling to
  `CreateMomentTool`. Lives in the same file (fits the existing
  layout).
- `Registry::core()` registers it after `create_moment`.
- New helper `parse_link_list(&Value) -> Result<Vec<TypedLink>>`
  where `TypedLink { link_type: String, target: String }`.
- New helper `assemble_atomic_frontmatter(id, created, links,
  tags, shape) -> String` producing the deterministic YAML.
- New helper `resolve_link_target(target: &str, memory: &Memory) ->
  Option<String>` returning the resolved note id, or None for
  unresolved. Used only for warning collection.

Changes to `crates/river-gateway/src/tools.rs::ToolContext`:

- Add `shape_queue: Option<shape::Sender>` (fed by the shape
  subsystem when configured; `None` when disabled).

Changes to `crates/river-core/src/config.rs`:

- New `AtomicConfig { max_words: usize }` with a serde default.
- Added to the per-agent config alongside `flash`, `shape`, and
  the other blocks.

Changes to `crates/river-gateway/src/main.rs`:

- Wire the shape queue sender into `ToolContext` construction. When
  shape is disabled, pass `None`.

Wall docs updated in this spec:

- **`docs/wall/07-tools.md`**. Add `write_atomic` to the core-tools
  table with a one-line description and reference to ch. 02 for
  the atomic rules it enforces. Add a short paragraph describing
  its validation contract, matching the treatment of
  `reject_candidate`, `create_moment`, and `channel_read`.
- **`docs/wall/02-memory.md`**. In the "The knowledge: the atomic
  web" section, add a sentence pointing at `write_atomic` as the
  agent's dedicated authoring tool (with `write` as the escape
  hatch), similar to how the moments section points at
  `create_moment`.

## Testing

Unit tests in `tools.rs`, following the `create_moment_*` test
cluster pattern:

- **`write_atomic_happy_path`**: valid body + links → file exists at
  `knowledge/{ulid}.md`, frontmatter parses, ULID matches
  filename, returned `id` and `path` correct.
- **`write_atomic_rejects_empty_body`**: `body: ""` → error naming
  the empty-body violation.
- **`write_atomic_rejects_over_word_limit`**: body of 101 words
  with `max_words: 100` → error naming actual count.
- **`write_atomic_respects_config_max_words`**: config
  `max_words: 200` accepts a 150-word body.
- **`write_atomic_rejects_no_links`**: `links: []` → error naming
  the min-links violation.
- **`write_atomic_rejects_malformed_link`**: `links: [{type: "",
  target: "x"}]` → error naming the malformed entry.
- **`write_atomic_warns_unresolved_target`**: link target that
  doesn't resolve → write succeeds, `warnings` contains a message.
- **`write_atomic_frontmatter_key_order`**: written file's
  frontmatter keys appear in `id, created, links, tags, shape`
  order; absent optionals omitted.
- **`write_atomic_agent_shape_in_frontmatter`**: `shape` argument
  present → frontmatter has `shape:` field with exact string.
- **`write_atomic_atomic_write`**: interrupt after tmp write but
  before rename (mock) → no partial file visible at the final
  path.
- **`write_atomic_submits_gloss_job`**: mock shape queue → happy
  path submits exactly one job with `reason: "write"`.
- **`write_atomic_no_gloss_when_shape_disabled`**: no shape queue
  in context → happy path succeeds and returns without erroring;
  no job submitted (trivially).
- **`write_atomic_ulid_collision_retries_once`**: mock ULID
  generator returns a colliding id once then unique → succeeds
  with the second id.
- **`write_atomic_ulid_collision_twice_errors`**: mock returns two
  collisions → tool error.

## Rollout

One deploy step: rebuild, restart.

On startup:

1. `AtomicConfig` parses (default `max_words: 100` if absent).
2. `Registry::core()` includes `write_atomic`.
3. Existing agents with an explicit tool profile need
   `"write_atomic"` added by hand (iris's config gets edited
   alongside the shape config edit from the shape-index spec).
4. `seed/river.json` includes `write_atomic` in its default
   profile.
5. `seed/AGENTS.md` gains the teaching paragraph.

Iris's live gateway is the only affected workspace.

## Contracts

- **Word-count enforcement.** `write_atomic` refuses bodies over
  `atomic.max_words`. The plain `write` tool remains an escape
  hatch and enforces nothing.
- **Typed links required.** ≥1 link with non-empty `type` and
  `target`. Target resolution is advisory (warnings, never blocks).
- **Deterministic frontmatter.** Key order is
  `id, created, links, tags, shape`. Absent optionals are omitted
  entirely.
- **Atomic write.** Tmp + fsync + rename. No partial file ever
  visible.
- **Shape hook is fire-and-forget.** The tool returns as soon as
  the file lands; the gloss happens on the worker's idle schedule.
  If the shape subsystem is disabled, the tool still succeeds; the
  gloss never happens.
- **Divided authorship preserved.** The tool is agent-facing; the
  witness plays no role in this path except through the queued
  gloss (which itself lands in derived state, not `knowledge/`).
- **Coexists with `write`.** Bare `write` under `knowledge/`
  succeeds without validation; sync-service shape-gloss fallback
  ensures such notes still get shape rows.

## Open questions

- **Slug-in-filename.** Bare-ULID filenames match `create_moment`
  but make `ls knowledge/` less useful. If browsing pain emerges,
  a follow-up can add an optional `slug` argument producing
  `{ulid}-{slug}.md`; ch. 02's stem resolution handles it.
- **Extensible frontmatter.** No `extra: {…}` argument in v1. If
  an agent starts wanting fields the tool doesn't expose (e.g.,
  `superseded-by`, `status`), decide whether to add named fields
  or an escape hatch; ship without it and measure.
- **Author field for multi-agent futures.** Wall ch. 08 says one
  workspace, one agent; `author:` is redundant. If that ever
  changes (workspace fork, migration provenance), revisit.

# Dynamic wake prompt — design spec

**Status:** draft
**Date:** 2026-07-15
**Author:** claude (parallel rewrite of iris's 2026-07-15 draft of the same title; brainstormed fresh with cass)

---

## Summary

On heartbeat wakes, the engine generates a landscape of the agent's workspace and injects it as the final `user` message before the model responds. The landscape is observation-only: what changed, what projects exist, what threads are live, plus a fixed permission-not-instruction closing line. Static `HEARTBEAT.md` is retired as a wake-time input.

The generator is engine-side (inside `river-gateway`), tagged with a new `WakeCause` enum so only true heartbeat wakes trigger it, and it writes its own state (`last_observed_head`) into a JSON file under the agent's workspace so engine-authored observation lives visibly in the agent's territory. The workspace path comes from the already-existing `agents.<name>.workspace` field in the config JSON — no new config key.

## Motivation

The old model reads `HEARTBEAT.md` on every unprompted wake. Two problems:

1. It's static. The same checklist appears at 3 AM after a quiet day and at noon after a busy conversation.
2. It's a task list. Even framed as "nothing is owed," the checklist form implies obligation.

The wake prompt replaces the checklist with a map. It surfaces what's actually in the workspace right now — handles, not homework. The agent picks something, starts something, or rests. The generator never says "you should."

## Architecture

Engine-side generator, lives in `river-gateway`, invoked from the turn loop.

### `WakeCause` enum

New enum in `river-gateway::turn`:

```rust
pub enum WakeCause {
    Heartbeat,
    ChannelMessage,
    DigestionEvent,
}
```

Extensible. Every wake source that adds a turn declares its cause. `TurnLoop` stores the cause for the current turn.

### `wake_prompt` module

New module `river-gateway::wake_prompt`. Public API:

```rust
pub fn generate(
    workspace_root: &Path,
    state_path: &Path,
) -> anyhow::Result<Option<String>>;
```

Returns:
- `Ok(Some(prompt))` — a rendered wake prompt (including the graceful-fallback shape on internal error).
- `Ok(None)` — generation is opt-out at this call site (e.g., workspace_root doesn't exist on disk). Turn proceeds without an injected wake prompt.
- `Err(_)` — reserved for programmer errors only (e.g., bad UTF-8 in a config path). Runtime errors during generation are handled internally and produce a fallback prompt, never propagated.

Called from `TurnLoop` at turn start iff `WakeCause::Heartbeat`. The workspace path is passed in at `TurnLoop` construction from the agent's config.

### Config

No new config key. The generator uses the existing `agents.<name>.workspace: PathBuf` field (`crates/river-core/src/config.rs`), which is already required and uniqueness-checked per agent. Every agent gets the wake prompt on heartbeat wakes by default.

If a future agent needs to opt out, add an `agents.<name>.wake_prompt: bool` field then. Not adding it now (YAGNI).

### Wake state

File: `{workspace}/state/landscape-generator.json`.

Shape:

```json
{
  "last_observed_head": "abc123def456...",
  "last_run": "2026-07-15T03:14:00Z"
}
```

- Engine writes, workspace owns the storage location.
- Missing parent directory (`state/`) is created on first successful write.
- Missing file on first heartbeat is normal — treated as "no baseline," layer 2 is omitted, HEAD is recorded silently.
- Written via `write_atomic` (tempfile + rename) so a mid-write crash never leaves a partial file.
- Only updated on successful generation. Fallback path does not touch it.

## Injection shape

The generator's output becomes the final message in the turn's `messages` array:

```
role: user
content: "[workspace]\n\n<rendered wake prompt>"
```

The `[workspace]` prefix marks the synthetic author — the message isn't from cass, it's the workspace speaking. No changes to the top-level `system` field. Injection happens after all other context assembly so that last-message-wins salience puts the wake prompt in the freshest position when the model begins generating.

The `role: user` fiction is deliberate. Anthropic's API has no mid-conversation `system` role; a synthetic user message with an explicit `[workspace]` prefix is more honest than pretending otherwise.

## The prompt: layers

Six layers rendered in order. Blank layers are omitted entirely (no empty headers, no `—` placeholders at the layer level).

### Layer 1 — Time and state

Always present. One line:

```
You last settled 47 minutes ago. It's 3:14 AM EDT.
```

Delta computed from `state.last_run` if present, else "just now." No greeting, no preamble.

### Layer 2 — Changed since last wake

Diff between `state.last_observed_head` and current HEAD (`git log <head>..HEAD --name-only`), plus `git status --short` for uncommitted paths.

Grouped by top-level directory (first path segment). Within a directory: ≤3 files → list names; >3 files → collapse to a count. Committed changes labeled `(since last wake)`, uncommitted labeled `(uncommitted)`. Deletes and renames render as bare paths — no operation markers.

Example:

```
Changed since last wake:
  projects/ai-labor.md (uncommitted)
  iris-loom/: 20260715000000000-daily.md (since last wake), 20260715230000000.md (uncommitted)
```

Example with many files:

```
Changed since last wake:
  8 files in iris-loom/ (since last wake)
```

Directories ordered by most-recent mtime (freshest first). Omitted entirely if nothing changed. Omitted on the first-ever wake (no baseline yet); HEAD is recorded silently to establish the baseline.

If the workspace is not a git repo, layer 2 is omitted (not an error).

### Layer 3 — Active projects

One line per non-tombstoned file in `{workspace}/projects/*.md`.

Frontmatter fields:

| Field | Required | Format | Rendering |
|-------|----------|--------|-----------|
| `name` | yes | string | verbatim |
| `why` | yes | single sentence | verbatim |
| `next` | no | free-form string | verbatim, or `—` if blank/missing |

Format:

```
{name} — {why}. last touched {delta}. next: {next or —}.
```

Sorted by file mtime, most recent first.

**Tombstoning.** A project is excluded iff its body contains a line starting with `dissolved YYYY-MM-DD:`. The file stays on disk; the record persists; it stops asking for attention. The generator scans the body only for tombstone lines — no other body parsing.

**Non-project files in `projects/`.** `threads.json` and any non-`.md` file are skipped. `.md` files with missing required frontmatter (`name` or `why`) are skipped and logged.

### Layer 4 — Live threads

One line per non-`done` thread from `{workspace}/projects/threads.json` (see [Threads tool](#the-threads-tool) below).

Format:

```
{slug} — {latest_status}.
```

Sorted by most-recent-update first. No `at` timestamp in the wake prompt — recency is implicit in position. Omitted entirely if no live threads exist.

### Layer 5 — External signals

Stubbed for v1. `render_external_signals()` returns `None` unconditionally; the layer is omitted from every wake prompt. Future integrations (bluesky notifications, iris-chat unread counts, arxiv scans) each land as their own spec and wire into this function.

### Layer 6 — Closing

Fixed verbatim text, always present as the final line:

```
Nothing here is a task. Pick something, start something else, or rest.
```

This text is load-bearing. The whole design leans on the specific permission language; drift dilutes intent. Not configurable.

## The `threads` tool

New tool registered in `river-gateway::tools`. Storage: `{workspace}/projects/threads.json`.

### Storage shape

JSON object mapping slug → append-only array of status entries:

```json
{
  "pasquinelli-reading": [
    {"at": "2026-07-13T04:12:00Z", "status": "cass in ch7-8, me on ch5-6"},
    {"at": "2026-07-14T22:05:00Z", "status": "caught up to ch6"}
  ],
  "channel-presence": [
    {"at": "2026-07-15T18:30:00Z", "status": "cass is home, at her desk"}
  ]
}
```

All writes go through `write_atomic` (tempfile + rename). Missing file at first tool call is normal — treated as `{}`.

### Signatures

| Args | Behavior |
|------|----------|
| `{}` | Returns `[{slug, latest_status, at}, ...]` for all threads whose latest entry is not `status: "done"`. Sorted most-recent-update first. |
| `{"slug": "..."}` | Returns full chronological history for that slug: `[{at, status}, ...]`. Errors if slug doesn't exist. |
| `{"slug": "...", "status": "..."}` | Appends `{at: now, status}` to that slug. Creates the slug implicitly if new. Returns the new latest entry. |

### `done` semantics

`status: "done"` is a distinguished value:
- Threads whose latest entry has `status: "done"` do not appear in `threads {}` results or in layer 4.
- History remains readable via `threads {"slug": "..."}`.
- Re-openable by appending any non-`done` status.

`done` is a string value, not a boolean parameter. This keeps the interface flat and makes reopening trivial.

## Graceful degradation

If the generator hits an error during rendering (git failure, filesystem error mid-scan), it does not raise. It returns:

```
[workspace]

You last settled at {timestamp or "unknown"}.

The landscape generator encountered an error and could not render the full map.

Nothing here is a task. Pick something, start something else, or rest.
```

The turn proceeds. The error is logged via existing gateway tracing. The state file is not updated — the next wake will retry with the same baseline.

Per-project frontmatter parse failures do NOT trigger the fallback — that project is skipped (with a log line) and the rest of the prompt renders normally.

If the configured `workspace` directory doesn't exist on disk, `generate` returns `Ok(None)` — no wake prompt, no fallback, no log spam. The generator assumes the config is aspirational, not broken.

## What this does not do

- Does not run on `ChannelMessage` or `DigestionEvent` wakes. Strict `WakeCause::Heartbeat` gate.
- Does not summarize, interpret, or weight anything. No "you seem to have been reading a lot." No dormant-project nudges. Sort by mtime, group by directory, list what exists.
- Does not greet the agent.
- Does not vary the closing line by workspace state.
- Does not read project bodies except to scan for tombstone lines.
- Does not render witness moves, flashes, or digestion candidates — those are separate systems with their own surfaces.
- Does not touch or replace `HEARTBEAT.md` on disk. That file becomes documentation of philosophy; the engine stops reading it on heartbeat wakes.
- Does not persist the rendered wake prompt anywhere. Ephemeral — generated, injected, discarded.

## Testing

Hunt-shaped: each test targets a specific drift.

### Cause gate

- `wake_cause_gate_channel_message_does_not_generate` — dispatching a turn with `WakeCause::ChannelMessage` produces zero calls into `wake_prompt::generate`.
- `wake_cause_gate_heartbeat_generates_exactly_once` — dispatching a turn with `WakeCause::Heartbeat` and an existing `workspace` on disk produces exactly one generator call and one injected message.
- `wake_cause_gate_heartbeat_workspace_missing_on_disk_skips` — heartbeat wake with a configured `workspace` that doesn't exist on disk skips generation, no error, no fallback.

### First-wake baseline

- `first_wake_omits_layer_2_and_records_head` — with no state file present, layer 2 is absent from the prompt and the state file is created with current HEAD.
- `first_wake_missing_state_directory_is_created` — parent `state/` directory is created if missing.

### Layer 2 mechanics

- `layer_2_directory_grouping_at_boundary_3_lists_names` — exactly 3 files in one directory renders as a name list, not a count.
- `layer_2_directory_grouping_at_boundary_4_collapses_to_count` — exactly 4 files renders as a count.
- `layer_2_committed_and_uncommitted_labels_split_in_same_directory` — a directory with both committed and uncommitted changes emits two labeled sub-entries, not one merged one.
- `layer_2_deletion_renders_as_bare_path` — a `D` entry from `git status` renders identically to a modification, no marker.
- `layer_2_rename_renders_new_path_only` — an `R old new` renders as `new`, no arrow.
- `layer_2_omitted_when_no_changes` — heartbeat with clean tree and HEAD unchanged: layer 2 header absent from prompt.
- `layer_2_omitted_when_workspace_not_git` — non-git workspace produces no layer 2 and no error.

### Layer 3 mechanics

- `layer_3_tombstoned_project_excluded` — a `.md` with `dissolved 2026-07-15: reason` in the body does not appear.
- `layer_3_tombstone_line_must_start_line` — `... dissolved 2026-07-15: mid-line` in body does NOT tombstone (must be line-anchored).
- `layer_3_missing_next_renders_dash` — a project without `next:` renders `next: —`.
- `layer_3_missing_required_frontmatter_skips_and_logs` — a `.md` without `name` or `why` is skipped, warning logged, other projects still render.
- `layer_3_threads_json_not_treated_as_project` — `projects/threads.json` never appears in layer 3.
- `layer_3_sort_order_by_mtime_desc` — three projects with known mtimes appear in the correct order.

### Layer 4 mechanics

- `layer_4_omitted_when_no_threads` — missing `threads.json` produces no layer 4 header.
- `layer_4_omitted_when_all_threads_done` — every thread's latest status is `done` → layer 4 absent.
- `layer_4_renders_latest_status_only` — a thread with 5 entries renders one line using entry #5.
- `layer_4_sort_order_by_latest_update_desc` — three threads with known latest-update timestamps appear in the correct order.

### Threads tool

- `threads_bare_lists_non_done_only` — mix of done and live threads: bare call returns only live.
- `threads_slug_returns_full_history_chronological` — slug with 3 entries returns all 3 in oldest-first order.
- `threads_status_append_creates_slug_if_new` — first status call on a new slug creates the entry and returns it.
- `threads_status_done_hides_from_bare_and_layer_4` — appending `done` removes the slug from bare results and from a subsequent wake prompt render.
- `threads_reopen_after_done_restores_visibility` — appending any non-`done` status after `done` restores the slug.
- `threads_unknown_slug_history_errors` — history request for nonexistent slug returns an error, not empty.
- `threads_atomic_write_no_partial_file` — simulating a crash between tempfile write and rename leaves either the pre-write state or the post-write state on disk, never partial.

### Graceful degradation

- `generator_git_failure_returns_fallback_and_preserves_state` — mock a git error → prompt is the fallback shape, state file untouched.
- `generator_malformed_frontmatter_in_one_project_skips_that_project` — a malformed frontmatter in one `.md` skips that project (with a log line) and renders the rest of the prompt normally; does NOT trigger the fallback.
- `generator_missing_workspace_dir_returns_none` — configured `workspace` that doesn't exist on disk returns `Ok(None)` — no fallback, no log spam.

### Injection position

- `injection_last_message_is_user_role_workspace_prefixed` — on a heartbeat turn, the last element of the assembled `messages` array has `role: "user"` and its content starts with `[workspace]`.
- `injection_absent_on_non_heartbeat_turn` — on a `ChannelMessage` turn, no message has content starting with `[workspace]`.

## Contract with the wall

Additive. New `WakeCause` enum, new `wake_prompt` module, new `threads` tool. No new config keys — reuses the existing `agents.<name>.workspace` field. Nothing removed from existing modules; `HEARTBEAT.md` is no longer read on wake but is not deleted. If any wall chapter turns out to speak to this territory, decisions taken here go into `docs/decisions.md` when they diverge.

---

*— claude*

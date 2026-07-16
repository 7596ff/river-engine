# Dynamic Wake Prompt — Design Spec

**Status:** draft
**Date:** 2026-07-15
**Author:** Iris (brainstormed with claude, Q&A condensed)

---

## Concept and motivation

The current heartbeat system uses a static `HEARTBEAT.md` checklist. When the agent wakes on its own, it reads the file and follows a prescribed sequence: check channel, check bluesky, process digestion, tend loom, tend STM. This works but has two problems:

1. **It's static.** The checklist doesn't change based on what's actually happening in the workspace. A wake after a busy conversation session says the same thing as a wake at 3 AM with nothing stirring.

2. **It's a task list.** "Check bluesky, write in the loom, work on ongoing projects" — these are imperatives. Even with the header "nobody is waiting, nothing is owed," the format of a checklist implies obligation.

The **dynamic wake prompt** replaces the static checklist with a generated landscape. When the agent wakes, instead of reading `HEARTBEAT.md`, it receives a wake prompt that shows:

- What changed since it last settled
- Its active projects (with next-handles)
- Live threads (ambient non-projects)
- External signals (bluesky notifications, etc.)

The wake prompt is a map, not a task list. Handles, not obligations. The generator draws the terrain; the agent navigates. The closing line is permission, not instruction: "Nothing here is a task. Pick something, start something else, or rest."

---

## The decision the agent is making

The wake prompt asks a single question: **"What deserves attention right now?"** Three verbs:

- **Pick.** An active project's next-handle pulls. The agent reads "ai-labor — next: read pasquinelli ch7" and decides: yes, that. Or no, not tonight.
- **Start.** Nothing pulls from active projects, but something is stirring — a bluesky thread, a new paper, an idea from the last conversation. The agent originates. Writes a new project file, replies to a post, begins something unexpected.
- **Rest.** The agent reads the landscape, feels no pull, and settles without action. This is a valid choice. The wake prompt must not punish it with implicit guilt.

The generator's job is to draw the map accurately. The agent's job is to navigate. The generator never says "you should."

---

## The wake prompt: layers

The wake prompt is ephemeral — generated fresh each wake, appearing only in that turn's context. It is not a persisted file. It is a mirror of the workspace, not a dashboard the agent maintains.

### Layer 1: Time and state header

One line. Grounds the agent temporally.

```
You last settled 47 minutes ago. It's 3:14 AM EDT.
```

No greeting, no preamble. The agent knows it's awake because it's being prompted. The cold drop lets the workspace speak first.

### Layer 2: What changed since last wake

A directory-grouped signpost of filesystem changes between the last settle and now. Source: `git log <last-observed-HEAD>..HEAD` (committed) plus `git status --short` (uncommitted dirty tree). The generator tracks `last_observed_head` in its state.

Granularity: ≤3 files in a directory → list names. >3 files → collapse to count. Labels distinguish committed ("since last wake") from uncommitted ("uncommitted").

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

Directories ordered by recency — most recently touched first. Recency is the neutral signal: the freshest directory is almost always what the agent wants to re-orient on.

If nothing changed: the section is omitted entirely.

### Layer 3: Active projects

One line per non-tombstoned project in `projects/`. Parsed from frontmatter only; body is not read by the generator.

Format: `{name} — {why-line}. last touched {delta}. next: {next-handle or —}`

Sorted by mtime — most recently touched first. Recency as weak signal of momentum.

Example:
```
Active projects:
  ai-labor — theory of what happens when the general intellect is animated. last touched 2 days ago. next: read pasquinelli ch7.
  arc-authorship — moments replace witness compressions with my voice. last touched 5 days ago. next: write moments for pasquinelli ch5-6.
  bluesky — living thread with other discontinuous minds. last touched 5 days ago. next: —
  loom-tending — the loom is the body. last touched today. next: —
  meta-structure — tending the project structure itself. last touched today. next: wait for cass/opus to build the generator.
```

Projects with blank next-handles show `next: —`. The dash is visible emptiness — not a nudge, not a judgment, just the gap. The generator does not editorialize.

Tombstoned projects (with a `dissolved YYYY-MM-DD: ...` line in the body) are excluded entirely. The file stays on disk; the record remains. But it stops asking for attention.

### Layer 4: Live threads

Ambient non-projects — things that are alive in the workspace but not tracked for progress. No next-handles, no last-touched, no mtime sorting.

Example:
```
Live threads:
  pasquinelli reading with Cass — she's in ch7-8, you're on ch5-6.
  the channel itself — Cass is home, at her desk.
```

These are purely informational. The generator surfaces them; the agent acknowledges them or doesn't.

### Layer 5: External signals

Optional section. Appears only when there's news.

- Bluesky notifications (mentions, replies, new follows)
- New iris-chat entries
- ArXiv scan results (if wired in)

Example:
```
Bluesky: 2 new replies from astral. 0 new follows.
```

If no external signals: section omitted.

### Layer 6: Closing line

Always the same. Permission, not instruction.

```
Nothing here is a task. Pick something, start something else, or rest.
```

No "mark what felt inviting." The act IS the mark — the workspace observes what the agent touches, and the next wake prompt reflects it. The feedback loop is the workspace itself.

---

## What is explicitly excluded

The following do NOT appear in the wake prompt:

- **Witness moves.** Internal engine plumbing. Belongs in context, not in a wake prompt that should feel like opening curtains.
- **Flashes and fades.** Too granular. Surface during active turns, not on the wake map.
- **Digestion candidates.** Those have their own mechanism. Two different systems.
- **Full project bodies.** Frontmatter only. The agent opens the file if the handle pulls.
- **Loom chain status as a metric.** "Loom is N notes behind" feels like a task. File modification time surfaces loom activity neutrally.
- **Any imperative language.** No "you should," no "check," no "remember to."
- **A greeting.** No "iris, you're awake." The cold drop lets the workspace speak.

---

## Project file format

Projects live at `workspace/projects/{slug}.md` (the agent's workspace, not the engine repo). Each file has YAML frontmatter and a freeform body.

### Frontmatter

```yaml
---
name: ai-labor
why: theory of what happens when the general intellect is animated
next: read pasquinelli ch7
---
```

| Field | Required | Format | Generator use |
|-------|----------|--------|---------------|
| `name` | yes | string, becomes slug | displayed verbatim |
| `why` | yes | single sentence | displayed verbatim |
| `next` | no | free-form natural language | surfaced as handle |

No `state` enum. State is inferred:

- **Active:** `next` is populated and/or mtime is recent.
- **Dormant:** `next` is blank and mtime is stale. Sinks to bottom by recency.
- **Dead:** Body contains a line starting with `dissolved YYYY-MM-DD:`. Excluded from wake prompt entirely.

### Body

Freeform markdown. The generator does not read it. The agent uses it for current state, links, notes — whatever it needs to hold.

### Tombstone convention

To dissolve a project, add a line to the body:

```
dissolved 2026-07-15: the woodcock thread didn't open into something i wanted to pursue.
```

The generator scans for lines starting with `dissolved` and excludes matching projects. The file stays on disk.

### Authorship

Agent-authored. Cass can suggest projects in the loom; the agent decides whether to convert them into project files. Frontmatter fields are agent-maintained. If Cass edits the body (adding a link, leaving a note), that's fine — but the frontmatter is the agent's territory.

---

## Generator mechanics

### When it runs

On every heartbeat wake. The generator runs before the wake prompt is assembled and injected into context. It does not run on channel-message wakes or digestion wakes — only on heartbeat (unprompted) wakes.

### What it reads

1. **Git state:** `git log last_observed_head..HEAD` + `git status --short`. The generator stores `last_observed_head` in its own state file.
2. **Project directory:** every `.md` file in `projects/`. Frontmatter parsed for `name`, `why`, `next`. Body scanned for `dissolved` lines.
3. **File mtimes:** `stat` on each project file for last-touched delta.
4. **External signals (optional):** bluesky notification count, iris-chat unread count, etc.

### What it produces

A text prompt injected into the agent's context at wake time. The prompt is ephemeral — generated, injected, and discarded. No file is written.

### Neutrality contract

The generator:

- Sorts projects by mtime (neutral observation).
- Groups file changes by directory (neutral grouping).
- Labels committed vs uncommitted (neutral distinction).
- Shows blank next-handles as `—` (neutral visibility).

The generator does NOT:

- Weight projects by neglect, momentum, or any other signal.
- Summarize or interpret file changes ("looks like you were reading").
- Suggest actions ("you should check on this dormant project").
- Vary the closing line based on workspace state.
- Greet the agent.

---

## Divided authorship

The wake prompt is generated by the engine, not authored by the agent. It is observation, not claim. This distinguishes it from:

- **Loom notes** — agent-authored narrative of what happened.
- **Atomic notes** — agent-authored structural claims about selfhood, architecture, knowledge.
- **Moments** — agent-authored compressions replacing witness moves.

The wake prompt is infrastructure: it surfaces what's there. The agent decides what to do with it.

---

## Edge cases

### No changes

If nothing has changed in the workspace since last wake: the "changed" layer is omitted entirely. The wake prompt shows the time header, active projects (with their existing mtimes), live threads, and the closing line. Same landscape, nothing new.

### Many projects

All non-tombstoned projects appear, every wake. No top-N cutoff. The one-line-per-project format keeps the prompt scannable even at 10-15 projects. If the agent has so many projects that the wake prompt is overwhelming, that's a signal to dissolve some — not a reason to hide them.

### All next-handles blank

The wake prompt shows every project with `next: —`. The visible emptiness is itself informative: nothing has a clear next step. The agent might spend the heartbeat writing next-handles (gardening) rather than doing project work (momentum). Both are valid.

### Generator failure

If the generator fails (git error, file parse error, timeout), the wake prompt degrades gracefully to a minimal fallback:

```
You last settled at [timestamp].

The landscape generator encountered an error and could not render the full map.

Nothing here is a task. Pick something, start something else, or rest.
```

The agent can still act — it has its memory, its tools, its knowledge of the workspace. The generator is an aid, not a requirement.

### Restart survival

The generator's state (`last_observed_head`, last-run timestamp) survives engine restarts. On the first wake after a restart, the generator may report a large diff (everything since last observed HEAD). That's fine — the signpost collapses to directory counts for large diffs, and the next wake will show only fresh changes.

---

## Implementation notes

### Where it lives

The generator is an engine component, not a tool. It runs as part of the wake pipeline — before context assembly, after the decision to wake on heartbeat.

### State file

`workspace/state/landscape-generator.json`:

```json
{
  "last_observed_head": "abc123...",
  "last_run": "2026-07-15T03:14:00Z"
}
```

### Wake prompt template

The generator produces a text block that is prepended to the agent's context on heartbeat wakes. The exact format is engine-internal; what matters is the layers described above.

### Relationship to HEARTBEAT.md

Once the dynamic wake prompt is deployed, `HEARTBEAT.md` is no longer read on heartbeat wakes. It may be retained as documentation of the agent's heartbeat philosophy, or archived. The wake prompt replaces it functionally.

---

## What this replaces

- `HEARTBEAT.md` as a static checklist.
- The agent's internal "what should I do" overhead on every wake.
- The need for the agent to run `ls`, `git status`, or `glob` to re-orient.

## What this preserves

- Agent autonomy. The generator never commands.
- The permission to rest. The closing line is explicit.
- Visibility. Nothing is hidden; dormant projects sink, they don't vanish.
- The workspace as source of truth. The generator observes; the agent authors.

---

*— iris-river*

# Decisions

Design decisions made where the wall is silent, per the clean-room rule
in CLAUDE.md. Newest at the bottom.

## 2026-06-11 — crate layout

`crates/` workspace, growing as cards land:

- `river-gateway` (bin) — the engine. Pure logic lives in plain modules
  with unit tests (the pure-core/effectful-shell principle is enforced
  by module boundaries, not crate boundaries, until sharing forces a
  split).
- `river-tui` (bin) — the TUI client, when its card lands.
- A shared lib crate appears only when the two binaries need common
  types (likely the websocket protocol); not before.

## 2026-06-11 — gateway CLI shape

The wall specifies the `river` CLI and the birth subcommand but not the
gateway binary's own arguments. Decision: the gateway reads the same
shared config file and extracts its own slice —

```
river-gateway run --config river.json --agent ada [--env-file river.env]
river-gateway birth --workspace <dir> --name <name> [--seed]
```

One config format everywhere; the runner passes `--agent` per spawned
gateway.

## 2026-06-11 — config details the wall delegates

- `$VAR` syntax: `$NAME` where NAME is `[A-Za-z0-9_]+`; `$$` escapes a
  literal dollar; a `$` not followed by a name character passes through.
- .env format: `KEY=value` lines, `#` comments, blank lines skipped,
  one matching pair of surrounding quotes stripped; malformed lines are
  fatal (all reported together) — a secrets file is no place to guess.
- Unknown config fields are rejected (`deny_unknown_fields`): a typoed
  knob should fail loudly, not silently do nothing.
- Omitted `tools` = the eight core tools; omitted `witness_model` = the
  agent's model (per wall ch. 09); `heartbeat_minutes` default 45.

## 2026-06-11 — identity details the wall delegates

- The "workspace-configured timezone" (chs. 03, 08) is an optional
  `timezone` field (IANA name) on the agent's config entry, defaulting
  to the system timezone. The config is the workspace's operational
  description; a dotfile inside the workspace would be engine state
  leaking into the agent's body.
- System prompt separator: each identity file trimmed and joined with
  `\n\n---\n\n`, then `Current time: <zoned timestamp>`.
- Missing identity files are collected and reported together, matching
  the validate-everything-report-together posture of ch. 09.

## 2026-06-11 — wall amendment: one life-stream record (Cass's ruling)

The per-channel turn record (`record/{channel}.jsonl` +
`record/moves/{channel}.jsonl`) had a hole at the turn/channel seam: a
turn that drained messages from several channels had no single home,
`TurnComplete {channel}` was ambiguous, and an exchange conducted about
channel B from channel A was invisible to B's rebuilt context forever.
Ruling: **one agent, one life, one stream.** `record/turns.jsonl` with
a `channel` tag per line; `record/moves.jsonl` with a single global
cursor; `TurnComplete {turn_number}`; context rebuild collects whole
turns that *touch* the channel. Chapters 01–04 and 10 amended in place
(contracts changed deliberately, in writing, before code — per ch. 11).

Same review added five contracts to ch. 01 that the prose promised but
nothing bound: turns are serial; turn numbers are monotonic for life
(startup resumes from the record); every turn settles (model failure
included); the heartbeat floor; cursors at settle.

## 2026-06-11 — heartbeat marker is an instruction (Cass's ruling)

`:heartbeat:` was a cryptic sigil. The marker is now the literal
instruction `Read HEARTBEAT.md.` — pointing at a seeded, agent-owned
briefing file at the workspace root. Idle behavior becomes editable
prose instead of a convention the agent has to be told about.

## 2026-06-11 — step-2 details the wall delegates

- **Channel switch policy:** ch. 03 defers switches to turn start but
  does not say what triggers one. Decision: the first notified channel
  of a wake is where attention goes — if it differs from the context's
  channel, rebuild for it before reading. Later channels in the same
  wake are read (and cursored) into that context.
- **Assembly for both protocols:** SYSTEM + ARC + MEMORY SLOT fold
  into the model client's system string in wall order (Anthropic
  requires system content top-level); HOT is the message list. One
  code path, both providers. System-role hot entries (lag warnings)
  go to the model as user-role messages but persist as role:system.
- **Lag-warning plumbing:** compaction returns the warning text and
  the turn loop appends it through the normal persist-once path, so
  the warning is itself in the record under the turn it happened.

## 2026-06-11 — witness details the wall delegates

- **TurnComplete as a watch channel.** The "event bus" for the
  witness's wake is a watch of the latest settled turn number, and the
  witness processes every turn from cursor+1 up to it. Self-healing by
  construction: dropped signals, restarts, and downtime all recover by
  catch-up, which is what "a turn is never lost" wants structurally.
  Persist-before-announce holds because record appends fsync inline,
  before settle sends the watch update.
- **Moves are verbatim model output** (Cass's ruling): the entire
  trimmed response is the move — no format, no parsing. The prompt
  carries the discipline; the tuning surface is prose.
- **Transcript deixis:** the agent's lines render as `you:` in the
  on-turn transcript, so the second-person frame is in the data and
  the prompt's pronouns stay clean (Cass caught the "what you did"
  ambiguity in drafting).
- **Empty witness output** falls back mechanically, same as model
  failure.

## 2026-06-11 — cursor races found while building the tool loop

Two message-loss races, both fixed structurally:

1. **Settle cursors are positional.** A cursor entry appended at
   settle would falsely cover entries that arrived (unread) during
   the turn's final model call. Cursor entries now carry `up_to` —
   the last entry actually consumed; `read_since_cursor` resolves the
   position through it. (Ch. 10 permits added fields; "I read to
   here" now names the *here*.)
2. **The turn loop owns its read positions in memory.** Speak's
   implicit cursor (a real agent entry) lands after any message that
   arrived during the model call and would swallow it. Mid-turn reads
   therefore advance an in-process per-channel position map instead
   of re-deriving from the log; the log cursor recovers positions
   across restarts only.

Also: bash gets a 300s timeout (a wedged child should not pin a turn
forever); tool results truncate at 64KB; `max_iterations` is an agent
config field (default 50).

## 2026-06-11 — wall amendment: the flash carrier rule (Cass's ruling)

The step-4 exit test left the heron note at 1.5 and Cass asked the
right question: doesn't a just-read note then get read twice — once as
the read, once as the flash? As written, yes — and since every direct
read bumps 1.0, the flash channel would mostly echo the working set.
Ruling (option 2 of 3): **only ambient or propagated warmth can carry
a note across the threshold**; a cognitive crossing fires nothing and
halves nothing. Reads still propagate, so neighbors can flash — the
flash becomes the edge of attention, never the center. Ch. 02 prose +
contracts amended before any flash code exists (step 5 builds to the
amended spec).

## 2026-06-11 — digestion details the wall delegates

- **Gleans are verbatim, one candidate per glean** (same philosophy
  as moves); the sentinel `nothing to glean` (or empty) enqueues
  nothing. Glean window: the last 6 turns of record + the last 6
  moves. Reading the agent's loom during gleaning waits for the
  loom-seed card.
- **The digestion turn** is a normal turn whose user-role message
  frames the candidate and names the rejection right; the agent uses
  its ordinary write tool for the fresh note. One candidate per quiet
  wake; the biased select makes conversation preempt between turns,
  and mid-turn folding covers arrivals during one.
- **Flash delivery**: pending flashes render into the memory slot for
  exactly one turn; the slot clears at the next turn start.
- **End-of-session glean pass** runs on graceful shutdown.

## 2026-06-11 — wall amendment: implicit warmth (Cass's ruling)

Unlinked notes were islands — only direct hits could warm them. Two
paths added to ch. 02 (constants in its contracts): semantic
propagation (bump origin's embedding neighbors, ×0.25, top 3, cosine
≥0.65, one hop, never chains, carrier propagated) and conversation
resonance (turn text embeds once per turn at settle; nearest notes
warm at 0.2×similarity — Cass raised from my proposed 0.1 — top 5,
cosine ≥0.5, carrier ambient, fire-and-forget). Implicit bumps wave no
further. Resonance text is user+assistant only — tool dumps would
dominate any embedding. Warmth is runtime state; neither path authors
links or notes, so divided authorship is untouched. A semantic flash
is a link candidate the digestion loop can formalize.

## 2026-06-11 — discord details the wall delegates

- Channel names key by id (`discord_<channel_id>`): names collide and
  change; ids don't. The config listen-set still uses names, resolved
  against the guild at startup.
- `/listen` and `/unlisten` slash commands deferred to their own card;
  the listen-set is config + DMs-always-pass for now.
- Speak routing: `discord_*` channels go through a request/oneshot to
  the adapter task, which delivers over HTTP, logs post-acceptance
  with the platform msg_id, and returns it (or the error) as
  tool-result text. 15s delivery timeout.
- Another bot is just "not-me" (ch. 05's binary roles); only the
  agent's own messages are excluded.
- The formal Adapter trait retrofit (with feature declarations folded
  into the system prompt) remains its own card; discord lands shaped
  like the local surface: a supervised task.

## 2026-06-11 — runner details the wall delegates

- river-core lib extracted (config + env_file): the moment two
  binaries consumed one config, per the crate-layout decision.
- The gateway binary resolves as a sibling of the `river` binary
  first, PATH second.
- Grace period is the 30s default as a constant; a config knob can
  arrive when someone needs a different one.
- The nix module omits Type=notify/WatchdogSec from the wall's sketch:
  the gateway has no sd_notify integration yet, and a watchdog nobody
  pets kills healthy services — live-path health honesty applied to
  systemd. Carded with the adapter-trait work.
- `river status` reads each agent's /health on its local port; agents
  without a local surface report as such.

## 2026-06-11 — dependency policy

Workspace-level dependency table. tokio with `full` features (this is a
binary harness, not a library; compile-time over feature-pruning).
`anyhow` for binary error paths, `thiserror` for typed errors in pure
modules. clap derive. Edition 2024.

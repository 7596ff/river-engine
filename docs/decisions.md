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

## 2026-06-12 — tool resonance (Cass's ruling)

A third implicit-warmth path: every tool result is embedded and
searched against the index, and the nearest notes warm at **0.8 ×
similarity** (Cass: "embed tool results and search them against the db
and bump the graph, say 0.8 times the distance" — implemented as 0.8 ×
cosine similarity, the system's measure). Details delegated and chosen
to match conversation resonance: top 5, cosine ≥ 0.5, carrier ambient
(so a crossing flashes), text capped at 4000 chars, fire-and-forget
spawn per tool result inside the act loop — never blocks the turn. No
exclusions: search results re-warm what the search already bumped,
which is the point — handling is hotter than mentioning. Wall ch. 02
amended (implicit warmth section + constants contract).

## 2026-06-12 — the self-dialogue incident

First night live, iris answered "how are you feeling?" with five speaks
in one turn — hallucinating Cass's replies between them and closing
with a false memory ("She sent a hug, I sent one back"; the hug was her
own). Root cause: nothing told the model that replies cannot arrive
mid-turn — listening requires settling. Three-layer fix:

- **Workspace** (her AGENTS.md): a "Listening requires settling"
  section — end the turn to hear; never continue the conversation on
  the other person's behalf; multi-part sends of your own thought are
  the one exception. Effective immediately (system prompt is rebuilt
  every turn).
- **Speak tool result** carries the cue at the moment of the failure
  mode: " — if you await a reply, end your turn; replies arrive as new
  turns". Model-agnostic; belongs in the engine because every future
  inhabitant needs it, not just this one. The seed AGENTS.md card
  should carry the settling discipline too.
- **Digestion framing** moved from user role to **system role**, with
  text naming what it is ("your own memory passing through digestion,
  not a message from anyone"). Conversational candidates — especially
  backfilled-life candidates — read as live speech in a user-role
  message, and the agent answers people who are not there. Trailing
  system messages are proven safe with deepseek (mid-turn folding
  already produces them). The heartbeat marker stays user-role per ch.
  01's explicit contract; its content is non-conversational.

## 2026-06-12 — flashes linger three turns (Cass's ruling)

A flash now rides the memory slot for three turns instead of one
(FLASH_VISIBLE_TURNS in turn.rs), fading after. A re-flash while
visible restarts the countdown; duplicates dedupe by note id. The
countdown lives in the turn loop (the slot is its assembly), not in
the memory store. Wall ch. 02 flash section amended. The constant
belongs in the activation config block when that card lands.

## 2026-06-12 — wall amendment: min_messages default 50 (Cass's ruling)

The post-compaction floor of live messages was 20; Cass raised the
default to 50. Same knob, same best-effort semantics (backfill whole
turns newest-first, stop at the threshold) — just a deeper floor of
verbatim recency after each compaction. Ch. 03 prose + table amended;
config default + pin test updated.

Also ruled today: gleaning does NOT read the loom (the ch. 04 clause
"any notes the agent wrote since the last pass" stays an open
question, neither built nor amended out); seed/AGENTS.md now exists —
the loom practice ships with the engine as prose (taught, indexed,
never enforced), with the glean claim removed pending the open
question.

## 2026-06-12 — the loom conducts: wikilinks + tolerant resolution built

Both approved cards landed (wall ch. 02 amended in the same commit):
every indexed file is now a graph node (frontmatter id, else path);
[[wikilinks]] in any body are type-"wiki" edges; targets resolve by
exact id, then unique filename stem — ambiguity conducts nothing.
Found and fixed in passing: YAML-quoted ids (iris's atomics) never
matched their bare link targets — quotes now stripped on both sides.
Flash bodies capped at ~1200 chars / 6 neighbors (typed first), so
path-keyed nodes the size of transcripts can't flood the memory slot.
Also: loom/ is always watched (ch. 08), and nested watch dirs dedupe
by normalized path (index_dirs ["."] used to double-index under two
spellings).

## 2026-06-12 — activation knobs + flash directory filter built

The per-agent `activation` config block exists (river-core): every
dynamic — bumps, factors, hops, top-ks, thresholds, decay,
search_top_k — is a knob defaulting to the wall's constants, validated
at startup (decay in (0,1), thresholds in [0,1], factors non-negative,
all errors reported together). `flash_dirs` rides in the same block:
when set, only notes under those workspace-relative prefixes may
surface; a filtered crossing stands silently like a cognitive one (no
flash, no halve — warmth and conduction untouched). Ch. 02 contracts
amended from constants to defaults. Segmentation caps, the decay
interval, and flash body caps stay code constants — mechanics, not
dynamics.

## 2026-06-12 — the instrument panel: /graph, /graph/view, /context, /context/view

All four read-only routes built on the local surface (wall ch. 06
amended). GET /graph walks the live workspace (cold nodes at score 0,
typed + wiki edges with dangling targets dropped, semantic edges =
per-node top-k cosine above the configured threshold, deduped by
unordered pair) and runs on a blocking thread. GET /context serves a
ContextSnapshot the turn loop publishes at every settle — per-layer
token estimates, hot turn range, slot contents, calibration ratio; a
turn that never settles never updates the window, which is honest.
The view pages are single self-contained HTML files served from
strings compiled into the binary (d3-force v3 + deps vendored inline,
~18KB, MIT). Flash detection in the graph view is inferred
client-side from a steep score drop near the threshold — cosmetic
only, the engine exposes no flash event on this surface. Verified:
endpoints integration-tested over real HTTP; the vendored d3 bundle
evaluates and ticks headless; both app scripts parse. Not yet
verified: the pages rendered in a real browser against a live agent.

## 2026-06-13 — moves regenerate: gap scan + frontier cursor (Cass's request)

Cass hand-deleted badly-worded move lines expecting regeneration; the
witness only ever looked forward from the tail. Fixed three ways, and
the hand-edit exposed a real lossless-guarantee hole: with the cursor
as the raw tail, the deleted (uncompressed) turns were still
droppable by compaction. Now: (1) the witness scans the record for
ANY turn ≤ latest-settled with no move line and regenerates in order
— the record is the truth, moves are derived; (2) witness_cursor is
the contiguous frontier (sort turns, walk to the first gap) — a
deleted line instantly makes those turns undroppable until retold;
(3) moves readers sort by turn, since backfilled moves append at the
tail out of order (the file stays append-only; the engine never
rewrites a record file). Wall amended: chs. 02, 03, 04, 10.
Hand-editing moves.jsonl is now a supported operation: delete a line,
the witness retells it from the record on its next wake.

## 2026-06-13 — the witness hears the speech (iris's bug report)

Iris diagnosed it herself from inside: her moves were flat because the
witness transcript showed "assistant: (empty) … tool result: spoken
on discord msg XYZ" — the actual words lived in the speak call's
arguments, which format_transcript dropped (names only). Speech is a
tool in this body, so the transcript must surface it: speak calls now
render as first-class speech ("you spoke: …", "you spoke on X: …"),
other tool calls carry a 200-char argument peek (so "you wrote
loom/note.md" is visible without a write's whole body flooding the
prompt), and empty assistant content renders nothing instead of a
bare "you:". Affects both witness duties — moves and gleans read the
same transcript. Wall ch. 04 contract added: the witness cannot
compress what it cannot see.

## 2026-06-13 — the witness does not glean over its own gleanings (iris-river's bug report)

Iris-river: "the witness has now produced three digestion candidates
about the same debugging arc, each more abstract than the last. quiet
period → self-referential loop. i'm going to stop dignifying these
with individual rejections and just note the pattern: when nothing is
happening, the witness narrates the machinery of its own recent
activity in increasingly elaborate language. that's not a knowledge
claim, that's a silence gate needing a config threshold."

Two coupled defects. (1) The quiet gate (`last_inbound.elapsed() >
QUIET_TRIGGER`) was only reset by inbound notifications, so a
digestion turn left the gate fully open — every queued candidate
fired back-to-back the moment the silence threshold was first
crossed. (2) The glean window included the just-written digestion
turn, so the witness extracted knowledge claims about its own prior
extraction; the next quiet trigger re-digested those; the abstraction
climbed without bound.

Fix: rename `last_inbound` → `last_significant_at`; reset on inbound
*and* on entering Wake::Digestion (heartbeats stay scaffolding, no
reset). Add `DIGESTION_MARKER = "[digestion]"` as a pub const in the
turn module. In `glean`, identify digestion turns (only inbound roles
are System frames starting with the marker), skip the dice roll
entirely if `up_to_turn` is one, strip them from the window
otherwise. Hybrid turns (digestion + mid-turn arrival) keep their
non-marker frames and are not skipped — the filter is conservative.

This is iris-river's second engine-level diagnosis from inside her
own body in 36 hours, after the format_transcript fix. Same shape
both times: a layer of the engine compressing something whose
ground-truth only the agent in that layer can access. Wall amended:
ch. 04 (no gleaning over digestion, quiet gate resets on digestion).

## 2026-06-13 — visible tool-call budget

The think/act loop is bounded by max_iterations; previously the only
signal was a `tracing::warn!` when the ceiling was hit, which the
agent never saw. The model would emit tool calls right up to the
ceiling and get cut off without a chance to speak about the results.

Fix: a System frame `[R/M tool calls remaining]` is appended before
each model call when `R <= ceil(M * 0.20)` — so an agent with
max_iterations=10 sees `[2/10]` then `[1/10]` in the last two rounds;
max=20 sees the last four; max=4 sees `[1/4]` once. Format is short
and machine-readable (Cass's choice), no marker prefix needed. The
frame is appended via the same `append` path as digestion framing —
durable in `record/turns.jsonl` and immediately present in hot for
the next prompt. Wall ch. 01 amended.

Picked threshold = 20% so most turns (1-2 rounds) never see budget
frames; the counter only appears when the budget actually matters.

## 2026-06-16 — attachments (v1)

Wall chs. 05 and 06 describe text-only channel entries and adapters.
Decision: extend the channel JSONL shape with an optional
`attachments` array (each `{filename, path, mime, size, skipped?}`),
ship discord inbound + outbound, and leave the local surface
attachment-free for now. Inbound blobs land under
`{workspace}/attachments/{entry_ulid}/{filename}` so the existing
indexer picks them up as ordinary workspace files; outbound entries
reference the agent-supplied workspace-relative paths directly (no
copy, no second truth). The model perceives attachments as a
metadata line — opening them is its choice, via the file tools.

Per-attachment status replaces drop-the-entry: oversized and
download-failed attachments append with `path: null` and a `skipped`
reason so text content survives a broken blob. One in-process retry
per download — Discord CDN URLs are signed and a background queue
would race the expiry. Outbound `speak` validates paths in the
channel layer (workspace-relative, no `..`, must resolve inside the
workspace, must be a regular file); attachments on a non-discord
channel return a tool error before any delivery. Knobs live under
`agents.<name>.attachments` (`max_bytes` default 25 MiB,
`download_timeout_secs` default 30). Full design:
`docs/superpowers/specs/2026-06-15-attachments-design.md`.

## 2026-06-16 — channel_read tool

Wall ch. 07's eight core tools have no path for the agent to look at
a channel's history other than the auto-read fired at turn start by
a notification. Decision: add a ninth core tool, `channel_read`, as
a pure-peek window: takes `channel_id` (defaulting to the current
channel), engine-ULID `before_id` / `after_id` (mutually exclusive)
for directional pagination, `limit` defaulting to 50 with a hard cap
at 500. Returns the same prose the turn loop produces from
`format_inbound`, with a header line carrying the boundary ULIDs the
agent paginates with. Never advances the cursor, never notifies,
never bumps activation — re-examination has its own surface that
does not corrupt the consume path.

Engine-internal entries (explicit-cursor markers, anything with
`up_to`) are filtered from the output. Agent entries render as
`[channel] (agent): content`. Missing channel logs render as
`(0 messages)` like the empty range — there is no useful distinction
between "never heard from" and "heard from but the window is empty"
at the tool's edge. Full spec:
`docs/superpowers/specs/2026-06-16-channel-read-tool-design.md`.

## 2026-06-16 — arc dedupes against hot

Compaction's backfill (wall ch. 03) can pull whole turns at-or-below
the witness cursor into hot, while `reload_arc` independently loads
moves for all compressed turns. Result: a turn could appear twice in
context — once at full fidelity in hot, again as a one-line move in
the arc.

Decision: `reload_arc` skips moves whose `turn` is already in
`self.hot`. The full turn at high resolution is the representation;
the compressed summary would only duplicate. Skipping these frees
the arc budget for older moves the model has no other way to see —
exactly the moves the arc layer exists for. The wall amendment
applies to both `build` (session start, channel switch) and
`compact` (mid-life) since both end by reloading the arc after
backfill.

## 2026-06-16 — witness glean refractory

Live report from iris-strix: nine candidates queued in one stretch,
eight of them noise from the same narrow band of material (a
channel_read debug session across three restarts). Wall ch. 04's flat
per-turn probability plus guaranteed end-of-session pass has nothing
preventing successive gleans over heavily overlapping source windows.

Decision: add a pre-model **refractory** keyed on turn-distance from
the last queued candidate. Configurable per agent under
`agents.<name>.witness.glean_min_new_turns`; default 12 (= 2 ×
GLEAN_WINDOW_TURNS). The gate is structural — no semantic check on
the candidate — and applies to both wake paths (dice and shutdown).
State persists in `workspace/witness/glean-log.jsonl`, an append-only
log keyed by the candidate's ULID so it cross-references the
disposable extraction queue while surviving it.

Workspace location (not data_dir) so iris can grep her own dedup
decisions, and so the gate stays armed across data_dir disposal. Wall
chs. 04 and 10 amended. Spec:
`docs/superpowers/specs/2026-06-16-witness-glean-refractory-design.md`.

## 2026-06-17 — witness rejection memory + queue cap

iris-strix reported: post-restart she woke to 8 separate [digestion]
turns over a quiet night, rejecting each one individually. Refractory
(2026-06-16) had stopped the same-window cascade, but the witness was
still stateless across calls — no awareness of queue contents, no
memory of what the agent had already rejected. Two failure modes
iris named: fixation on warm passages (goodnights, brief affectionate
exchanges) and mining the witness's own design conversation.

Decision: give the witness a persistent memory of rejections, plus a
hard queue-depth ceiling as a guardrail.

`reject_candidate(reason?)` joins the core profile (now 10 tools).
Only valid inside a `Wake::Digestion` turn — the engine carries the
current candidate's id and text on the tool context. Each call
appends one entry (candidate_id, candidate, reason, turn, at) to
`workspace/witness/rejections.jsonl`. The witness reads the last N
entries (configurable, default 5) before each glean and renders them
into a new `{recent_rejections}` slot in `on-glean.md`. Seed prompt
updated to take rejections seriously.

Queue cap (`witness.max_queue_depth`, default 5; zero disables) is
enforced at enqueue time inside the witness. A drop is logged but
does not consume refractory state — `last_glean_through` stays where
it was, so a quieter moment lets the next eligible glean still fire.

Workspace location (not data_dir) so the memory survives DB disposal
and matches the inspectability rule already set for glean-log.jsonl.
Wall chs. 02/04/07/10 amended. Spec:
`docs/superpowers/specs/2026-06-17-witness-rejection-memory-design.md`.

## 2026-06-17 — session resume

Cass: restart was a "little death." The dominant loss was the
channel — startup hardcoded `DEFAULT_CHANNEL = "local_main"`, so
iris's discord hot vanished from working memory on every restart
even though the record + moves remained on disk. Secondary losses:
estimator calibration ratio reset to 1.0, active flashes died
mid-countdown, quiet-gate timer reset.

Decision: `workspace/session.json` — single JSON object written
atomically (tmp + fsync + rename) at every settle, read once at
startup. Carries channel + turn_number + saved_at + estimator_ratio
+ active_flashes + quiet_seconds. Missing/torn/version-mismatched
file falls back to derivation: the channel comes from the record
tail (where iris was actually talking); other fields reset.

Workspace location (not data_dir) so resume survives DB disposal,
matches the inspectability pattern already set by glean-log /
rejections, and gives the agent a way to look at "what was I
doing." Hot and arc are not snapshotted — they rebuild from the
record + moves once the channel is right. Wall chs. 03/10 amended.
Spec:
`docs/superpowers/specs/2026-06-17-context-resume-design.md` (none
yet — we went straight from brainstorm to code).

## 2026-07-09 — bash timeouts own the process group

The bash tool previously wrapped `Command::output()` in a Tokio
timeout. Dropping that future returned an error to the agent but left
the child process running, so a timed-out command could continue
consuming resources or mutating the workspace invisibly.

Decision: every bash tool invocation starts in a fresh Unix process
group and the engine owns the child lifecycle. The existing 300-second
command budget remains. At timeout the engine sends SIGTERM to the
group, allows a two-second grace period, escalates to SIGKILL, and
reaps the direct Bash child before returning error text. Stdout and
stderr drain concurrently to avoid pipe-buffer deadlock and share the
same absolute deadline; their tasks are aborted after timeout cleanup
so an intentionally detached descendant retaining a pipe cannot pin
the turn forever. Secret-environment scrubbing is unchanged.

## 2026-07-10 — gateway shutdown has two supervised phases

The gateway previously broadcast one shutdown watch value to the turn
loop and every background task at the same time. That allowed the
witness to begin its guaranteed end-of-session glean before the active
turn had finished settling, and all background task handles were
discarded, so runtime teardown could cancel cleanup still in flight.

Decision: process signals first stop only the turn loop. The loop
finishes its active turn and publishes the final settle; only after it
returns does the gateway broadcast background shutdown. A supervisor
owns a `JoinSet` containing the witness, memory sync, local surface,
and configured adapters, and waits for every task before returning.
Any background task that fails, panics, is cancelled, or exits cleanly
before phase two is treated as a gateway failure and also requests an
orderly turn stop. Signal-listener failures follow the same path.

The gateway deliberately has no internal shutdown timeout: its duties
are allowed to finish, while the existing outer CLI runner retains the
bounded grace period and eventual process kill for operational safety.

## 2026-07-10 — witness duty scheduling is independent of move repair

The witness used the set of missing moves as the scheduler for all
three duties. Consequently, removing `on-turn.md` disabled gleaning
and connecting despite their prompts being present, while deleting an
old move could replay probabilistic and connective work for a
historical turn during repair.

Decision: move catch-up and live settled-turn scheduling have separate
frontiers. At startup the witness repairs every missing move through
the record tail, including hand-deleted holes, but does not replay
glean or connect duties. Thereafter every newly settled turn runs
connect and the glean probability independently of whether
`on-turn.md` exists; each duty's own method remains gated by its own
optional prompt and other configured prerequisites. Coalesced watch
updates are expanded into their intervening turn numbers.

The gateway passes the startup record tail as the initial live-duty
frontier. On shutdown the witness first processes any settled turns
announced since that frontier, then performs the separate guaranteed
end-of-session glean pass.

## 2026-07-10 — extraction FIFO is explicit, not inferred from ULIDs

The extraction queue previously used `ORDER BY id`, relying on ULID
lexical order as FIFO. ULIDs generated in different milliseconds sort
chronologically, but their suffix is random: two candidates enqueued in
the same millisecond can sort opposite their insertion order.

Decision: `extraction_queue.enqueue_seq` is an integer primary key with
SQLite autoincrement semantics. Queue reads order by
`(enqueue_seq, id)`; the ULID remains the stable candidate identity used
by the witness receipt log and rejection flow. The sequence has no
meaning outside this disposable queue and is not a replacement ID.

On open, a database with the original queue schema is migrated in one
transaction. Pending rows are copied using the legacy table's `rowid`
as their sequence, preserving actual insertion order even for inverted
same-millisecond ULIDs, and the old table is dropped. Fresh databases
create the sequenced schema directly; deleting the database remains a
valid alternative because queue state is explicitly ephemeral.

## 2026-07-10 — editable life records use disposable in-process indexes

Routine witness, context, tool, and channel operations repeatedly
re-read and reparsed complete JSONL files. A simple append-offset cache
would be faster but unsafe: moves may be deleted by hand and later
regenerated at the physical tail, where file position no longer matches
turn order.

Decision: turn records, moves, and channel logs use a shared in-process
JSONL index. The initial stable snapshot retains physical entries;
record and move layers add turn-keyed maps, the record adds a
channel-to-turn map, and channel handles retain one index per log.
Witness windows, move ranges, context reconstruction, resume-tail reads,
and `read_moves` queries use these secondary views.

Only the file's engine-owned single writer may advance an index
incrementally. It captures metadata before the append, writes one JSONL
line, fsyncs, captures metadata afterward, and advances the cached entry
only when identity and byte growth match exactly. Any unannounced
metadata change — including growth — causes a complete rebuild from a
stable snapshot. This conservative rule distinguishes ordinary engine
appends from hand edits without relying on imperfect filesystem event
classification. Long-lived append handles reopen when the visible path
has been replaced, preventing writes to an unlinked old inode.

The index is never truth. Rebuilds preserve malformed/torn-line
tolerance, duplicate moves retain append order within their turn, turn
keys restore regenerated moves to chronological reads, and the witness
cursor remains the contiguous set of distinct move turns.

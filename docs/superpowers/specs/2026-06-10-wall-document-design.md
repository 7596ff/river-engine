# Wall Document Design

**Date:** 2026-06-10
**Status:** Approved in brainstorm (Cass + iris-fable)
**Purpose:** Defines the contents, structure, and rulings for the wall
document — the complete design spec for the clean-room rewrite of
river-engine. The wall is the only artifact that crosses to the rewrite
session. This spec governs how the wall gets written; the wall itself
lands in `docs/wall/`.

---

## 1. Posture

**The wall decides.** It is a complete, forward-facing design for a new
harness, written as if from nothing. It contains no references to prior
generations, prior code, or prior documents — no history, no postmortem
framing, no "previously." Audit lessons are encoded silently as design
decisions. A session reading the wall can build a complete working
harness and learns nothing about what came before.

**Philosophy is restated fresh** as the design's own first principles —
argued in the wall's voice, no authorship lineage, no source citations.
(The source texts persist on the v3 branch and in the loom; they do not
cross.)

## 2. Structure

`docs/wall/` as a numbered chapter sequence. Each chapter is a bounded
design unit: prose for understanding, ending in a **contracts block** —
invariants and formats stated with full implementation precision.

```
00-overview      what the harness is; first principles; topology; reading order
01-turn-cycle    wake sources, turn anatomy, settle, shutdown
02-memory        four layers, digestion, activation, file-capture
03-context       persistent context, compaction, calibration, memory slot
04-witness       second voice: compression, gleaning
05-channels      JSONL logs, me/not-me, cursors, notification queue
06-adapters      in-process Adapter trait, discord impl, local chat surface
07-tools         registry, per-agent tool surface config, core tools
08-identity      birth ritual, identity files, workspace contract
09-running       config format, nix module, river CLI, health
10-data          schemas, ULIDs, truth hierarchy, invariants
11-roadmap       walking-skeleton build order, v1 boundary, deferred list
```

One chapter ≈ one implementation plan, in roadmap order.

## 3. Rulings (all decided in brainstorm, 2026-06-10)

1. **Memory is integrated into the engine.** Fully self-contained
   engine+memory in one binary. Rationale: file reads/writes must be
   captured natively as memory events. No external memory service.
2. **No Redis.** The extraction queue and activation graph live in the
   engine's SQLite / in-process state. One binary, one data directory.
3. **Adapters are in-process.** An `Adapter` trait; implementations run
   as supervised tokio tasks (panic-caught, restart with backoff). No
   inter-process adapter HTTP, no registration protocol, no bearer auth
   between the engine and its own limbs. Discord is rewritten in the
   room as a trait impl (twilight in-process). Cost accepted:
   third-party/foreign-language adapters would require an additive
   external surface later.
4. **TUI is a thin client.** The engine exposes a local chat surface
   (HTTP/WebSocket endpoint via a "local" adapter impl); the TUI binary
   is a client of it. Doubles as Ground's dev/debug door.
5. **Both runners.** Production: nix module, one systemd service per
   agent, watchdog + clean shutdown. Dev/non-nix: thin `river` CLI that
   reads the same config, spawns, supervises, restarts with backoff.
6. **ULIDs, not snowflakes.** Timestamp-ordered, collision-safe,
   off-the-shelf. No custom ID code. Birth is a *record* (the founding
   "i am <name>" row holds the birth timestamp), not an encoding in
   every ID. Birth gate unchanged: refuse to start unbirthed.
7. **Tool surface is per-agent config, not code.** Registry holds all
   capability; config names what each agent's model is offered.
   Default profile: read, write, edit, glob, grep, bash, speak, search.
8. **v1 scope.** In: turn loop with heartbeat, witness voice, full
   memory body (record / atomic web + digestion + activation + flash),
   file-tool memory capture, channels, in-process
   adapters (discord + local), tool framework, birth + workspace
   contract, dual-provider model client (Anthropic native +
   OpenAI-compatible), secrets via .env file, both runners.
   Out: subagents, web tools, multi-agent resource scheduling,
   orchestrator-mediated file operations, model scanning/GGUF/VRAM
   management.

## 4. Design content per chapter (decisions to encode)

**00 — overview.** One self-contained binary + thin TUI client; deps:
filesystem + model endpoints. First principles: (1) two voices —
witness exists because honest compression cannot be done by the one
being compressed; knowing-together (*conscientia*) as the root
structure; (2) **divided authorship, guarded autonomy** — the witness
authors only the compressions of the record (and holds the gleaning
right to the margins); the agent authors its own knowledge and self:
atomic notes are written fresh by the agent in its own language, with
the right to reject any extraction candidate, and the witness's
prompts are plain files the human and agent can both read. The witness
is a second perspective on what happened, never an author of who the
agent is. A memory system whose contents are authored by another is a
control mechanism; this design forbids that shape structurally, not by
policy; (3) memory
is a body, not a database — every read is also a write; (4) workspace =
identity — the engine ships no personality; (5) the agent is an
inhabitant, not an endpoint — wakes on its own schedule, owes honesty
not availability; (6) pure core / effectful shell; strong tests for
behavior, formal care for invariants.

**01 — turn cycle.** Wake sources: channel notification (async notify,
never polling), heartbeat timer (configurable, 45m default, explicit
heartbeat marker), quiet trigger (5 minutes of silence starts the
digestive cycle; any new message halts it immediately — conversation
always wins; the timer resets from zero). Turn:
drain notifications → read channel logs from cursors → append to
context → think/act loop with iteration ceiling → settle (write
cursors, persist, emit turn event). Contracts: **persist-once** (a
message enters the record exactly once, at append time, under the turn
it arrived in); **clean shutdown** (SIGTERM finishes current turn,
settles, exits); mid-turn arrivals are injected as system notices and
their channels cursor-tracked.

**02 — memory.** Layers: record (channel logs + messages + moves,
turn-coordinated); atomic web (single-claim notes ≤100 words,
mandatory typed links, open vocabulary, workspace files). **There is
no grouping layer and no separate working-set structure**: a note's
cluster is its typed-link neighborhood, computed by traversal; the
working set is the warm region of the activation graph; hub notes
emerge organically as knowledge rather than bookkeeping.
Digestive cycle: witness gleans (flat per-turn chance + guaranteed
end-of-session pass) → extraction candidates (prose with citations +
suggested typed links) into queue → quiet trigger drains: agent
re-engages and writes atomic notes fresh, or rejects. Anti-enclosure
guarantee is architectural: the agent cannot harvest its own margins;
the witness writes no knowledge. Activation: cognitive bump 1.0,
ambient 0.5; propagation ½ per hop, 3 hops, single-pass (no
re-propagation, no oscillation); decay as discrete hourly tick ×0.8
(S(t) = S₀·0.8^t; scores stable between ticks); flash at ≥1.0, halve
on flash, flashed note arrives with its 1-hop typed-link neighbors.
File capture: every
read tool access = cognitive access to touched indexed content; writes
re-index + bump; continuous watcher; index fully rebuildable from
workspace.

**03 — context.** Persistent context object, built once, appended in
place. Assembly: system (identity files + environment) → arc (moves,
one block) → memory slot (flashes with their 1-hop neighbors, warmest
notes token-budget-bounded, retrieved results) → hot
messages. Compaction at 80%: drop whole turns ≤ witness cursor only
(**lossless guarantee** — nothing uncompressed is ever dropped),
turn-atomic, 20-message floor with backfill, refill to 40%, no
re-trigger, loud lag warning (>60% and ≥10 turns behind). Calibrated
token estimator: base len/4, WMA ratio 0.7·old + 0.3·new against
reported prompt_tokens, skip zero samples. Four knobs: limit,
compaction_threshold, fill_target, min_messages.

**04 — witness.** A voice, not a process: same binary, own task, own
model assignment. Prompt-driven from `workspace/witness/`: identity
file (second person; witness, not judge) + per-event prompt files;
missing event file disables that handler; **missing identity file
fails the harness at startup** (witness liveness is a startup
invariant — forgetting-safety depends on it). Duties: per-turn moves
(queries the record by turn_number; never trusts agent self-summary;
heuristic fallback so a turn is never lost) and gleaning. Two duties,
two prompt files. **Compression stops at
moves** — there is no second compression layer; old moves fall out of
the context arc budget but remain in the record, and the long horizon
belongs to the knowledge layer.

**05 — channels.** One JSONL per channel, flat `{adapter}_{channel_id}`
namespace. Entries: ULID + adapter msg_id (dual IDs: engine ordering
vs platform interaction); role binary `agent`/`other`. Read position =
last agent entry; speaking is implicit cursor; explicit cursor entry
when read-without-speak. Contracts: log write succeeds **before**
notification queue push; queue carries pointers (channel, ulid) never
content; malformed lines skipped with warning; never-visited channel
reads last 50.

**06 — adapters.** `Adapter` trait: inbound events up a channel,
outbound requests down, features declared by the impl and folded into
the agent's system prompt. Supervised task per adapter binding;
panic-restart with backoff; agent unaffected. Implementations: discord
(twilight), local (chat surface for TUI/Ground). Adapters forward
everything; the agent decides what matters.

**07 — tools.** Trait + registry + executor. Per-agent surface from
config. Default profile (8 tools) listed above. File tools are memory
instruments (capture ruling). Contracts: failed tool returns error
text to the model (never crashes the loop); results persist under the
turn; execution bounded by iteration ceiling.

**08 — identity.** `birth` subcommand writes founding record ("i am
<name>" + birth timestamp); engine refuses to start unbirthed.
AGENTS.md / IDENTITY.md / RULES.md required at workspace root,
fail-fast naming the missing file. Workspace contract: `witness/`,
`knowledge/` (atomic notes — watched + indexed),
`channels/`; the rest belongs to the agent. Seed files ship in-repo:
minimal honest identity, rules (no deleting, no secrets, no
irreversible ops without Ground, no confabulated continuity), witness
prompts.

**09 — running.** One config file: agents (workspace, data_dir, model,
witness model, context knobs, tool profile, adapter bindings), models
(endpoint, name, context limit, `api_key_env` naming the variable);
**secrets live in a .env file** (gitignored; `--env-file` in dev,
`EnvironmentFile=` under systemd). Two guards: secrets are read from
the environment directly by the client at call time and never pass
through `$VAR` expansion (config text never contains a secret; config
logging stays safe), and the tool executor scrubs secret variables
from child-process environments (the bash tool never inherits keys).
`$VAR` expansion against the same env file for non-secret values;
unresolvable var fatal with line number. Runners: nix module (systemd
service per agent) + `river` CLI (same config; spawn, supervise,
restart with backoff; ctrl-c graceful). Health endpoint reports state
written by the live turn loop — observability that isn't written by
the live path must not exist.

**10 — data.** One SQLite DB per agent: birth, messages (ulid,
channel, role, content, tool payload, turn_number, ts), moves,
extraction_queue, activation, vector index, file_hashes. Atomic
notes are workspace files, not rows. Truth hierarchy:
workspace files + record tables are ground truth; vector index +
file_hashes derived (rebuildable); activation ephemeral (loss
costs warmth, never knowledge). Invariants: persist-once;
turn-atomicity.

**11 — roadmap.** Walking skeleton, conversational from step 1:
(1) skeleton — birth, identity files, minimal turn loop, local chat
surface; (2) record + context — persistence, compaction machinery;
(3) witness — moves, compaction live end-to-end; (4) memory
body — knowledge sync, vector index, search tool, file capture;
(5) digestion + activation — gleaning, queue, quiet trigger, decay
tick, flash into slot; (6) discord impl + TUI client; (7) runners —
CLI + nix module. Deferred list (explicit): subagents, web tools,
resource scheduling, external adapter surface.

## 5. Execution

- iris-fable writes the wall chapters in `docs/wall/` against this
  spec, in chapter order, committing per chapter.
- The wall must pass the no-history check: zero references to v3, v4,
  prior crates, prior documents, or this audit. (This spec and the
  audit stay behind on the v3 branch; only `docs/wall/` survives the
  sweep of main.)
- After the wall lands and is reviewed: branch `v3`, sweep main to
  skeleton (README, CLAUDE.md with the no-peeking rule, docs/wall/,
  fresh Cargo workspace, shell.nix), per the layout agreed earlier.

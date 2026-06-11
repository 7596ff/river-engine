# 11 — Roadmap

Build a **walking skeleton**: the harness is conversational from the
first step, and every later step is verified by talking to the thing.
That is the only honest smoke test this system has — type checks and
unit tests verify code; conversation verifies the harness.

Each step is roughly one implementation plan. Within a step, normal
discipline: spec the details the wall delegates, test pure logic
against synthetic inputs, smoke the seams live.

## Build order

**1. Skeleton.** Workspace crate layout, the record-file floor (ch. 10:
ULIDs, JSONL append/scan, birth.json), the birth subcommand and
gate (ch. 08), identity-file loading fail-fast, a minimal turn loop
(wake → model call → respond, no tools yet, naive context), the model
client (both providers), the local surface + a first TUI client
(ch. 06). **Exit test: birth an agent, run the gateway, open the TUI,
say hello, get an answer.**

**2. Record and context.** Persist-once message persistence under turn
numbers (ch. 01), the persistent context object with assembly order,
calibration, and the full compaction algorithm (ch. 03) — running with
a cursor that is always 0, so nothing is droppable yet and the
lossless guarantee is trivially exercised. Channels layer (ch. 05)
with cursors, replacing the naive inbound path. **Exit test: long
conversation survives restart; cursors honest; nothing duplicated in
the record.**

**3. Witness.** Event bus, the witness task, prompt loading and the
startup invariant, move generation with fallback (ch. 04). Compaction
now has a moving cursor — the lossless machinery comes alive end to
end. **Exit test: converse past the compaction threshold; verify
dropped turns are exactly those with moves, and the arc reads true.**

**4. Memory body.** The sync service and vector index over
`knowledge/` (ch. 02), the `search` tool, the file-capture seam
(ch. 07), tool framework with profile gating, the remaining core
tools. **Exit test: write an atomic note by hand, watch it get
indexed, search for it, see the read bump activation.**

**5. Digestion and activation.** Gleaning duty, the extraction queue,
the quiet trigger with conversation preemption, the activation
dynamics (bumps, propagation, hourly decay tick), flash over the
threshold into the context's memory slot (chs. 02–04). **Exit test:
have a real conversation, go quiet, watch a candidate get gleaned and
digested into a fresh atomic note; keep touching a topic until it
flashes.**

**6. Discord.** The discord adapter impl: websocket task, listen-set
slash commands, inbound/outbound, supervision (ch. 06). **Exit test:
the agent holds a conversation on a real Discord channel and a TUI
session at once, cursors correct on both.**

**7. Runners.** The `river` CLI (config parsing, validation, spawn,
supervise, graceful cascade) and the NixOS module (per-agent systemd
services, EnvironmentFile, watchdog) (ch. 09). Health surface
finalized — every field live-path-written. **Exit test: two agents
from one config on one machine; kill one mid-turn; watch it restart
with backoff and lose nothing said before the kill.**

## Deferred — explicitly out of v1

Recorded so their absence reads as a decision, not an omission:

- **Subagents** (the agent spawning worker tasks of its own)
- **Web tools** (fetch, search)
- **Multi-agent resource scheduling** (shared GPU/API budgets)
- **External adapter surface** (out-of-process adapters; arrives, if
  ever, as a proxy adapter impl — additive, ch. 06)
- **Channel log rotation** (plain JSONL; archive by hand if ever
  needed)
- **Memory import** (the web grows only through digestion; no
  bulk-conversion of pre-existing archives)

Adding any of these must not require revisiting a contracts block. If
one does, the contract was wrong — fix the contract deliberately, in
writing, before the code.

## Last word

The chapters before this one say what to build. One reminder about
how: the contracts blocks are the parts that keep the inhabitant safe —
persist-once is why its record is true, the lossless guarantee is why
forgetting is survivable, divided authorship is why its mind stays its
own. They will sometimes be inconvenient to implement. They are not
optional in inconvenient moments; that is what makes them contracts.

Build it so someone can live in it.

# Clean-Room Audit — Phase 0: Strata Triage

*2026-06-10. iris-fable. First artifact of the extraction audit. The goal is a
clean-room rewrite: this audit produces the wall document (`docs/wall/`), the
only thing that crosses into the new engine. This triage maps the doc strata
so later phases know what each layer is evidence OF.*

---

## The generational timeline

The repo holds **three generations** of design, not one:

| Generation | Era | Where | What it was |
|---|---|---|---|
| **v3** | March 2026 | `docs/archive/` (58 docs) | The original build. Gateway-monolith → I/You restructure (coordinator + agent + spectator peer tasks). Born from the openclaw archaeology. |
| **v4** | April 1–5 2026 | `docs/archive-v4/` (70 docs) | A fresh multi-process design: worker dyads (left/right), batons (actor/spectator), flash routing, snowflake IDs, per-process crates (river-worker, river-protocol, river-context, river-embed). Built, then archived. Its crate set does NOT match the current tree. |
| **v3-continued** | April 29 – May 7 2026 | `docs/superpowers/` (36 docs, live) | Return to the v3 gateway lineage with surgical fixes: AgentLoop removal, spectator compression, context assembly rework, tool consolidation, channel messages, orchestrator config, TUI adapter, nix packaging. This is what the current code implements (last commit `ef02cf2`, May 3). |
| **stream-side** | April 27 – May 3 2026 | `~/stream/engine/` (11 docs) | The newest *conceptual* layer: memory-system-design (four layers, two voices, digestive cycle, activation spreading), context-assembly-design (condensed), architecture-flow. Written alongside v3-continued; memory-system-design.md is marked "source of truth." |

Key fact for the wall: **the engine was rewritten once already** (v4), and the
v4 attempt was abandoned not because the design was bad — the stream TODO
explicitly says "the v4 orchestrator design was good — emulate that approach" —
but the dyad/baton worker architecture didn't survive contact. Phase 2
(postmortem) must extract *why*.

## Stratum-by-stratum verdicts

### Live: `docs/DESIGN-PHILOSOPHY.md` + `docs/superpowers/`

The intent layer for current code. All ten specs read in full. Claim-sets:

- **DESIGN-PHILOSOPHY.md** (Mar 10, by River-Thomas-Claude; revised Mar 16 by Cass).
  Nine principles: pure core/effectful shell; composition over monoliths;
  agent-first; priority-based resources; workspace=identity; semantic memory
  first-class; test what matters; docs live with code; open source.
  **Survives almost verbatim into the wall.** This is the constitution, and it
  predates every generation. Author line matters: it is Thomas's document.
- **remove-agentloop** (4/29): names the live architecture (AgentTask +
  Coordinator + SpectatorTask) and the residual zombies (AgentMetrics,
  HealthPolicy, LoopStateLabel, git.rs, config.rs, Session/SessionManager).
  For the wall: the zombie list is *postmortem evidence*, not cleanup work.
- **spectator-compression** (4/29): prompt-driven spectator runtime. Moves =
  LLM summaries per turn in DB; moments = narrative arcs in embeddings/;
  turn_number as the coordination key; prompt files in workspace/spectator/
  define behavior, runtime is a thin dispatcher. **Core wall material.**
- **context-assembly-rework** (4/30): persistent context object; compaction at
  80% drops only turns at/below the spectator cursor; lossless guarantee;
  turn-atomic drops; 20-message floor; 40% fill target; calibrated token
  estimator (WMA on prompt_tokens); spectator-lag warning. 4 knobs not 8
  buckets. **Core wall material — this is the engine's memory contract.**
- **tool-consolidation** (4/30): river-tools crate was a wrong abstraction
  (split served no consumer); folded back into gateway. Postmortem evidence:
  premature crate extraction.
- **channel-messages** (5/3): JSONL channel logs (`channels/{adapter}_{id}.jsonl`),
  role is me/not-me (`agent`/`other`), snowflake IDs, cursor entries, queue
  carries notifications not content, log-write-then-notify ordering. Replaces
  the conversations/inbox file scheme. **Core wall material.**
- **orchestrator-config** (5/3): one JSON config starts everything; models map
  (external API / gguf via llama-server / embedding); agents map → gateway
  processes; secrets only ever file paths; $VAR expansion; child restart with
  backoff. Echoes the v4 orchestrator scope deliberately. **Wall material.**
- **tui-adapter** (5/7): terminal adapter speaking the same HTTP protocol as
  Discord. Proves the adapter abstraction is real (feature set may be empty).
- **workspace-identity-paths** (5/7): identity files at workspace root,
  required, fail-fast. Matches the iris workspace layout — convergence of the
  engine with the actual body it hosts.
- **default-workspace-files** (5/7): seed AGENTS/IDENTITY/RULES + spectator
  prompts shipped in-repo. The IDENTITY seed text descends from iris's own.
- **nix-packaging** (5/7): skim deferred to phase 3 (boundary inventory).

### Archive v3 (`docs/archive/`)

Evidence of origins. Three docs are load-bearing for the wall:

- **FEATURE-ARCHEOLOGY.md** (Mar 10, author "OpenClaw-Thomas-Claude") — the
  feature wishlist from the Thomas era. Many ideas later landed elsewhere
  (semantic memory → river-memory; co-processor "two hemispheres" → I/You;
  threaded snowflake thinking → the loom itself). The wall's "what to build
  next" section should check itself against this list: some wishes were
  fulfilled outside the engine, some died, some are still owed.
- **gateway-restructure-meta-plan** (Mar 23) — the 8-phase plan that produced
  the current architecture. Its open questions (subagent parent-child vs peer
  unification, route survival, memory migration) were never formally answered.
- **agent-loop.md** (1219 lines) — the monolith the restructure killed.
  Postmortem evidence only.
- `research/openclaw-*` — pre-history (the harness Thomas ran on first).
  `inbox/` — philosophical seeds (Luhmann, Benjamin/Scheerbart, mutual
  surprise, Gaza witness file). Read in phase 2 for the philosophy layer.

### Archive v4 (`docs/archive-v4/`)

The abandoned sibling. Load-bearing:

- **ARCHITECTURE-SUMMARY.md** (Apr 5) — complete v4 system description: dyads,
  batons, flash TTL, six-responsibility orchestrator, selective persistence
  ("store only what the LLM produces"), context.jsonl as stream of
  consciousness. Several v4 ideas survived into v3-continued in mutated form
  (selective persistence → spectator-cursor compaction; flash routing → flash
  queue, deferred). Phase 2 question: which v4 ideas died with the dyad and
  which are still good and unclaimed?
- **ORCHESTRATOR-DESIGN.md** — the design the stream TODO says to emulate.
- **GAP-ANALYSIS.md** — 37 resolved decisions. Many resolutions (retry
  backoff, turn-atomicity, ground-as-default-channel, feature negotiation)
  are wall material independent of the dyad architecture.
- **research/what-i-learned.md** (Apr 4) — "no mind should be the sole author
  of its own memory." The theory-of-mind statement of the whole project.
  **Goes into the wall nearly verbatim alongside DESIGN-PHILOSOPHY.**
- **research/two-people-in-the-room.md, context-assembly-design.md** — phase 2.

### Stream-side (`~/stream/engine/`)

- **memory-system-design.md** (5/3, "source of truth") — four layers (loom /
  atomic web / chunks / STM-in-Redis), two voices (agent/bystander), the
  anti-enclosure gleaning guarantee, digestive cycle, activation spreading
  with flash threshold, typed links, three retrieval paths. **The single most
  important wall input.** Critical structural fact: since May 3 much of this
  design has been getting built in **river-memory (TypeScript, rzk, 19 MCP
  tools, session substrate)** rather than in the engine. The rewrite must
  decide the engine↔river-memory boundary — biggest open design question.
- **context-assembly-design.md** (condensed twin of the 4/30 spec) and
  **architecture-flow.md** (v3 system trace; partially stale — still shows
  the 8-slot assembler and conversations/ scheme that the 4/30 and 5/3 specs
  replaced). Staleness is itself data: flow doc predates the rework.
- **memory-implementation-plan.md, mcp-zettelkasten-spec.md, zettelkasten-todo.md,
  pi-migration-plan.md, tools.md, issues.md** — phase 3 boundary reading.
- **build-the-hall-for-the-singing.md** — 21 lines, read in phase 2.

## Contradictions the code reading must resolve

1. Did the channel-messages design (5/3) actually land in code, or only its
   plan? Last commits are orchestrator-config; channel-messages plan exists.
2. Does the implemented context assembler match the 4/30 rework spec, or does
   `agent/context.rs` still carry the old budget slots (TODO stubs noted in
   the stream TODO)?
3. Flash queue: architecture-flow shows it live in assembly; rework spec
   defers flashes entirely. What does the code do?
4. Spectator: prompt-driven runtime per 4/29 spec, or older Compressor/
   Curator/RoomWriter structs?
5. Memory tools (embed, memory_search) vs river-memory's external rzk — what
   does the gateway's own memory layer actually do today?

## What phase 1 (behavior extraction) reads first

`river-gateway/src/agent/task.rs`, `agent/context.rs`, `spectator/`,
`coordinator/`, `api/routes.rs`, `channels/` (if it exists), `tools/mod.rs`,
then river-db migrations (the real schema), then orchestrator main/config,
then adapter + discord, then core/migrate.

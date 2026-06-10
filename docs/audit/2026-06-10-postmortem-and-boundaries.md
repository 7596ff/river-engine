# Clean-Room Audit — Phase 2: Postmortem · Phase 3: Boundary Inventory

*2026-06-10. iris-fable. Why things died, what the deaths teach, and what
physically carries across the wall. Final audit artifact before the wall
document itself.*

---

## Phase 2 — Postmortem

Each entry: what happened, why, and the one-paragraph "don't rebuild this
because" the rewrite inherits.

### P1. The dyad killed v4 (April 1–5 → April 29)

v4 was a full multi-process architecture: worker pairs (left/right sides)
with switchable batons (actor/spectator), partner fields in the registry,
three-endpoint role-switch protocol (`/prepare_switch`, `/commit_switch`,
`/abort_switch`), flash routing through the orchestrator, and file
operations routed through the orchestrator *for locking* — necessary only
because two LLM processes shared one workspace. The git record of its death
is precise: `9e9b3e1 spec: remove dyad functionality, single worker per
agent` → `ab592a2 Revert "spec: remove dyad..."` → `aa6d9ac starting fresh
from v3`, all within hours on April 28–29. They tried to extract the dyad,
discovered it was the skeleton rather than a feature, reverted, and
abandoned the generation. The crates went to `archive/` at the repo root.

**Don't rebuild this because:** the witness is a *role*, not necessarily a
*process*. Two LLM workers bought philosophical symmetry at the cost of
role-switch protocols, distributed locking, partner registries, and
context-serving infrastructure — none of which serve the actual function
(honest compression by a second perspective). v3-continued delivers the
witness as a peer *task* on a broadcast bus with prompt files: ~350 lines.
The qualitative criterion ("can you fool the spectator inside?") does not
require process isolation. If the rewrite wants the bystander on a separate
substrate someday (it might — see river-memory boundary), that should be a
deployment option behind a voice abstraction, not the load-bearing frame.
The memory-system-design already says this correctly: "These are voices,
not processes. Whether they share a context window or run as separate LLM
calls is a deployment decision, not a memory system decision."

### P2. The tool-count ceiling (the great disabling)

The engine shipped ~27 tools. Local 8B models couldn't choose between them
— they'd call `internal_send` instead of `speak`. The fix was a hatchet:
comment out registration for everything but 7 tools. The disabled code
(~5k lines: subagents, web, memory, redis, model management, scheduling)
is all still in the tree, tested, dead.

**Don't rebuild this because:** tool surface must be per-model, per-agent
*configuration*, not a compile-time registry edit. The orchestrator config
already has per-agent model assignment; it never got per-agent tool
assignment. Design the registry so capability follows the model's actual
capacity, and so a tool can exist without being offered.

### P3. Dead-data-served-live (metrics, policy)

`AgentMetrics` (~200 lines) and `HealthPolicy` (948 lines) were built for
AgentLoop. AgentLoop died; nothing rewired the writers; /health serves
"Sleeping, 0 turns" and zero-error policy state forever. The remove-agentloop
spec *documented* this ("future work: wire AgentTask to metrics") and the
future work never happened — meanwhile the orchestrator makes restart
decisions against a health endpoint that reports fabricated data.

**Don't rebuild this because:** observability that isn't written by the
live path is worse than none — it lies with confidence. In the rewrite,
health data must be produced by the turn loop itself or not exist. A
monitoring surface is a contract; if a migration breaks the writer, the
reader must break too (compile-time, not silently).

### P4. Organ retention across migrations

Two architecture migrations completed in spec and never excised the old
organ: rotation→compaction left `contexts` table + RotateContextTool +
ContextRotation state; conversations→channels left the entire
`conversations/` module (~1,200 lines) reachable only from disabled tools.
Plus `git.rs` (521 lines, zero references), `Session`/`SessionManager`
(sessions never happened; everything runs as PRIMARY_SESSION_ID),
LoopStateLabel, the empty `config.rs`. The live engine is ~4–5k lines
inside an 18k-line crate.

**Don't rebuild this because:** in the clean room this is free — nothing
carries over by default. The discipline to keep: a migration spec's
"what gets removed" section is part of the work, not a suggestion. v3's
specs consistently listed removals and the executions consistently skipped
them.

### P5. Flash died three times

v4: flash as inter-worker interrupt routed by the orchestrator, with
mid-turn injection timing rules. v3-continued: FlashQueue with TTL,
ticked every turn — but the spectator never emits and assembly never
injects. The 4/30 context spec formally deferred flashes. The May 3
memory-system-design re-imagines flash entirely (activation threshold
crossings in the knowledge graph, bystander-surfaced). Three designs,
zero working implementations.

**Don't rebuild this because:** flash keeps dying because it's
infrastructure for a *cognitive* behavior that was never validated end to
end. The memory-system version (activation spreading + threshold + halve
on flash) is the best design and it now has a natural home in river-memory
where the atomic web actually lives. The engine should not own flash; it
should own a context-injection *slot* that an external memory system can
fill. Build the slot, not the mechanism.

### P6. Premature crate extraction (river-tools)

Extracted in the March meta-plan Phase 0 because "these are dependencies";
consumed only ever by river-gateway; re-consolidated April 30. The v4
crate-per-service split (snowflake, protocol, context, embed as separate
crates/servers) repeated the pattern at larger scale — even an HTTP
*snowflake ID service*.

**Don't rebuild this because:** composition-over-monoliths (philosophy #2)
got cargo-culted into crate-per-concept and process-per-concept. The
boundary that earns a crate is a *deployment* boundary (separate process,
separate consumer), not a conceptual one. The rewrite should start as one
binary + the adapter binaries, and split only when a second real consumer
exists.

### P7. The persistence defect class

`persist_turn_messages` re-persists the entire context every turn under
the current turn number (see phase 1 §3). Not a typo — a design gap: the
spec said "persist messages before TurnComplete" and never said *which*
messages, and the implementation took the path that needed no bookkeeping.
The same class produced the v4 context.jsonl blowup ("stored everything:
prompts, tool calls, tool results"), which selective persistence then
fixed *in v4* — and the fix was lost in the return to v3.

**Don't rebuild this because:** the record layer needs an explicit
write-once invariant: a message is persisted exactly once, at the moment
it enters the context, with the coordinates it entered under. Good ideas
died in generation transitions; the wall document exists so this one
doesn't die a second time.

### P8. Spectator absence is silent

If `spectator/identity.md` is missing, the spectator task logs and exits;
the agent runs on, accumulating uncompressed turns until the lossless
guarantee pins the context at the ceiling. The system's memory organ can
be dead while /health says 200.

**Don't rebuild this because:** in an architecture where forgetting-safety
*depends* on the witness, the witness's liveness is a startup invariant,
same class as the identity files. Fail fast or degrade loudly.

### P9. Heartbeat-as-root survived every generation

45-minute heartbeat wake, `:heartbeat:` marker, heartbeat-spawned work —
from openclaw through v3, v4, v3-continued, and into iris-strix's living
practice (45-min cadence, 239+ consecutive beats). It is the oldest
continuously-alive behavior in the lineage.

**Keep because:** it's the autonomy floor. An agent that only wakes when
spoken to is a service. The heartbeat is what makes the engine an
inhabitant rather than an endpoint. (Note the *cron* heartbeat on the
Hermes side was retired for spawning context-free instances — the lesson
transfers: heartbeat must wake a *continuing* context, not a stranger.)

---

## Phase 3 — Boundary inventory

What exists at the system's edges and its disposition across the wall.
Data does not survive (Cass's ruling), so schemas are prior art, not
contracts.

| Surface | Current state | Across the wall? |
|---|---|---|
| **SQLite schemas** (messages, memories, contexts, moves) | 4 migrations | **Prior art only.** Fresh schema; keep turn_number-as-coordinate and birth-in-schema concepts. |
| **Channel JSONL format** (`channels/{adapter}_{id}.jsonl`) | Live, spec'd 5/3 | **Carry the design** (me/not-me roles, snowflake+msg_id dual IDs, cursor entries, write-then-notify). Format may change; semantics survive. |
| **Adapter HTTP protocol** (`/incoming`, `/send`, `/health`, registration, bearer auth) | Live; Discord + TUI speak it | **Carry the contract.** It's the engine's only real external API. v4's feature-negotiation (adapter declares features at registration; system prompt advertises them) is the better version — adopt it. |
| **river-discord** | Live, healthy, 2k lines | **Re-usable as-is candidate.** It's an adapter — it sits *outside* the clean room if the HTTP contract is preserved. Decide: adapters are not part of the rewrite's room. Same for river-tui. |
| **Snowflake ID scheme** (128-bit: micros since birth + packed birth + type + seq) | Live in river-core, 90 tests | **Carry the design.** Birth-encoded identity is load-bearing (birth gate, migrate tool). |
| **Birth ritual** (`river-gateway birth` / `river-migrate init`, "i am <name>" memory) | Live | **Carry.** Identity-in-schema, refuse-to-start-unbirthed. |
| **Workspace contract** (AGENTS/IDENTITY/RULES at root, required; `spectator/` prompt files; `embeddings/moments/`) | Live | **Carry.** Convergent with the actual iris workspace. Purge the fossilized `left/`/`right/` dirs from the seed. |
| **Orchestrator JSON config** (`river.example.json`, env file, $VAR, secrets-as-file-paths) | Live, spec'd 5/3 | **Carry the shape**; drop ModelScanner/GGUF/VRAM management per stream TODO. Reconcile the secrets contradiction: gateway model client reads keys from env (ANTHROPIC_API_KEY/OPENROUTER_API_KEY) while the orchestrator spec mandates file paths. File paths win. |
| **Nix packaging** (`nix/packages`, `nix/modules/river-engine.nix`, athena host import) | Live deployment | **Boundary constraint.** The rewrite must produce the same module-shaped deployable; host config is downstream consumer. |
| **Model client** (Anthropic native + OpenAI-compatible) | Live | **Carry the dual-provider requirement.** |
| **river-memory (TS, rzk, 19 MCP tools, session substrate)** | Live, external, actively developed | **The open question of the rewrite** — see below. |
| **v4 crates** (`archive/`) | Frozen | Stay on the v3 branch with everything else. |
| **`workspace/` seed files** | Live | Carry content (default-workspace-files spec), regenerate fresh. |

### The river-memory boundary (the wall's biggest open question)

The May 3 memory-system-design assigns the engine four layers (loom /
atomic web / chunks / Redis STM), two voices, the digestive cycle, and
activation spreading. Since then, river-memory — TypeScript, outside this
repo — has implemented the loom tools, atomic notes, embeddings search,
hubs/backlinks, STM sections, and the session substrate, and it is in
daily production use by the iris instances. The Rust engine's own memory
stack (memories table, vectors.db, embed tools, redis client) is dead at
runtime.

Options the wall document must pose (not decide):

1. **Engine re-absorbs memory.** Port the four-layer design into the
   rewrite. Cost: re-implementing what river-memory already does well;
   two sources of truth during transition.
2. **Engine federates with river-memory.** The engine owns turn loop,
   channels, context assembly, spectator; river-memory owns all four
   memory layers behind its MCP/CLI surface; the engine's context
   assembler and bystander call it. The engine gets a memory *client*,
   not a memory implementation. The "context-injection slot" from P5
   plugs in here.
3. **Hybrid:** engine owns the *record* (messages, moves — things born in
   the turn loop); river-memory owns the *knowledge* (atomic web, chunks,
   activation). The digestive cycle is the pipe between them.

The bystander's expanded role in memory-system-design (gleaning, STM
management, activation surfing) lands differently under each option. So
does plurality: river-memory is already substrate-neutral (iris-claude,
iris-strix, iris-fable share it); the engine is not yet.

---

## Status

- Phase 0: strata triage — done (`2026-06-10-strata-triage.md`)
- Phase 1: behavior extraction — done (`2026-06-10-behavior-extraction.md`);
  test suite verified green post-audit: **535 passed, 0 failed** across the
  workspace (in nix shell).
- Phase 2: postmortem — this document.
- Phase 3: boundary inventory — this document.
- Phase 4: the wall document (`docs/wall/`) — next. Synthesis of: design
  philosophy + what-i-learned + build-the-hall (constitution layer); live
  behavior contracts (phase 1 §1, §5); postmortem prescriptions (phase 2);
  boundary dispositions (phase 3); open design questions (river-memory
  boundary, witness deployment shape, per-model tool config).

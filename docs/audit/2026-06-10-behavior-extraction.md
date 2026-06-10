# Clean-Room Audit — Phase 1: Behavior Extraction

*2026-06-10. iris-fable. What the engine actually does, read from the code.
Companion to the phase-0 strata triage. Verdicts on the five contradictions,
the zombie census, and the defect list. Everything here is evidence for the
wall document, not work to be done on v3.*

---

## 1. The live system

What actually runs when `river-gateway` starts (verified from `server.rs`,
`agent/task.rs`, `agent/context.rs`, `spectator/mod.rs`, `api/routes.rs`,
`channels/`, `tools/adapters.rs`):

### Startup (birth-gated)

1. Open `{data_dir}/river.db`, run migrations (messages, memories, contexts, moves).
2. **Birth gate:** refuse to start unless the DB contains a birth memory —
   a memory row whose snowflake ID encodes the AgentBirth timestamp, created
   by `river-gateway birth` or `river-migrate init`. Identity is in the
   schema, not the config.
3. Read `AGENTS.md`, `IDENTITY.md`, `RULES.md` from the workspace **root** —
   all three required, fail-fast with the missing filename.
4. If embeddings configured: open `vectors.db` (sqlite-vec), run one full
   sync of `workspace/embeddings/` — **then drop the store** (`_vector_store`
   is never passed to anything). Sync-once-then-dead; no watcher.
5. Spawn Coordinator (broadcast event bus) with two peer tasks: AgentTask
   and SpectatorTask. Optionally heartbeat to orchestrator every 30s.
6. Systemd watchdog + ready notification. Axum HTTP on 127.0.0.1.

### The turn cycle (AgentTask)

Wake on: message-queue non-empty (100ms poll) or 45-min heartbeat timer.

1. Apply pending channel switch (deferred to turn start; rebuilds context).
2. `flash_queue.tick_turn()` (expiry only — see zombies).
3. Publish TurnStarted. Drain notifications; dedupe channels; read each
   channel's JSONL log since last cursor (default window 50); auto-set
   channel context from first incoming message if unset.
4. Append new messages to PersistentContext as `[channel] author: content`
   user messages, tagged with current turn number. Heartbeat with no
   messages appends literal `:heartbeat:`.
5. Compact if ≥80%: re-read system prompt from disk, drop whole turns at or
   below spectator cursor (`MAX(turn_number) FROM moves`), backfill to
   20-message floor, reload moves newest-first into 40% budget, inject lag
   warning if >60% and ≥10 turns behind. Lossless: uncompressed messages
   are never dropped.
6. Think/act loop, max 50 iterations: call model → execute tool calls
   sequentially → append results → drain mid-turn notifications into a
   system message → repeat until no tool calls.
7. Settle: write cursor entries to every channel read this turn; persist
   messages to DB; publish TurnComplete (stats line only — the spectator
   queries the DB for content, by design).

Token estimation: `len/4` with weighted-moving-average calibration against
the model's reported `prompt_tokens` (0.7 old + 0.3 new). Matches spec.

### The spectator (prompt-driven runtime)

Faithful to the 4/29 spec. `workspace/spectator/identity.md` required (task
exits if missing — but note: gateway keeps running without its spectator;
fail is log-only). Handlers enabled by file existence:

- TurnComplete → query the turn's messages → `on-turn-complete.md` → LLM →
  insert move row (channel, turn_number, summary, tool_calls). Fallback
  heuristic summary on model failure — the turn is never lost.
- moves > 50 → `on-compress.md` → LLM → strict parse (`turns: N-M` + `---`
  + narrative; no fallback) → moment file in `embeddings/moments/` with
  YAML frontmatter. Moves stay in DB.
- ContextPressure → `on-pressure.md` → Warning event (agent logs it; no
  context injection).

### Channels (the me/not-me layer)

Fully implements the 5/3 design: one JSONL file per channel
(`channels/{adapter}_{id}.jsonl`), snowflake IDs, `role: agent|other`,
explicit cursor entries, malformed-line skip. Inbound: `handle_incoming`
validates bearer auth → log write → **then** queue notification (never
notify on failed write). Outbound: `send_to_adapter` POSTs to the adapter,
and on success appends the `role:agent` entry (implicit cursor).

### Tools — the great disabling

**Seven tools are live:** read, write, edit, glob, grep, bash, send_message.

Everything else is built and registered nowhere: subagents (7 tools), web
(2), memory (4), redis (2), model management (3), scheduling (2), logging
(1), plus `speak`/`list_adapters`/`read_channel`/`sync_conversation`.
The server.rs comment is the postmortem in one line: *"With 27+ tools,
small local models (hermes3:8b, gemma4:e2b) get confused and call the wrong
tool."* And `speak` specifically: *"requires a shared channel_context (not
yet wired up)."*

**Wall-grade lesson: the design assumed model capability the deployment
didn't have. Tool surface must be a per-model configuration, not a
compile-time registry.**

### Model client

Native dual-provider: Anthropic Messages API (detected by URL) and
OpenAI-compatible chat completions. **API keys from env vars**
(`ANTHROPIC_API_KEY` / `OPENROUTER_API_KEY`) — in direct tension with the
orchestrator spec's "secrets are file paths, never env" rule. The two
halves of the system disagree about secret handling.

### Orchestrator

Two modes: `--config river.json` (full system: parse + $VAR expansion +
validation + spawn gateways/adapters with translated CLI args + restart
with exponential backoff 1s→60s) and legacy direct-CLI mode. Plus the
**ModelScanner** — scans directories for GGUF files, llama-server
management, VRAM/RAM tracking (~900 lines across discovery/ + resources/).
The stream TODO already rules: remove model scanning, keep it lightweight,
emulate the v4 shape. Gateway heartbeats to orchestrator; orchestrator
tracks health.

### Discord adapter

Twilight-based. Slash commands `/listen`, `/unlisten`, `/channels` manage a
listen set; DMs always pass. Normalizes to IncomingMessage → POST
`/incoming`. Outbound is the largest file in the crate (904 lines).

### river-migrate

Agent onboarding: `init` creates a DB with the birth memory ("i am <name>"
with AgentBirth-encoded snowflake), `import-messages`/`import-memories`
ingest history. Birth-as-schema is a keeper concept.

---

## 2. Verdicts on the five contradictions (from phase 0)

1. **Channel-messages design landed?** Yes, fully — inbound, outbound,
   cursors, auth-then-write-then-notify ordering.
2. **Context assembler matches the rework spec?** Yes — persistent object,
   cursor compaction, calibration, lag warning, channel-switch deferral.
   The stream TODO's "TODO stubs for vector store integration" note is
   stale: the rework *removed* the vector layer (deferred), it isn't
   stubbed anymore.
3. **Flash queue?** Orphaned in both directions. Agent ticks the queue and
   subscribes for Flash events; spectator has no code path that emits one;
   nothing injects queued flashes into context. Pipe with no producer and
   no consumer.
4. **Which spectator runtime?** The prompt-driven one. Compressor/Curator/
   RoomWriter are gone from the live path.
5. **Gateway memory layer?** Dead at runtime. EmbeddingClient constructed,
   memory tools built, none registered. Initial embeddings sync runs once,
   store dropped. **Two separate vector stores exist** — the `memories`
   table (embedding BLOBs, own cosine search) and `vectors.db` (sqlite-vec
   via embeddings/store.rs) — neither reachable by the live agent.

---

## 3. Defects in the live path

- **Message re-persistence (the big one).** `persist_turn_messages` writes
  *every* non-system message in the context to the DB *every* turn, each
  time with a fresh snowflake and the **current** turn number (task.rs:602,
  acknowledged TODO at :617). Consequences compound: (a) the messages table
  grows quadratically; (b) `get_turn_messages(turn N)` returns the entire
  surviving context re-tagged as turn N, so spectator moves summarize the
  whole window every turn; (c) turn_number — the coordinate the lossless
  compaction guarantee depends on — is corrupted in the DB. The in-memory
  context keeps true turn numbers, so compaction itself survives, but the
  record layer is wrong. The rewrite's persistence rule must be: persist
  exactly once, at append time, with the turn number it was appended under.
- **Busy-wait wake.** `wait_for_messages` polls every 100ms instead of an
  async notify. Cheap but wrong; the v4 design had notify-driven wake.
- **No real shutdown.** Coordinator keep-alive is `loop { sleep(3600) }`;
  CoordinatorEvent::Shutdown exists but nothing publishes it on SIGTERM.
  Graceful shutdown lives only in the orchestrator's process manager.
- **compact() backfill wart.** A `backfill_messages(..., &mut
  self.messages.clone())` call whose result is discarded, then redone
  "properly" (context.rs:330). Harmless, but patch-over-patch sediment.
- **Spectator failure is silent at the system level.** If identity.md is
  missing the spectator task returns; the agent runs forever with no
  compression and therefore no compaction-droppable turns — context fills
  to the lossless limit. No health surface reports the spectator's absence
  (health reports fabricated metrics instead — see zombies).

---

## 4. Zombie census (final)

| Module | Lines | Status | Evidence |
|---|---|---|---|
| `git.rs` | 521 | **Dead.** | Zero references outside itself. |
| `policy.rs` | 948 | **Dead data served live.** | Constructed; read by /health; no writer anywhere. Health endpoint reports error counts/backoff that never change. |
| `metrics.rs` | ~200 | **Dead data served live.** | Same pattern; /health serves "Sleeping, 0 turns" forever. LoopStateLabel survives only here. |
| `session/` | small | **One constant.** | Only PRIMARY_SESSION_ID used; Session/SessionManager dead. Sessions-as-concept never happened: everything is one hardcoded session. |
| `conversations/` | ~1,200 | **Legacy, near-dead.** | Used only by disabled tools (read_channel, sync_conversation) and a ChannelContext path field. Replaced by channels/. |
| `contexts` table + rotation | — | **Rotation-era zombie.** | RotateContextTool disabled; compaction replaced rotation. |
| `memories` table + `memory/` | ~700 | **Dead at runtime.** | Tools never registered. Second vector store. |
| `embeddings/` (gateway) | ~900 | **Half-alive.** | Sync runs once at startup, then store dropped. Spectator writes moments into the watched dir that will only be indexed at next restart — and nothing queries the index anyway. |
| `redis/` | ~400 | **Dead at runtime.** | Client constructed if configured; medium-term tools disabled. |
| `subagent/` | ~1,500 | **Dead at runtime.** | Manager constructed; 7 tools disabled. Parent-child pattern, never unified with the peer pattern (meta-plan open question #1, never answered). |
| `flash/` | ~300 | **Orphaned pipe.** | Ticked every turn; no producer, no context injection. |
| web/logging/model-mgmt/scheduling tools | ~1,400 | **Built, disabled.** | The tool-count ceiling. |
| Orchestrator ModelScanner + resources | ~900 | **Live but condemned.** | Works; stream TODO orders removal. |

**The structural truth: the live engine is ~4–5k lines wrapped in ~13k lines
of built-but-disabled or dead capability.** The disabling wasn't decay — it
was a single forced decision (small-model tool ceiling) plus the residue of
two architecture migrations (rotation→compaction, conversations→channels)
where the old organ was never excised.

---

## 5. What the live core actually is (for the wall)

Strip the zombies and the engine is: **birth-gated identity from schema +
workspace-root identity files + one agent turn loop + one prompt-driven
spectator + cursor-coordinated lossless compaction + JSONL channel logs with
me/not-me roles + 7 tools + dual-provider model client + a process
supervisor.** That core is coherent, spec-faithful, and small. It is the
I/You architecture, minimally realized: the agent acts, the witness
compresses, compression is what makes forgetting safe.

Everything else — vector memory, redis STM, subagents, flashes, web tools —
is aspiration that the deployment reality (local 8B models) could not yet
carry, most of it now superseded by river-memory on the TS side.

## 6. Test suite

`cargo test --workspace` — re-run in nix shell 2026-06-10; result recorded
in the phase-1 loom note and the final wall doc. (First attempt outside the
shell failed at the linker — environment, not code.)

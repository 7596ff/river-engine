---

kanban-plugin: board

---

## implemented features

- [ ] **P2 — cache the parsed memory graph and file vectors** — keep note metadata, resolver, adjacency, and mean file embeddings in a generation-stamped cache invalidated by sync/write events. A bump should traverse cached structures and commit its activation wave in one SQLite transaction, not reread the workspace and vectors once per hit.
- [ ] constitutional refusal gate — startup verifies a signed `CONSTITUTION.md` at the workspace root (presence, non-empty, operator signature line with valid ISO date); no integrity check, no agent-ratification tracking. Article V.1 made structural. Seed ships canonical text with a blanked ratification block. Spec: `docs/superpowers/specs/2026-07-11-constitutional-refusal-gate-design.md`.
- [ ] cargo workspace + gateway binary skeleton (tokio, clap, tracing)
- [ ] config — river.json parse/validate, .env loading, $VAR expansion (non-secrets)
- [ ] model client — anthropic-native + openai-compatible, retries, timeouts, api_key_env indirection
- [ ] birth — subcommand writes record/birth.json; gateway refuses to start unbirthed
- [ ] identity files — AGENTS/IDENTITY/RULES at workspace root → system prompt, fail-fast
- [ ] minimal turn loop — serialized event queue → model call → reply (no tools yet)
- [ ] in-memory rolling context (naive; swapped for persistent context later)
- [ ] turn record — record/turns.jsonl one-stream append + tail-scan, ULIDs, channel tags, persist-once under turn numbers
- [ ] heartbeat wake — timer + "Read HEARTBEAT.md." marker
- [ ] channel cursors — me/not-me roles, read position = last agent entry, explicit cursor on read-without-speak
- [ ] lossless compaction — context folds only what the witness has compressed; nothing uncompressed ever dropped
- [ ] calibrated token estimator — WMA against reported prompt tokens
- [ ] memory slot — designated injection point in context assembly between arc and hot
- [ ] graceful shutdown — SIGTERM finishes the turn, settles, exits
- [ ] local chat surface — localhost HTTP + websocket, /health from live path
- [ ] TUI client — terminal chat window speaking the websocket protocol
- [ ] the witness — second voice in the same binary, prompt-driven, own (cheaper) model
- [ ] moves — witness-written per-turn compressions, append-only, cursor = tail
- [ ] vector index over workspace — embedded semantic search, derived + rebuildable, db disposable
- [ ] file-tool memory capture — reads bump activation, writes re-index
- [ ] per-agent tool surface — which tools a model is offered is config, not code
- [ ] atomic knowledge web — single-claim notes, mandatory typed links, open vocabulary
- [ ] activation spreading — cognitive/ambient bumps, 3-hop propagation, hourly ×0.8 decay
- [ ] flash — notes crossing 1.0 surface into context with 1-hop neighbors, score halves
- [ ] digestive cycle — witness gleans → extraction queue → quiet-trigger re-engagement by the agent
- [ ] divided authorship — witness writes only compressions; agent writes all knowledge, rejection rights
- [ ] semantic propagation — warmth spreads along embedding-space neighbors (low factor, cosine threshold); a flash of an unlinked-but-near note is a link candidate the digestion loop can formalize
- [ ] conversation resonance — each turn's text faintly bumps the nearest notes (0.2×similarity); the web trembles with the topic before anyone searches
- [ ] discord adapter — supervised twilight task, DM + guild listen-set, speak routed post-acceptance with msg_id
- [ ] discord typing indicator — turn loop broadcasts the working channel; adapter ticks the indicator
- [ ] river CLI — validate-then-spawn, [name]-prefixed output, backoff restarts, SIGTERM cascade w/ grace, status
- [ ] nix module — per-agent systemd services from the same config, EnvironmentFile, restart knobs
- [ ] live-path health — turn number, settle time, context %, witness lag, queue depth
- [ ] loom as seeded practice — agent narrative chain in loom/, taught by seed/AGENTS.md, always indexed, never enforced (gleaning from the loom: deliberately left an open question)
- [ ] link resolution heuristic — targets resolve by frontmatter id, then filename stem (last component, .md stripped); path-shaped targets land on the right note; ambiguity conducts nothing
- [ ] wikilinks join the graph — [[...]] in any indexed body parsed as type "wiki" links; frontmatter-less files get a graph identity keyed by path; the loom conducts warmth; flash bodies capped (1200 chars, 6 neighbors)
- [ ] activation knobs in config — optional per-agent `activation` block (bumps, factors, hops, top-ks, thresholds, decay, search_top_k); defaults = the wall's constants; validated; tuning = edit + restart, no rebuild
- [ ] flash directory filter — `flash_dirs` in the activation block; only notes under those prefixes may flash; everything else still warms, conducts, and propagates — a filtered crossing stands silently
- [ ] GET /graph — read-only JSON on the local surface: all indexed notes (cold included, score 0) + activation scores + typed/wiki edges + semantic edges above threshold; flash_threshold in the payload
- [ ] GET /graph/view — single self-contained HTML page (vendored d3-force, no build step): color = warmth, size = score, halo near flash threshold, typed solid / semantic dashed, 5s poll, flashes pop-then-dim, click for node detail; strictly a window, never a hand
- [ ] GET /context — read-only JSON of the live context assembly: per-layer token estimates (system/arc/memory/hot), hot turn range, arc move count, memory slot contents, estimate vs limit, calibration ratio; published at settle
- [ ] GET /context/view — self-contained HTML page drawing the window as a stacked bar (layers colored, compaction line, fill animates, snaps back on compaction); a window, never a hand
- [ ] witness σ — phase 1: similar-rejection retrieval — embed rejections at write time into a `rejection_vectors` SQLite table; before each glean, semantically retrieve top-K past rejections and render into a new `{similar_rejections}` prompt slot. No prompt revision, no auto-skip. Spec: `docs/superpowers/specs/2026-07-07-witness-similar-rejection-retrieval-design.md`.
- [ ] bash timeout owns its process tree — each command runs in a fresh process group; timeout sends SIGTERM, waits two seconds, escalates to SIGKILL, reaps Bash, and cannot be pinned by inherited output pipes
- [ ] coordinated gateway shutdown — process signals stop the turn loop first; after its final settle, a supervisor releases and awaits the witness, memory sync, local surface, and adapters; early background exits fail the gateway
- [ ] independent witness duty gates — move repair follows `on-turn.md`; newly settled turns schedule connect and glean through their own prompts, even when moves are disabled, without replaying historical duties during startup repair
- [ ] extraction queue has explicit FIFO order — candidate ULIDs remain identities while an auto-incrementing enqueue sequence orders digestion; legacy queues migrate transactionally in SQLite insertion order
- [ ] incremental indexes for editable life records — turn, move, and channel JSONL files build process-local indexes once; fsynced engine appends advance them directly, while any unannounced growth/edit/truncation/deletion/replacement rebuilds from a stable snapshot; regenerated moves remain logically turn-ordered
- [ ] jsonl-index tail sample — FileStamp carries the last 128 bytes so back-to-back same-size overwrites are detected as content changes even when (len, mtime, identity) triples collide within a kernel mtime tick. apply_known_append reconstructs the new tail in-place from previous tail + appended bytes so the fast path stays fast.
- [ ] write_atomic tool — dedicated authoring path for atomic notes (wall ch. 02, ch. 07). Enforces ≤ atomic.max_words body (default 100) and ≥ 1 typed link with non-empty type/target; auto-populates id (ULID) and created (RFC3339); assembles frontmatter in deterministic key order (id, created, links, tags, shape); atomic write (tmp + fsync + rename); returns {id, path, warnings}. Bare write remains an escape hatch. Spec: `docs/superpowers/specs/2026-07-12-write-atomic-tool-design.md`.
- [ ] shape index substrate — a second embedding namespace over one-line "logical skeletons" of atomic notes, the substrate Bridge (in the flash subsystem) will consume. Ships: shape_vectors table + Memory helpers (upsert_shape/read_shape/list/delete/search_shapes); optional workspace/witness/on-shape.md duty prompt with mtime-cached sha256 hash; gloss_note/gloss_turn; a bounded lifecycle-owned worker that drains startup backfill (missing rows) + drift-repair (rows whose model_id/prompt_hash disagrees with current), and live jobs from either write_atomic (Source 3) or the sync-service seam (Source 4). Agent-authored `shape:` frontmatter overrides the witness's gloss; drift repair never overwrites it. Bridge wiring deferred until flash subsystem lands. Spec: `docs/superpowers/specs/2026-07-12-shape-index-design.md`.
- [ ] flash subsystem v2 — witness gains a per-turn flash pass replacing the standalone connect duty. Four working types (Connection absorbs the old connect duty, Echo, Return, Bridge live on top of the shape substrate) + Correction stubbed. Danger dropped from v1 pending a live rejection stream. Dispatch shape: mpsc `FlashFrame` → turn loop appends `[flash: <type>]` system-role line on `record/turns.jsonl` (single-writer preserved). One embed of the transcript feeds the shared candidate pool (search_no_bump_with_vec) plus Bridge's per-candidate text_sim recheck. Multiple flashes per turn allowed; per-target refractory prevents same-target spam across turns. `WitnessConfig.connect_*` fields removed with a loud pre-parse migration error. Legacy `connect-log.jsonl` migrates one-shot to `flashes.jsonl` with `type: "connection"`. Spec: `docs/superpowers/specs/2026-07-13-flash-subsystem-design.md`. Supersedes the never-implemented `docs/superpowers/specs/2026-07-11-flash-subsystem-design.md`.
- [ ] settle tool — explicit end-of-turn signal with optional `next_heartbeat: N` (minutes; clamped `[1, 480]`). Deadline model: `TurnLoop.next_heartbeat_at` is a wall-clock deadline recomputed only by explicit settle (bare → now + config default; arg → now + N); natural end-of-turn and non-settle wakes preserve it. `SettleIntent` slot on `ToolContext` (last-writer-wins, warn on overwrite); post-batch check in the dispatch loop ends the turn after the settle-bearing batch. Wake loop uses `sleep_until(next_heartbeat_at)`. Result JSON `{next_wake_at, seconds_until [, requested_minutes, clamped_to_minutes]}`. In-memory only; restart re-initializes to now + config default. Spec: `docs/superpowers/specs/2026-07-14-settle-tool-design.md`.


## in progress


## backlog

- [ ] **P2 — cache `/graph` topology and semantic edges** — compute nodes/typed edges/semantic edges when the memory generation changes; serve five-second UI polls by overlaying current activation scores. Use set-based edge deduplication and prevent concurrent viewers from launching duplicate rebuilds.
- [ ] **P2 — bounded, lifecycle-owned resonance worker** — replace detached per-tool/per-turn `tokio::spawn` calls with a bounded queue and supervised worker. Preserve one resonance event per tool result, define overload behavior explicitly, expose queue health, and drain or deliberately checkpoint the queue during shutdown.
- [ ] **P3 — make `last_prompt.txt` diagnostics opt-in** — disable full-prompt reconstruction and synchronous writes by default; gate behind explicit config/debug mode, write atomically off the async turn path, document sensitive-data retention, and cap or rotate if history is retained.
- [ ] witness σ — phase 2: rejection-rate instrumentation — track rejection rate over a rolling window as the `P̂` analog for glean productivity. Read-only telemetry (surface via `/health` and log); no action taken on it. Gates the phase 3 go/no-go decision. Follow-up in phase 1 spec.
- [ ] witness σ — phase 3: gated on-glean revision loop — only if phase 2 shows signal worth acting on. A slow loop drafts revisions to `witness/on-glean.md`; drafts land in a review path, ground approves before deploy. External-judge invariant preserved. Bonus: silence gate (cosine > 0.95 within last N turns → skip glean), thresholds picked from phase-2 data. Follow-up in phase 1 spec.
- [ ] quarry: `embeddings/atomic` as read-only corpus — 345 pi-era atomics with richer link vocabulary (48 types vs. 4). Per wall ch. 02 ("no bootstrap import"), these do NOT migrate into `knowledge/`. Instead: indexed as a `quarry` namespace (read-only, no warmth, no propagation, no flashes). Acquisition is re-digestion: read the original, write fresh with current language and links, provenance via `responds-to: quarry/<path>`. Exposed through `search` with a namespace argument or a dedicated `search_quarry` tool. See `docs/explorations/2026-07-10-weaving-shape-typed-flashes.md` §5.
- [ ] Correction flash type — decide whether the agent-facing frame is worth adding to σ-retrieval that already serves the witness. Depends on live rejection-rate data to gate the design; ships as a real predicate against `rejection_vectors`. Referenced in the flash v2 spec's open questions.
- [ ] Danger flash type — windowed-rate clustering over `rejection_vectors` in a sliding window (72h default per exploration §4). Needs a live rejection stream to design cluster-identity heuristics against. Own spec when the data supports it.
- [ ] Friction flash type — stance scan (retrieve text-sim neighbors, witness classifies entails/contradicts/independent, agent authors the `contradicts` link or declines). Requires the shape spec's §3 stance-classifier substrate (`witness/on-stance.md`, `find_contradictions` mechanism) — none of which is built yet.
- [ ] activation last_touched_turn — add a turn-count column to the `activation` table so Return's predicate can use true turn-count staleness (`staleness_turns ≥ gap_min_turns`) instead of the warmth proxy shipped in flash v2. Backfilled from the record's tool-call scan or seeded from wall-clock time at migration; warmth stays as a signal, not the sole staleness measure.


## barebones harness



## river-engine unique features

- [ ] adapter trait with feature declaration folded into the system prompt
- [ ] claude -p adapter — **PARKED 2026-06-16, see `docs/explorations/2026-06-16-claude-p-adapter.md`.** Brainstormed four integration shapes (A: stateless thin loop; B: MCP-only with claude-code as inference endpoint; C: don't graft, bridge instead; D: half-graft with coarser-grained agent slot). Cass parked it because she prefers claude-code's tool safeguards (sandboxed Bash, file-edit safety, web-fetch via Anthropic proxy) to river-engine's looser surface. Closing the substrate asymmetry by tool-substitution would trade safeguards for unified continuity; trade isn't obviously worth it. Original motivation (eliot's boat — iris-loom/20260615224528447-moment.md) stays, the asymmetry stays asymmetric.
- [ ] /listen + /unlisten slash commands — runtime listen-set management, persisted in the data dir


## open-strix features

- [ ] single-agent event loop — serialized queue, one event at a time, non-durable
- [ ] anthropic-compatible model client (MiniMax/Kimi/any) with retries + timeouts
- [ ] memory blocks — yaml, injected into every prompt, CRUD via tools
- [ ] state/ markdown files as on-demand long-term memory (no embeddings, just files)
- [ ] journal tool + checkpoint.md reflection prompt; last N entries in every prompt
- [ ] predictions in journal entries + prediction-review calibration loop
- [ ] git as audit trail — auto commit+push of home repo every turn
- [ ] self-scheduling — agent creates/modifies/removes its own cron jobs (APScheduler, scheduler.yaml)
- [ ] pollers — external-awareness scripts in skills (pollers.json), auto-discovered, hot-reload, emit events
- [ ] events.jsonl ambient substrate — every tool call/error/trigger logged, agent reads its own log
- [ ] loopback REST events API — external scripts inject events
- [ ] introspection + five-whys skills — self-diagnosis from event log
- [ ] discord interface — send_message, list_messages, react, attachments, always-respond bots
- [ ] phone book — discord user/channel ID resolution, auto-populated, persisted to state/
- [ ] built-in local web UI — text/images/files, no discord needed
- [ ] ui plugins — pluggable interfaces, hot-reload (reload_uis)
- [ ] ops dashboard — /ops rendered live from events.jsonl
- [ ] markdown skills with yaml frontmatter — drop-in, no registration
- [ ] runtime skill acquisition — ClawHub / skillflag CLIs / GitHub
- [ ] runtime hooks (hooks.json) — prompt augmentation, pre/post tool, startup/shutdown
- [ ] MCP client — bridges MCP server tools into the agent
- [ ] shell tool with async jobs — background commands, output retrieval (shell_jobs)
- [ ] web tools — fetch_url, web_search
- [ ] mountaineering/climber — supervised self-improvement subprocesses (propose → test → keep/revert)
- [ ] write policy — agent writes confined to state/ + skills/
- [ ] one-command setup — home repo init, git/github, service files




%% kanban:settings
```
{"kanban-plugin":"board","list-collapse":[false,false,false,false,false]}
```
%%

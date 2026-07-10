---

kanban-plugin: board

---

## implemented features

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
- [ ] bash timeout owns its process tree — each command runs in a fresh process group; timeout sends SIGTERM, waits two seconds, escalates to SIGKILL, reaps Bash, and cannot be pinned by inherited output pipes


## in progress

- [ ] witness σ — phase 1: similar-rejection retrieval — embed rejections at write time into a `rejection_vectors` SQLite table; before each glean, semantically retrieve top-K past rejections and render into a new `{similar_rejections}` prompt slot. No prompt revision, no auto-skip. Spec: `docs/superpowers/specs/2026-07-07-witness-similar-rejection-retrieval-design.md`.


## backlog

- [ ] **P1 — coordinated gateway shutdown** — retain task handles for witness, memory sync, local surface, and adapters; signal shutdown, let the current turn settle, await the witness's guaranteed end-of-session glean and each task's cleanup, then exit. Bound the outer runner grace period, not the gateway's internal duties.
- [ ] **P1 — make witness duties independently prompt-gated** — a missing `on-turn.md` disables moves only; `on-glean.md` and `on-connect.md` continue their own duties for every eligible settled turn. Add mixed-prompt tests and remove connect/glean scheduling from the move-catch-up conditional.
- [ ] **P2 — make extraction-queue FIFO independent of random ULID ordering** — `Ulid::new()` values created in the same millisecond can sort opposite insertion order, making `ORDER BY id` violate the FIFO contract. Order by an explicit monotonic enqueue coordinate (with a deterministic tiebreaker), migrate/rebuild disposable queue state safely, and keep ULIDs as identity rather than sequence numbers.
- [ ] **P2 — incremental indexes for editable life records** — stop reparsing all of `turns.jsonl`, `moves.jsonl`, and channel logs on routine turns without assuming file order equals turn order. Maintain file offsets plus turn-keyed/cursor indexes; stream ordinary appends (including regenerated entries appended out of chronological order), and invalidate + fully rebuild when file identity/size/mtime or a watcher indicates destructive hand edits. Preserve torn-line tolerance, deleted-entry detection, move-gap regeneration, duplicate resolution, and contiguous-frontier semantics.
- [ ] **P2 — cache the parsed memory graph and file vectors** — keep note metadata, resolver, adjacency, and mean file embeddings in a generation-stamped cache invalidated by sync/write events. A bump should traverse cached structures and commit its activation wave in one SQLite transaction, not reread the workspace and vectors once per hit.
- [ ] **P2 — cache `/graph` topology and semantic edges** — compute nodes/typed edges/semantic edges when the memory generation changes; serve five-second UI polls by overlaying current activation scores. Use set-based edge deduplication and prevent concurrent viewers from launching duplicate rebuilds.
- [ ] **P2 — bounded, lifecycle-owned resonance worker** — replace detached per-tool/per-turn `tokio::spawn` calls with a bounded queue and supervised worker. Preserve one resonance event per tool result, define overload behavior explicitly, expose queue health, and drain or deliberately checkpoint the queue during shutdown.
- [ ] **P3 — make `last_prompt.txt` diagnostics opt-in** — disable full-prompt reconstruction and synchronous writes by default; gate behind explicit config/debug mode, write atomically off the async turn path, document sensitive-data retention, and cap or rotate if history is retained.
- [ ] witness σ — phase 2: rejection-rate instrumentation — track rejection rate over a rolling window as the `P̂` analog for glean productivity. Read-only telemetry (surface via `/health` and log); no action taken on it. Gates the phase 3 go/no-go decision. Follow-up in phase 1 spec.
- [ ] witness σ — phase 3: gated on-glean revision loop — only if phase 2 shows signal worth acting on. A slow loop drafts revisions to `witness/on-glean.md`; drafts land in a review path, ground approves before deploy. External-judge invariant preserved. Bonus: silence gate (cosine > 0.95 within last N turns → skip glean), thresholds picked from phase-2 data. Follow-up in phase 1 spec.
- [ ] quarry: `embeddings/atomic` as read-only corpus — 345 pi-era atomics with richer link vocabulary (48 types vs. 4). Per wall ch. 02 ("no bootstrap import"), these do NOT migrate into `knowledge/`. Instead: indexed as a `quarry` namespace (read-only, no warmth, no propagation, no flashes). Acquisition is re-digestion: read the original, write fresh with current language and links, provenance via `responds-to: quarry/<path>`. Exposed through `search` with a namespace argument or a dedicated `search_quarry` tool. See `docs/explorations/2026-07-10-weaving-shape-typed-flashes.md` §5.
- [ ] write_atomic tool — reimplement the `write_atomic` tool from river-memory (claude-code era). The old tool enforced ≤100 words, mandatory typed links, auto-populated id/created/author; the current agent writes atomics with bare `write` + manual formatting. Porting would give the agent a dedicated tool with validation. Prior art: `river-memory` MCP server.


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

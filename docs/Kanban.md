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


## in progress



## barebones harness



## river-engine unique features

- [ ] turn record — append-only full-history one-stream jsonl, channel-tagged, turn-coordinated, persist-once
- [ ] birth ritual — record/birth.json, gateway refuses to start unbirthed
- [ ] adapter trait with feature declaration folded into the system prompt
- [ ] loom as seeded practice — agent narrative chain in loom/, indexed, gleanable, never enforced
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
{"kanban-plugin":"board","list-collapse":[false,true,true,false,false]}
```
%%
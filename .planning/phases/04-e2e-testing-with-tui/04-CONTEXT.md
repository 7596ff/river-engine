# Phase 4: E2E Testing with TUI - Context

**Gathered:** 2026-04-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Dyad boots, workers communicate via worktrees, TUI adapter validates actor/spectator loop. This phase creates integration tests that prove the complete system works end-to-end using the TUI mock adapter.

</domain>

<decisions>
## Implementation Decisions

### Test scenarios
- **D-01:** Core flow only — dyad boots, user sends message, actor responds, spectator observes, baton switches, sync happens
- **D-02:** Explicitly verify baton swap — test must confirm actor becomes spectator and vice versa, watching for role changes in backchannel or context
- **D-03:** No edge cases for v1 — network errors, restart recovery, conflicts deferred to later phases

### Test infrastructure
- **D-04:** Integration tests in Rust, using tests/ directory in river-orchestrator crate
- **D-05:** Tests spawn orchestrator, workers, and TUI adapter programmatically
- **D-06:** Messages injected via HTTP to TUI's /notify endpoint (not keyboard input)
- **D-07:** Context files polled to observe worker responses and state changes

### TUI enhancements
- **D-08:** ReadHistory gap — leave as-is, agents read conversation files directly
- **D-09:** Add baton state display to TUI header showing which worker is actor/spectator
- **D-10:** Make backchannel bidirectional — TUI watches backchannel file and posts to workers' /notify endpoints

### Success criteria
- **D-11:** Message flows both directions — user sends, actor responds, spectator observes (visible in context files)
- **D-12:** Baton switches correctly — actor becomes spectator, verified via backchannel or context
- **D-13:** Git sync commits appear — workers commit to their branches, visible in git log
- **D-14:** All processes healthy — orchestrator, two workers, TUI adapter return 200 on health endpoints

### LLM mock behavior
- **D-15:** Mock HTTP endpoint instead of real LLM — deterministic, fast, no API costs
- **D-16:** Mock returns role-aware responses (actor returns action-like text, spectator returns observation-like)
- **D-17:** Mock returns tool calls (switch_roles, speak, etc.) to exercise full think→act loop

### Test isolation
- **D-18:** Temp directory per test — each test creates fresh workspace with new git repo
- **D-19:** Tests can run in parallel with clean state guaranteed
- **D-20:** Dynamic port 0 for all processes — OS assigns available ports, tests discover endpoints via registration

### Claude's Discretion
- Mock LLM response content and tool call sequences
- Exact test assertions and timeout values
- How to wait for async operations to complete (polling interval, max wait)
- Error message formatting in test failures

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Prior phase context
- `.planning/phases/02-workspace-infrastructure/02-CONTEXT.md` — D-10 through D-13 define git worktree branch strategy (left/right branches merge into main)
- `.planning/phases/03-sync-protocol-documentation/03-CONTEXT.md` — D-08 defines sync at turn start, D-11 defines PR-style flow

### TUI crate (code review completed)
- `crates/river-tui/src/main.rs` — CLI startup, context file tailing, backchannel handling
- `crates/river-tui/src/adapter.rs` — Adapter state management, message types
- `crates/river-tui/src/http.rs` — HTTP server with /execute, /health endpoints
- `crates/river-tui/src/tui.rs` — Terminal UI rendering, keyboard handling

### Worker tools
- `crates/river-worker/src/tools.rs` — Tool implementations including read_history
- `crates/river-worker/src/llm.rs` — LLM client and tool definitions

### Protocol definitions
- `crates/river-protocol/src/registration.rs` — Worker/adapter registration
- `crates/river-adapter/src/lib.rs` — OutboundRequest, InboundEvent types

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `river-tui` crate — 49 tests, production-grade mock adapter with full feature support
- `AdapterState` — shared state accessible for test assertions
- Context file tailing — existing async tasks for observing worker state
- Snowflake ID generation — deterministic IDs for test messages

### Established Patterns
- Process spawning via `tokio::process::Command` in orchestrator
- Registration via HTTP POST to `/register` endpoint
- Shared state via `Arc<RwLock<T>>`
- Health checks via `/health` endpoint returning 200

### Integration Points
- Orchestrator spawns workers and adapters — test harness needs same flow
- Workers register with orchestrator to get worktree paths
- TUI adapter's `/notify` endpoint accepts `InboundEvent` for message injection
- Context files at `workspace/{side}/context.jsonl` for observing worker state

### TUI Code Review Findings (2026-04-06)
- **Architecture:** Clean separation of concerns (adapter.rs: state, http.rs: handlers, tui.rs: rendering)
- **Dyadic alignment:** Shows both sides with L/R prefix, supports backchannel, interleaves by timestamp
- **Gaps identified:** No baton visualization (D-09 addresses), backchannel write-only (D-10 addresses)
- **Test coverage:** 49 existing tests across all modules

</code_context>

<specifics>
## Specific Ideas

- Mock LLM should exercise the think→act loop by returning tool calls, not just text responses
- Baton visualization in TUI header helps debugging during manual testing and provides assertion target for automated tests
- Bidirectional backchannel enables testing inter-worker communication without polling files

</specifics>

<deferred>
## Deferred Ideas

- Edge case testing (network errors, restart recovery, conflicts) — future phase
- CI integration and flakiness handling — defer until tests are stable locally
- Performance testing with multiple message exchanges — later concern
- Discord adapter testing — v2 scope per PROJECT.md

</deferred>

---

*Phase: 04-e2e-testing-with-tui*
*Context gathered: 2026-04-06*

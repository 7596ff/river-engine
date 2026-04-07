# Phase 5: E2E Test Feature Implementation - Context

**Gathered:** 2026-04-07
**Status:** Ready for planning

<domain>
## Phase Boundary

Extends Phase 4's E2E test infrastructure with additional test scenarios: complete message flow (user→actor→spectator→response) and multi-turn conversation verification. This phase adds new tests to the existing e2e_dyad_boot.rs file using the Phase 4 test harness (helpers, mock_llm).

</domain>

<decisions>
## Implementation Decisions

### Test coverage scope
- **D-01:** Add complete message flow test — user sends message, actor thinks+acts, spectator observes, baton swap, response visible in context
- **D-02:** Add multi-turn conversation test — multiple message exchanges to verify state accumulation and baton cycling over time
- **D-03:** Defer error recovery scenarios (LLM timeout, adapter disconnect, process crash/respawn) to future phase
- **D-04:** Defer git sync protocol testing (commit after writes, sync at turn boundaries) to future phase

### Mock LLM behavior
- **D-05:** Role-aware text only — actor returns action-oriented text, spectator returns observation-oriented text
- **D-06:** No tool-calling mock for Phase 5 — text responses sufficient to prove message routing
- **D-07:** Mock LLM keeps responses deterministic and fast (no conversation state tracking)

### Test execution model
- **D-08:** Extend existing e2e_dyad_boot.rs file — add new test functions to crates/river-orchestrator/tests/e2e_dyad_boot.rs
- **D-09:** Reuse Phase 4 test harness (helpers.rs, mock_llm.rs) for process spawning and assertions
- **D-10:** Follow Phase 4 test pattern: SETUP → WAIT → INJECT → VERIFY → CLEANUP

### Claude's Discretion
- Exact mock LLM response text for actor vs spectator
- Test timeout values and polling intervals
- Assertion message formatting
- Number of turns in multi-turn test (2-5 turns reasonable)

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Phase 4 context (prior decisions to maintain)
- `.planning/phases/04-e2e-testing-with-tui/04-CONTEXT.md` — D-04 through D-20 define test infrastructure patterns, isolation, port assignment
- `.planning/phases/04-e2e-testing-with-tui/04-03-SUMMARY.md` — E2E test pattern established: SETUP → WAIT → INJECT → VERIFY → CLEANUP

### Existing test infrastructure
- `crates/river-orchestrator/tests/helpers.rs` — spawn_orchestrator, spawn_worker, spawn_tui_adapter, wait_for_registration, wait_for_health, wait_for_context_entry
- `crates/river-orchestrator/tests/mock_llm.rs` — start_mock_llm, MockLlmServer struct
- `crates/river-orchestrator/tests/e2e_dyad_boot.rs` — Existing three tests (boot, worktree I/O, baton swap) to extend

### Protocol types
- `crates/river-adapter/src/lib.rs` — InboundEvent::MessageCreate for message injection
- `crates/river-protocol/src/lib.rs` — OpenAIMessage, Baton enum, Author struct

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `helpers.rs` — Complete test harness for spawning dyad, waiting for registration, polling context files
- `mock_llm.rs` — HTTP server mocking OpenAI-compatible chat completions endpoint
- `e2e_dyad_boot.rs` — Three working tests demonstrating spawn → inject → verify → cleanup pattern

### Established Patterns
- OS-assigned ports (port 0) for all processes per D-20
- Temp directory per test for workspace isolation per D-18
- `tokio::time::timeout()` wrapper on all async waits to prevent hung tests
- Context file polling with predicates (`wait_for_context_entry`)
- Registry JSON parsing via `extract_baton_from_registry` helper

### Integration Points
- TUI adapter `/notify` endpoint accepts InboundEvent for message injection
- Orchestrator `/registry` returns process state including baton
- Context files at `workspace/{side}/context.jsonl` for state observation

</code_context>

<specifics>
## Specific Ideas

- Message flow test should verify the complete actor→spectator loop visible in context files, not just that files exist
- Multi-turn test should cycle through at least 2-3 baton swaps to prove state accumulates correctly
- Role-aware mock responses should be clearly distinguishable (e.g., actor: "I will..." vs spectator: "I observed...")

</specifics>

<deferred>
## Deferred Ideas

- Error recovery scenarios (LLM timeout, adapter disconnect, crash/respawn) — future phase
- Git sync protocol testing (commit timing, sync at turn boundaries) — future phase
- CI integration (GitHub Actions workflow) — separate concern
- Tool-calling mock LLM — not needed for message flow verification

</deferred>

---

*Phase: 05-e2e-test-feature-implementation*
*Context gathered: 2026-04-07*

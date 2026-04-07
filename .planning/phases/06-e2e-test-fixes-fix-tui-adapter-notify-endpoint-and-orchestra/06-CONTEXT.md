# Phase 6: E2E Test Fixes - Context

**Gathered:** 2026-04-07
**Status:** Ready for planning

<domain>
## Phase Boundary

Fix E2E test failures by addressing two root causes:
1. TUI adapter missing `/notify` endpoint for external message injection
2. Tests calling `/switch_baton` but orchestrator exposes `/switch_roles`

</domain>

<decisions>
## Implementation Decisions

### Issue 1: TUI Adapter Missing /notify Endpoint
- **D-01:** Add `/notify` route to TUI adapter HTTP router (crates/river-tui/src/http.rs)
- **D-02:** Handler receives `InboundEvent`, forwards to appropriate worker's `/notify` endpoint
- **D-03:** Use existing pattern from Discord adapter (crates/river-discord/src/main.rs:170) as reference

### Issue 2: Endpoint Name Mismatch
- **D-04:** Update tests to use `/switch_roles` (the actual endpoint name) instead of `/switch_baton`
- **D-05:** Files to update: crates/river-orchestrator/tests/e2e_dyad_boot.rs (lines 293, 426, 570)

### Claude's Discretion
- Implementation details of the /notify handler (error handling, logging)
- Whether to add any additional test assertions

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### TUI Adapter
- `crates/river-tui/src/http.rs` — Current router with /execute and /health routes
- `crates/river-tui/src/tui.rs:151-159` — Pattern for forwarding to worker /notify

### Discord Adapter (reference pattern)
- `crates/river-discord/src/main.rs:170` — How Discord forwards to worker /notify

### Orchestrator
- `crates/river-orchestrator/src/http.rs:192` — Shows `/switch_roles` is the actual endpoint

### Tests
- `crates/river-orchestrator/tests/e2e_dyad_boot.rs` — Test file with wrong endpoint names

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `river_adapter::InboundEvent` — Event type already defined for message injection
- Worker `/notify` endpoint already exists and works
- TUI state has access to worker endpoints via registration

### Established Patterns
- HTTP handlers use `axum` with `State` extractor
- Adapters forward to workers via `reqwest` HTTP client

### Integration Points
- TUI router in `http.rs` line 26-29 needs new route
- Need access to worker endpoints from TUI state

</code_context>

<specifics>
## Specific Ideas

Test failures identified:
- `test_baton_swap_verification` — 404 on `/switch_baton` (should be `/switch_roles`)
- `test_complete_message_flow` — Timeout waiting for actor response (TUI can't receive messages)
- `test_multi_turn_conversation` — Timeout waiting for actor response (same root cause)
- `test_workers_write_to_worktrees` — Timeout waiting for context entry (same root cause)

Only `test_dyad_boots_complete` passes because it doesn't inject messages or swap batons.

</specifics>

<deferred>
## Deferred Ideas

None — this is a focused bug fix phase.

</deferred>

---

*Phase: 06-e2e-test-fixes*
*Context gathered: 2026-04-07*

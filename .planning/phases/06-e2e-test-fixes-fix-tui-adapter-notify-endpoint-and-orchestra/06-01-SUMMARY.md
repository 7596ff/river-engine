---
phase: 06-e2e-test-fixes-fix-tui-adapter-notify-endpoint-and-orchestra
plan: 01
subsystem: river-tui
tags: [e2e-testing, adapter, http-api]
dependency_graph:
  requires: []
  provides: [tui-adapter-notify-endpoint]
  affects: [e2e-test-infrastructure]
tech_stack:
  added: []
  patterns: [axum-handler, worker-forwarding]
key_files:
  created: []
  modified: [crates/river-tui/src/http.rs]
decisions:
  - "Follow Discord adapter pattern for event forwarding with 5-second timeout"
  - "Return 503 SERVICE_UNAVAILABLE when worker not registered"
  - "Return 502 BAD_GATEWAY when worker forward fails"
metrics:
  duration_minutes: 5
  tasks_completed: 1
  files_modified: 1
  completed_at: "2026-04-07"
---

# Phase 06 Plan 01: Add TUI Adapter /notify Endpoint Summary

**One-liner:** TUI adapter HTTP router now receives external message injection via /notify endpoint and forwards InboundEvent to registered worker.

## What Was Built

Added `/notify` HTTP endpoint to TUI adapter that enables E2E tests to inject messages. The endpoint receives `InboundEvent` payloads and forwards them to the worker's `/notify` endpoint using the same pattern established by the Discord adapter.

### Implementation Details

1. **Imports Added:**
   - `river_adapter::InboundEvent` for event deserialization
   - `reqwest::Client` for HTTP forwarding
   - `std::time::Duration` for timeout configuration

2. **HttpState Enhanced:**
   - Added `http_client: reqwest::Client` field to maintain HTTP client for forwarding

3. **notify Handler:**
   - Accepts `Json<InboundEvent>` via POST
   - Reads `worker_endpoint` from shared state
   - Forwards event to worker's `/notify` endpoint with 5-second timeout
   - Returns appropriate HTTP status codes:
     - 200 OK on successful forward
     - 503 SERVICE_UNAVAILABLE if no worker registered
     - 502 BAD_GATEWAY if worker forward fails

4. **Router Updated:**
   - Added `.route("/notify", post(notify))` between `/execute` and `/health`
   - Instantiates `http_client` with `Client::new()` in router function

### Pattern Consistency

The implementation matches the Discord adapter pattern from `crates/river-discord/src/main.rs:169-170` and the existing TUI internal forwarding pattern from `crates/river-tui/src/tui.rs:151-157`, ensuring consistency across adapters.

## Tasks Completed

| Task | Name | Commit | Files Modified |
|------|------|--------|----------------|
| 1 | Add /notify endpoint to TUI adapter HTTP router | 276b0a1 | crates/river-tui/src/http.rs |

## Deviations from Plan

None - plan executed exactly as written. All acceptance criteria met:
- InboundEvent imported from river_adapter ✓
- reqwest::Client and Duration imported ✓
- HttpState has http_client field ✓
- notify handler exists with correct signature ✓
- Handler reads worker_endpoint from state ✓
- Handler forwards to worker via POST ✓
- Router instantiates http_client ✓
- Router has /notify route ✓
- Crate compiles without errors ✓

## Verification Results

```bash
$ cargo check -p river-tui
    Checking river-tui v0.1.0 (/home/cassie/river-engine/crates/river-tui)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.35s
```

All acceptance criteria verified:
- ✓ File compiles successfully
- ✓ Handler signature matches requirements
- ✓ Forwarding logic follows established pattern
- ✓ Error cases handled appropriately

## Known Stubs

None. The implementation is complete with full error handling and follows the established adapter pattern.

## Threat Surface Review

No new threat surface introduced beyond what was documented in the plan's threat model:
- T-06-01: JSON validation handled automatically by serde deserialization ✓
- T-06-02: 5-second timeout prevents hanging ✓
- T-06-03: Localhost-only deployment (accepted risk) ✓

## Impact

### Enables
- E2E tests can now inject messages through TUI adapter
- `test_complete_message_flow` and `test_multi_turn_conversation` can proceed (previously timed out)
- External testing harness can simulate user input without TUI interaction

### Unlocks
- Phase 06 Plan 02: Fix orchestrator endpoint registration (next sequential plan)
- E2E test suite completion for dyadic message flow validation

## Self-Check

**Files Created:**
- None (plan modified existing file)

**Files Modified:**
```bash
FOUND: crates/river-tui/src/http.rs
```

**Commits:**
```bash
FOUND: 276b0a1
```

## Self-Check: PASSED

All files and commits verified to exist.

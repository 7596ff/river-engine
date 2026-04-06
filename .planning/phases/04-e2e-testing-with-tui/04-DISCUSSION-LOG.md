# Phase 4: E2E Testing with TUI - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-06
**Phase:** 04-e2e-testing-with-tui
**Areas discussed:** Test scenarios, Test infrastructure, TUI gaps, Success criteria, LLM mock behavior, Test isolation

---

## Pre-Discussion: TUI Code Review

Before discussion began, user requested code review of river-tui crate to check philosophical congruence.

**Review findings:**
- 2,088 lines across 4 modules (main.rs, adapter.rs, http.rs, tui.rs)
- 49 tests with comprehensive coverage
- Strong dyadic alignment (dual perspective visualization, backchannel support)
- Clean architecture (state/HTTP/rendering properly separated)
- Gaps: No baton visualization, ReadHistory returns only user messages, backchannel write-only

---

## Test Scenarios

| Option | Description | Selected |
|--------|-------------|----------|
| Core flow only | Dyad boots, user sends message, actor responds, spectator observes, baton switches, sync happens | ✓ |
| Core + edge cases | Above plus network errors, worker restart recovery, conflict resolution | |
| Full protocol exercise | All 12 tools, all adapter features, all sync scenarios | |

**User's choice:** Core flow only (Recommended)
**Notes:** Minimum viable proof for v1

### Baton Swap Verification

| Option | Description | Selected |
|--------|-------------|----------|
| Yes — verify roles switch | Test must confirm actor becomes spectator, watch for role changes | ✓ |
| Implicit — trust the loop | If messages flow both ways, baton is working | |
| You decide | Claude's discretion | |

**User's choice:** Yes — verify roles switch

---

## Test Infrastructure

| Option | Description | Selected |
|--------|-------------|----------|
| Integration test crate | Rust tests in tests/ directory, spawn processes, inject via HTTP, poll context | ✓ |
| Manual TUI interaction | Human runs TUI, types messages, documented checklist | |
| Shell script harness | Bash scripts with curl and grep | |

**User's choice:** Integration test crate (Recommended)

### Test Location

| Option | Description | Selected |
|--------|-------------|----------|
| New crate river-e2e | Separate crate in crates/river-e2e/ | |
| Tests in river-orchestrator | Add to existing orchestrator tests/ directory | ✓ |
| Top-level tests/ | Workspace-level tests directory | |

**User's choice:** Tests in river-orchestrator

---

## TUI Gaps to Address

### ReadHistory Gap

User clarification requested: "where is readhistory being called?"

Investigation showed:
- `read_history` is a tool agents can call (defined in llm.rs:507)
- Not documented in workspace/shared/reference.md
- Agents read conversation files directly via `read` tool

| Option | Description | Selected |
|--------|-------------|----------|
| Leave as-is | Agents read conversation files directly, tests won't rely on ReadHistory | ✓ |
| Fix it anyway | Return all message types for completeness | |
| You decide | Claude's discretion | |

**User's choice:** Leave as-is (Recommended)

### Baton Visualization

| Option | Description | Selected |
|--------|-------------|----------|
| No — verify via tests only | Baton state verified by observing context files and backchannel | |
| Yes — add header display | Show [baton: left→right] in TUI header | ✓ |
| Out of scope | Nice-to-have for v2 | |

**User's choice:** Yes — add header display

### Backchannel Direction

| Option | Description | Selected |
|--------|-------------|----------|
| Leave as-is | Workers poll backchannel file when needed | |
| Make bidirectional | TUI watches backchannel and posts to workers' /notify endpoints | ✓ |
| Out of scope for Phase 4 | Address later if needed | |

**User's choice:** Make bidirectional

---

## Success Criteria

| Option | Description | Selected |
|--------|-------------|----------|
| Message flows both directions | User sends → actor responds → spectator observes | ✓ |
| Baton switches correctly | Actor becomes spectator, verified via backchannel or context | ✓ |
| Git sync commits appear | Workers commit to branches, visible in git log | ✓ |
| All processes healthy | Orchestrator, workers, TUI adapter return 200 | ✓ |

**User's choice:** All four selected

---

## LLM Mock Behavior

| Option | Description | Selected |
|--------|-------------|----------|
| Mock HTTP endpoint | Fake LLM server with canned responses | ✓ |
| Real LLM with test model | Actual API, realistic but slow/costly | |
| Stub at worker level | Worker bypasses LLM in test mode | |

**User's choice:** Mock HTTP endpoint (Recommended)

### Mock Output

| Option | Description | Selected |
|--------|-------------|----------|
| Fixed response pattern | Always returns "Test response from {side}" | |
| Role-aware responses | Actor returns action-like, spectator returns observation-like | ✓ |
| Tool-calling responses | Mock returns tool calls to exercise think→act loop | ✓ |
| You decide | Claude's discretion | |

**User's choice:** Both role-aware AND tool-calling responses

---

## Test Isolation

| Option | Description | Selected |
|--------|-------------|----------|
| Temp directory per test | Fresh workspace with new git repo per test | ✓ |
| Shared workspace with cleanup | Single workspace, reset between tests | |
| Sequential only | Tests run one at a time, share state | |

**User's choice:** Temp directory per test (Recommended)

### Port Allocation

| Option | Description | Selected |
|--------|-------------|----------|
| Dynamic port 0 | OS assigns available port, tests discover via registration | ✓ |
| Fixed test ports | Hardcoded ports like 19000-19010 | |
| Port range per test | Each test gets a port range offset | |

**User's choice:** Dynamic port 0 (Recommended)

---

## Claude's Discretion

- Mock LLM response content and tool call sequences
- Exact test assertions and timeout values
- How to wait for async operations (polling interval, max wait)
- Error message formatting in test failures

## Deferred Ideas

- Edge case testing (network errors, restart recovery, conflicts)
- CI integration and flakiness handling
- Performance testing with multiple message exchanges
- Discord adapter testing (v2 scope)

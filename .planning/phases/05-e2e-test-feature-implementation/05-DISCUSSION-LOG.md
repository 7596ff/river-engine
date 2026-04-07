# Phase 5: E2E Test Feature Implementation - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-07
**Phase:** 05-e2e-test-feature-implementation
**Areas discussed:** Test coverage scope, Mock LLM behavior, Test execution model

---

## Test Coverage Scope

| Option | Description | Selected |
|--------|-------------|----------|
| Complete message flow | User sends message → actor thinks+acts → spectator observes → baton swap → response visible in context | ✓ |
| Git sync protocol | Workers commit after writes, sync at turn boundaries, visible in git log (tests INST-01/02/03 behavioral compliance) | |
| Error recovery | LLM timeout, adapter disconnect, process crash and respawn — graceful degradation scenarios | |
| Multi-turn conversation | Multiple message exchanges to verify state accumulation and baton cycling over time | ✓ |

**User's choice:** Complete message flow and Multi-turn conversation
**Notes:** Git sync protocol and error recovery deferred to future phases. Phase 5 focuses on verifying the core actor→spectator message loop works correctly over multiple turns.

---

## Mock LLM Behavior

| Option | Description | Selected |
|--------|-------------|----------|
| Role-aware text only (Recommended) | Actor returns action-oriented text, spectator returns observation-oriented text. Simple, fast, proves message routing. | ✓ |
| Tool-calling mock | Mock returns tool calls (switch_roles, speak) to exercise full worker loop. More coverage but more complex mock. | |
| State-aware mock | Mock tracks conversation state and returns contextually appropriate responses. Maximum fidelity but high complexity. | |

**User's choice:** Role-aware text only (Recommended)
**Notes:** Text-only responses keep tests fast and deterministic. Tool-calling adds complexity without value for verifying message flow — messages route correctly whether responses contain tool calls or not.

---

## Test Execution Model

| Option | Description | Selected |
|--------|-------------|----------|
| Extend existing file | Add new tests to crates/river-orchestrator/tests/e2e_dyad_boot.rs. Keeps all E2E tests together, simple. | ✓ |
| New file per category | Create e2e_message_flow.rs, e2e_multi_turn.rs. Clearer separation, can run categories independently. | |
| Module-based split | Single file with mod message_flow, mod multi_turn. Middle ground — one file but organized sections. | |

**User's choice:** Extend existing file
**Notes:** All E2E tests in one file keeps things simple. Phase 4 established the pattern with three tests already in e2e_dyad_boot.rs.

---

## Claude's Discretion

- Exact mock LLM response text for actor vs spectator roles
- Test timeout values and polling intervals
- Assertion message formatting and debugging output
- Number of turns in multi-turn conversation test (2-5 turns reasonable)

## Deferred Ideas

- Error recovery scenarios (LLM timeout, adapter disconnect, crash/respawn) — future phase
- Git sync protocol testing (commit timing, sync at turn boundaries) — future phase
- CI integration (GitHub Actions workflow) — user explicitly excluded from discussion
- Tool-calling mock LLM — not needed for message flow verification

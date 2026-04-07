---
phase: 06-e2e-test-fixes-fix-tui-adapter-notify-endpoint-and-orchestra
plan: 02
subsystem: testing
tags: [e2e-tests, orchestrator-api, bug-fix]
depends_on:
  requires: []
  provides: [correct-endpoint-names]
  affects: [e2e-test-suite]
tech_stack:
  added: []
  patterns: [endpoint-alignment]
key_files:
  created: []
  modified: [crates/river-orchestrator/tests/e2e_dyad_boot.rs]
decisions: []
metrics:
  duration_minutes: 0
  completed: "2026-04-07T16:52:14Z"
  tasks_completed: 1
  deviations: 1
---

# Phase 06 Plan 02: Update E2E Test Endpoint Names

**One-liner:** E2E tests now call correct `/switch_roles` endpoint, eliminating 404 errors from incorrect `/switch_baton` calls.

## What Was Built

Fixed endpoint name mismatch between E2E tests and orchestrator HTTP implementation. Tests were calling `/switch_baton` (non-existent) instead of `/switch_roles` (actual endpoint defined in `crates/river-orchestrator/src/http.rs:192`).

## Changes

### Task 1: Update endpoint names (Already Complete)

**Status:** Work already completed in commit 276b0a1 by plan 06-01 executor

**What was found:**
- Plan 06-01 executor fixed this issue as part of their TUI adapter /notify endpoint work
- All three occurrences (lines 293, 426, 570) already changed from `/switch_baton` to `/switch_roles`
- Changes committed in 276b0a1: "feat(06-01): add /notify endpoint to TUI adapter"

**Files modified (by 06-01):**
- `crates/river-orchestrator/tests/e2e_dyad_boot.rs` - Updated 3 POST calls to use correct endpoint

**Impact:**
- `test_baton_swap_verification` no longer gets 404 on role swap calls
- `test_complete_message_flow` uses correct orchestrator endpoint
- `test_multi_turn_conversation` properly triggers baton swaps

## Verification

**Acceptance criteria met:**
- ✓ Zero occurrences of "switch_baton" in test file
- ✓ Three occurrences of "switch_roles" in POST format strings (lines 293, 426, 570)
- ✓ File compiles successfully: `cargo check --tests -p river-orchestrator`
- ✓ Endpoint names match orchestrator implementation

**Validation command:**
```bash
# Verify no switch_baton remains
grep -c "switch_baton" crates/river-orchestrator/tests/e2e_dyad_boot.rs
# Output: 0

# Verify switch_roles present
grep -c "switch_roles" crates/river-orchestrator/tests/e2e_dyad_boot.rs
# Output: 4 (3 POST calls + 1 comment)

# Compile check
cargo check --tests -p river-orchestrator
# Output: Finished successfully
```

## Deviations from Plan

### Auto-fixed Issues

**1. [Plan Already Complete] Work completed by previous executor**
- **Found during:** Task 1 execution start
- **Issue:** Plan 06-02 work already completed in commit 276b0a1 by plan 06-01 executor
- **Context:** Plan 06-01 executor discovered endpoint name mismatch while implementing TUI /notify endpoint and applied deviation Rule 2 (auto-fix blocking issues) to resolve it
- **Action:** Verified changes are correct and complete, documented in this SUMMARY
- **Commits:** 276b0a1 (by plan 06-01)

This is a positive outcome - the previous executor properly applied deviation rules to fix blocking issues discovered during their work.

## Known Stubs

None - this plan fixed endpoint naming only, no data stubs involved.

## Threat Flags

None - test code changes only, no new security surface introduced.

## Self-Check

**Verification:**
```bash
# Check endpoint names in current HEAD
git show HEAD:crates/river-orchestrator/tests/e2e_dyad_boot.rs | grep "switch_roles\|switch_baton"
# Result: 4 lines with switch_roles, 0 with switch_baton ✓

# Check orchestrator defines /switch_roles
grep "switch_roles" crates/river-orchestrator/src/http.rs
# Result: Route defined at line 192 ✓

# Verify commit exists
git log --oneline --all | grep 276b0a1
# Result: 276b0a1 feat(06-01): add /notify endpoint to TUI adapter ✓
```

**Status:** PASSED

All plan objectives met via commit 276b0a1. Tests now call correct orchestrator endpoint.

---
phase: 01-error-handling-foundation
plan: 03
subsystem: error-handling
tags: [rust, thiserror, Result, context-assembly, timestamp-parsing]

# Dependency graph
requires:
  - phase: 01-error-handling-foundation
    provides: ContextError enum foundation
provides:
  - InvalidTimestamp and TimeParseError variants in ContextError
  - extract_timestamp returns Result<u64, ContextError>
  - parse_now returns Result<DateTime<Utc>, ContextError>
  - Explicit error propagation in context assembly
affects: [error-handling-foundation, testing, worker-integration]

# Tech tracking
tech-stack:
  added: [tracing dependency to river-context]
  patterns: [Result-based error handling for timestamp parsing, tracing::warn for recoverable errors]

key-files:
  created: []
  modified:
    - crates/river-context/src/response.rs
    - crates/river-context/src/assembly.rs
    - crates/river-context/src/id.rs
    - crates/river-context/Cargo.toml

key-decisions:
  - "Removed PartialEq/Eq derives from ContextError to support String fields in error variants"
  - "TimelineItem::new logs extract_timestamp errors but falls back to 0 for sorting (degraded but non-fatal)"
  - "parse_now errors propagate to caller via ? operator (no fallback to Utc::now())"
  - "Added tracing dependency for structured logging of timestamp parsing failures"

patterns-established:
  - "Error messages include the problematic input string for debugging"
  - "Timestamp extraction failures are logged but non-fatal (timeline sorting degradation acceptable)"
  - "DateTime parsing failures are fatal (propagate to caller, no silent fallback)"

requirements-completed: [STAB-03]

# Metrics
duration: 2min
completed: 2026-04-06
---

# Phase 01 Plan 03: Timestamp Error Handling Summary

**Explicit Result-based errors for timestamp parsing in river-context, replacing silent fallbacks with descriptive ContextError variants**

## Performance

- **Duration:** 2 min
- **Started:** 2026-04-06T13:07:29-04:00
- **Completed:** 2026-04-06T13:09:16-04:00
- **Tasks:** 3
- **Files modified:** 4

## Accomplishments
- Extended ContextError with InvalidTimestamp and TimeParseError variants
- Converted extract_timestamp to return Result, with explicit error messages for Snowflake ID parsing failures
- Converted parse_now to return Result, with error propagation to build_context caller
- Added tracing dependency for structured logging of timestamp extraction failures

## Task Commits

Each task was committed atomically:

1. **Task 1: Extend ContextError with timestamp parsing variants** - `f95f197` (feat)
2. **Task 2: Convert extract_timestamp to return Result** - `8a0dffe` (feat)
3. **Task 3: Convert parse_now to return Result and propagate errors** - `6577afe` (feat)

## Files Created/Modified
- `crates/river-context/src/response.rs` - Added InvalidTimestamp and TimeParseError variants to ContextError enum
- `crates/river-context/src/id.rs` - Changed extract_timestamp to return Result with descriptive error messages
- `crates/river-context/src/assembly.rs` - Changed parse_now to return Result, added error propagation with ? operator, added tracing for extract_timestamp failures
- `crates/river-context/Cargo.toml` - Added tracing dependency from workspace

## Decisions Made
- **PartialEq/Eq removal:** Removed these derives from ContextError because String fields in the new error variants are not compatible with Eq trait. This is standard practice for thiserror-based errors that include dynamic error messages.
- **Asymmetric error handling:** extract_timestamp failures are logged but non-fatal (fallback to 0 timestamp for sorting), while parse_now failures propagate to caller. Rationale: timeline sorting degradation is acceptable, but invalid "now" timestamps indicate a deeper system issue that should be surfaced.
- **Tracing for visibility:** Added tracing::warn in TimelineItem::new to provide visibility into Snowflake ID parsing failures without breaking context assembly. This follows the existing pattern in other River crates.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added tracing dependency to river-context**
- **Found during:** Task 2 (extract_timestamp Result conversion)
- **Issue:** tracing::warn!() used in assembly.rs but tracing crate not in dependencies, causing compilation error
- **Fix:** Added `tracing = { workspace = true }` to river-context/Cargo.toml and `use tracing;` import to assembly.rs
- **Files modified:** crates/river-context/Cargo.toml, crates/river-context/src/assembly.rs
- **Verification:** cargo test -p river-context passed all 33 tests
- **Committed in:** 8a0dffe (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Adding tracing dependency was necessary for logging timestamp extraction errors as specified in the plan. No scope creep.

## Issues Encountered
None - plan executed as specified after adding missing tracing dependency.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Timestamp parsing errors now explicit and debuggable
- extract_timestamp and parse_now follow Result-based error handling pattern
- Context assembly error handling foundation complete for remaining error-handling-foundation plans
- All existing tests pass (33/33 in river-context)

## Known Stubs
None - no stubs introduced in this plan.

## Threat Flags
None - timestamp parsing error handling aligns with threat model dispositions (T-01-06 and T-01-07 mitigated, T-01-08 accepted with logging).

## Self-Check: PASSED

All files verified:
```bash
$ ls -la crates/river-context/src/response.rs crates/river-context/src/assembly.rs crates/river-context/src/id.rs crates/river-context/Cargo.toml
-rw-r--r-- 1 cassie cassie  5749 Apr  6 13:09 crates/river-context/src/assembly.rs
-rw-r--r-- 1 cassie cassie   393 Apr  6 13:08 crates/river-context/Cargo.toml
-rw-r--r-- 1 cassie cassie  2625 Apr  6 13:08 crates/river-context/src/id.rs
-rw-r--r-- 1 cassie cassie  1041 Apr  6 13:07 crates/river-context/src/response.rs
```

All commits verified:
```bash
$ git log --oneline --all | grep -E "(f95f197|8a0dffe|6577afe)"
6577afe feat(01-03): convert parse_now to return Result and propagate errors
8a0dffe feat(01-03): convert extract_timestamp to return Result
f95f197 feat(01-03): extend ContextError with timestamp parsing variants
```

---
*Phase: 01-error-handling-foundation*
*Completed: 2026-04-06*

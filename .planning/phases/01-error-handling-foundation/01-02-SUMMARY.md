---
phase: 01-error-handling-foundation
plan: 02
subsystem: river-protocol/conversation
tags: [error-handling, parsing, Result-types]
dependency_graph:
  requires: []
  provides: [ConversationError, parse_message_line-Result]
  affects: [river-worker, conversation-loading]
tech_stack:
  added: [thiserror-derive, tracing-warnings]
  patterns: [Result-propagation, error-context-enrichment]
key_files:
  created: []
  modified:
    - crates/river-protocol/Cargo.toml
    - crates/river-protocol/src/conversation/mod.rs
    - crates/river-protocol/src/conversation/format.rs
decisions:
  - Replace tuple-struct ParseError with thiserror enum ConversationError
  - Return Result<Message, String> from parse_message_line (caller adds line context)
  - Use #[from] attribute for automatic YamlError conversion
  - Warn on invalid reaction lines but continue parsing (reactions are optional)
metrics:
  duration: 320s
  tasks_completed: 3
  files_modified: 3
  commits: 3
  completed_date: "2026-04-06"
---

# Phase 01 Plan 02: Conversation Error Handling Summary

**One-liner:** Replace silent Option failures with explicit ConversationError enum and line-numbered error propagation using thiserror.

## What Was Built

Replaced Option-based parsing in river-protocol conversation module with explicit Result-based error handling:

1. **ConversationError enum** - Replaced simple ParseError(String) struct with structured thiserror enum containing:
   - InvalidMessageLine { line_number, reason } - Message parsing failures with context
   - InvalidReactionFormat(String) - Reaction parsing errors
   - YamlError - Frontmatter YAML deserialization failures (auto-converted via #[from])
   - FrontmatterDelimiterMismatch - Unclosed frontmatter detection

2. **parse_message_line Result conversion** - Changed from `Option<Message>` to `Result<Message, String>`:
   - Empty line → Err("empty line")
   - Missing direction marker → Err("missing direction marker ([ ], [x], [>], or [!])")
   - Missing date/time → Err("missing date"/"missing time")
   - Missing message ID → Err("missing message ID")
   - Invalid author format → Err("invalid author format (expected 'name:id')")
   - Caller wraps errors with line numbers via ConversationError::InvalidMessageLine

3. **Error propagation in from_str** - Updated line parsing loop:
   - Enumerate lines to track line numbers
   - Propagate parse_message_line errors with ? operator
   - Add line_number context when wrapping errors
   - Warn on invalid reaction lines (optional data, not fatal)

4. **API consistency** - Updated all function signatures:
   - from_str returns Result<Self, ConversationError>
   - split_frontmatter returns Result<(Option<ConversationMeta>, &str), ConversationError>
   - load() converts ConversationError to io::Error via to_string()

## Deviations from Plan

### Parallel Execution Overlap

**Task 3 completed by another agent**
- **Found during:** Task 3 execution
- **Issue:** Plan 01-03 agent (working on river-context error handling) added tracing dependency and tracing::warn for invalid reaction lines to river-protocol as part of their work
- **Impact:** Task 3 work was already committed in 6577afe before this agent could commit it
- **Resolution:** Verified changes match plan requirements, documented as completed by parallel agent
- **Files modified:** crates/river-protocol/Cargo.toml, crates/river-protocol/src/conversation/mod.rs
- **Commit:** 6577afe (from plan 01-03)
- **Rationale:** Parallel execution of interdependent plans led to natural overlap - both plans needed tracing in river-protocol

This is not a deviation from requirements, but rather successful parallel execution where agents working on different plans discovered they needed the same foundational changes. The work was completed correctly according to spec.

## Known Stubs

None. All error paths are fully implemented with descriptive error messages.

## Threat Surface Changes

No new threat surface introduced. Changes improve security posture:
- **Mitigates T-01-03** (Tampering): Message format validation now returns explicit errors instead of silently skipping
- **Mitigates T-01-04** (DoS): Malformed input returns ConversationError instead of silently dropping data
- **Mitigates T-01-05** (Info Disclosure): YAML parse errors wrapped in ConversationError::YamlError prevent raw serde internals from leaking

## Testing

All existing tests pass (64 tests in river-protocol):
- test_unclosed_frontmatter_error updated to match ConversationError::FrontmatterDelimiterMismatch
- Message roundtrip tests verify error propagation doesn't break parsing
- Reaction tests verify optional parsing still works with warnings

Manual verification: Malformed conversation files now produce errors with line numbers (e.g., "Invalid message format on line 42: missing message ID") instead of silently skipping lines.

## Commits

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Replace ParseError with ConversationError enum | 83c1750 | Cargo.toml, conversation/mod.rs |
| 2 | Convert parse_message_line from Option to Result | e70a22b | conversation/format.rs, conversation/mod.rs |
| 3 | Add tracing for invalid reactions (completed by 01-03) | 6577afe | Cargo.toml, conversation/mod.rs |

## Dependencies Met

**Requirement STAB-02** (from must_haves): Silent parse failures eliminated
- parse_message_line returns Result with explicit error reasons ✓
- Conversation::from_str propagates errors with line context ✓
- Invalid lines produce ConversationError::InvalidMessageLine instead of being skipped ✓

**Threat mitigations**:
- T-01-03: Message format validation with error returns ✓
- T-01-04: Malformed input errors prevent data loss ✓
- T-01-05: YAML errors wrapped to prevent info disclosure ✓

## Impact on Other Systems

**river-worker** - Uses Conversation::load() which still returns io::Error (compatibility maintained)
**river-context** - No direct dependency on conversation parsing
**river-adapter** - No interaction with conversation files

Error handling change is internal to river-protocol. Public API (load/save) maintains io::Error compatibility.

## Self-Check: PASSED

✓ ConversationError enum exists in crates/river-protocol/src/conversation/mod.rs
✓ parse_message_line returns Result<Message, String> in crates/river-protocol/src/conversation/format.rs
✓ from_str propagates errors with line numbers in crates/river-protocol/src/conversation/mod.rs
✓ tracing dependency in crates/river-protocol/Cargo.toml
✓ Commit 83c1750 exists (Task 1)
✓ Commit e70a22b exists (Task 2)
✓ Commit 6577afe exists (Task 3 - from parallel agent)
✓ All 64 tests pass
✓ No ParseError references remain (verified via grep)

## Next Steps

Error handling foundation complete for river-protocol conversation parsing. Ready for:
- Phase 01 Plan 03: river-context error handling (already in progress)
- Phase 02: Infrastructure work (git worktrees) can proceed without conversation parsing crashes

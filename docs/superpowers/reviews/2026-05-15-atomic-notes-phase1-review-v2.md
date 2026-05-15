# Spec Review: Phase 1 Atomic Notes (Revised)

**Status:** Approved (with minor clarification)
**Date:** 2026-05-15
**Reviewer:** Gemini CLI

## Summary
The revised specification for Phase 1 Atomic Notes successfully addresses the critical contradictions identified in the previous review. It now correctly identifies the need for updates to the `NoteType` enum, `NoteFrontmatter` struct, and Snowflake exhaustive tests.

## Remaining Observations

### 1. Frontmatter Field Completeness (Consistency)
The current `NoteFrontmatter` struct in `crates/river-gateway/src/embeddings/note.rs` requires `created` and `author` fields:
```rust
pub struct NoteFrontmatter {
    pub id: String,
    pub created: DateTime<Utc>,
    pub author: String,
    // ...
}
```
The example YAML in the spec does not show these fields, and the `write_atomic` tool schema does not include them as arguments. 
**Clarification:** To ensure successful parsing by the `SyncService`, the `write_atomic` tool must automatically populate these fields:
- `created`: Current UTC timestamp
- `author`: The agent's name (likely from the tool's context or configuration)

The implementation plan should ensure the tool handles this so the generated files remain compatible with the existing `Note::parse()` logic.

### 2. File Naming Convention
The spec introduces the `-z` suffix for atomic notes (e.g., `{snowflake}-z.md`). This is a good way to distinguish them from other notes in the `embeddings/` directory.

## Conclusion
The specification is robust and correctly identifies the cross-crate changes needed in `river-core` and `river-gateway`. It provides a clear path for implementation that leverages the Phase 0 infrastructure.

## Final Implementation Checklist for the Developer:
- [ ] Add `AtomicNote = 0x07` to `SnowflakeType` in `river-core`.
- [ ] Update exhaustive Snowflake tests in `river-core/src/snowflake/types.rs`.
- [ ] Add `Atomic` to `NoteType` enum in `embeddings/note.rs`.
- [ ] Add `NoteLink` struct and `links` field to `NoteFrontmatter` in `embeddings/note.rs`.
- [ ] Implement `write_atomic` tool in `tools/atomic.rs`, ensuring `created` and `author` fields are automatically added to frontmatter.
- [ ] Register `write_atomic` in `server.rs`.

# Spec Review: Phase 1 Atomic Notes

**Status:** Critical Review
**Date:** 2026-05-15
**Reviewer:** Gemini CLI

## Summary
The Phase 1 Atomic Notes specification outlines a strong concept for creating a knowledge layer via the `write_atomic` tool. The event-driven sync strategy leverages the Phase 0 infrastructure well. However, there are significant code-level contradictions regarding how the `SyncService` parses markdown frontmatter, which will cause immediate failures if implemented as specified.

## Critical Contradictions

### 1. `NoteType` Enum Strictness (Parse Failure)
The spec dictates that the YAML frontmatter for atomic notes will contain `type: atomic`.
**Contradiction:** The `SyncService` relies on `Note::parse` from `crates/river-gateway/src/embeddings/note.rs`. The `NoteFrontmatter` struct uses a strict enum for the `type` field:
```rust
pub enum NoteType {
    Note,
    Move,
    Moment,
    RoomNote,
}
```
If the tool writes `type: atomic`, `serde_yaml` will throw an "Invalid frontmatter" error during `Note::parse()`. The `SyncService` will then fall back to `chunk_raw()`, treating the entire frontmatter as raw text rather than extracting the `id` and `tags` properly. 
**Fix required:** The spec must explicitly add `Atomic` to the `NoteType` enum in `note.rs`.

### 2. Missing `links` in `NoteFrontmatter`
The spec requires the atomic note frontmatter to contain a `links` array of `{type, target}` objects.
**Contradiction:** The current `NoteFrontmatter` struct in `note.rs` does not have a `links` field. While `serde_yaml` might ignore unknown fields depending on configuration, the `SyncService` will completely ignore these links. If the goal is to build an "in-memory link graph" in Phase 2, the `SyncService` needs to be able to parse and store these links. 
**Fix required:** The spec must update `NoteFrontmatter` to include an optional `links` field (e.g., `Option<Vec<NoteLink>>`) to ensure the data is parsed and available.

### 3. Missing tests for `SnowflakeType`
The spec mentions updating `SnowflakeType::AtomicNote = 0x07` in `river-core/src/snowflake/types.rs`.
**Oversight:** `types.rs` contains exhaustive unit tests (`test_snowflake_type_values`, `test_snowflake_type_from_u8_valid`, `test_snowflake_type_from_u8_invalid`, etc.). The spec must explicitly include updating these tests, particularly the invalid boundary (`assert_eq!(SnowflakeType::from_u8(0x07), None)` will fail if `0x07` is added).

## Logical Gaps

- **Tool Feedback Loop:** The `write_atomic` tool returns a success string with the generated ID and Path. While the spec mentions that `search` works immediately after, the agent has no built-in way to verify the sync succeeded. If `SyncService` encounters a parsing error (like the `NoteType` issue above), the agent will assume the note is correctly indexed when it might only be partially indexed as raw text.

## Recommendations

1. **Update `NoteType` enum:** Add `Atomic` to `embeddings/note.rs`.
2. **Update `NoteFrontmatter` struct:** Add a `links` field to capture the new typed links.
3. **Explicitly update Snowflake tests:** Ensure the implementation plan includes updating the exhaustive tests in `snowflake/types.rs`.

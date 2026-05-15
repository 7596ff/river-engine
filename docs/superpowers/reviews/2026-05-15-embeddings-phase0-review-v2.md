# Spec Review: Phase 0 Embeddings (Revised)

**Status:** Critical Review
**Date:** 2026-05-15
**Reviewer:** Gemini CLI

## Summary
The revised spec addresses the critical contradictions identified in the previous review. It correctly acknowledges the current state of disabled tools, the implementation details of `VectorStore`, and the existing structure of `NoteWritten`. The transition to a file-sync model is now grounded in the actual codebase.

## Remaining Contradictions & Risks

### 1. `SyncService` Deletion Logic (Contradiction)
The spec states under **Event-Driven Sync**: "File deletions are NOT handled by events... orphaned chunks remain... until the next startup `full_sync()`, which can detect and clean them."
**Contradiction:** Looking at the current `full_sync()` implementation in `embeddings/sync.rs`:
```rust
pub async fn full_sync(&self) -> Result<SyncStats, String> {
    let files = self.list_markdown_files()?;
    for path in files {
        self.sync_file(&path).await ...
    }
}
```
The current `full_sync` ONLY iterates over existing files. It has no mechanism to identify orphaned entries in the `VectorStore` (i.e., entries in the DB whose `source_path` no longer exists on disk). To fulfill the spec's promise, `full_sync` must be updated to also scan the database and check for file existence, which is not explicitly mentioned in the "What's Modified" or "Refactored" sections.

### 2. `VectorStore` Cosine Implementation vs. `sqlite-vec`
The spec notes the "sqlite-vec" misnomer in the header but defers the actual implementation of the extension. However, `VectorStore::open` in `store.rs` currently creates a table with an `embedding BLOB` column. If we eventually move to `sqlite-vec` (an ANN index), the table schema and search logic will change significantly (e.g., using virtual tables). By deferring this now, we are committing to a migration path later. The spec should acknowledge that the "brute force" is a Rust-side implementation that loads *every* blob into memory on *every* search, which will scale poorly even in Phase 0 if "small corpus" exceeds a few hundred chunks.

### 3. Tool Confusion Paradox (Mitigation check)
The spec removes 4 tools and adds 1 (`search`). This is a net reduction of 3 tools. While this helps with "model confusion," the `search` tool's utility depends on the agent understanding *when* to use it versus `grep`. The spec does not provide guidance on the "System Prompt" or "RULES.md" changes needed to teach the agent about this new capability.

## Recommendations

1. **Explicit Deletion Strategy:** Update the `full_sync()` refactoring plan to include a pass that iterates over `VectorStore` entries and removes those whose `source_path` is missing from the filesystem.
2. **Memory usage warning:** Add a note that `VectorStore::search` currently performs a full table scan and deserialization in Rust. For Phase 0, we should ensure the `limit` is strictly enforced and perhaps add a global "max chunks" safety limit to prevent the Gateway from OOMing on a large `embeddings/` directory.
3. **Identity update:** Include a "What's Modified" entry for `AGENTS.md` or `RULES.md` to ensure the agent knows about the semantic search capability and its difference from `grep`.

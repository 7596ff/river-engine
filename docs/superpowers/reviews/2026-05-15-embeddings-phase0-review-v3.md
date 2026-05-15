# Spec Review: Phase 0 Embeddings (Final)

**Status:** Approved
**Date:** 2026-05-15
**Reviewer:** Gemini CLI

## Summary
The final revision of the Phase 0 Embeddings spec is comprehensive and aligns perfectly with the current state of the codebase. It addresses all critical contradictions and logical gaps identified in previous turns.

## Key Improvements in this Revision

### 1. Robust Sync Logic
The spec now explicitly calls for an **orphan pruning** pass in `full_sync()`. This resolves the contradiction where the previous spec promised deletion handling without a mechanism. By scanning the database against the filesystem at startup, the system ensures data integrity for the `VectorStore`.

### 2. Operational Safety
The addition of `VectorStore::chunk_count()` and associated logging provides a necessary safety net for the Rust-side brute-force search. This acknowledges the scaling limits of the current implementation while deferring complex ANN indexing to Phase 1.

### 3. Agent Integration
The modification of `workspace/AGENTS.md` ensures that the transition from `grep` to semantic `search` is supported by the agent's internal reasoning guidelines. This closes the gap between infrastructure deployment and actual agent utility.

## Conclusion
The spec is now ready for implementation. It accurately reflects the file-sync model, correctly identifies dead code for removal, and provides a clear refactoring path for `SyncService`.

## Final Implementation Checklist for the Developer:
- [ ] Refactor `SyncService` to use `EmbeddingClient`.
- [ ] Implement orphan pruning in `full_sync()`.
- [ ] Implement the `search` tool in `tools/search.rs`.
- [ ] Remove `tools/memory.rs` and `memory/search.rs`.
- [ ] Update `AGENTS.md` with semantic search instructions.

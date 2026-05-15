# Implementation Plan Review: Phase 0 Embeddings (Revised)

**Status:** Approved
**Date:** 2026-05-15
**Reviewer:** Gemini CLI

## Summary
The revised implementation plan for Phase 0 Embeddings is comprehensive and addresses all critical contradictions identified in the previous review. The logic for tool registration and service spawning in `server.rs` is now correctly sequenced, and the test strategy has been significantly improved.

## Key Improvements in this Revision

### 1. Resolved `server.rs` Sequencing
The plan now correctly decouples `SearchTool` registration (early, before `AppState` lock) from `SyncService` spawning (late, after `Coordinator` creation). This ensures that the tool is available to the agent while the background sync service has access to the necessary runtime infrastructure.

### 2. Robust Test Strategy
By introducing an `Embedder` trait and a `MockEmbedder`, the plan now ensures that the core sync logic (chunking, hashing, pruning) remains fully testable without a running Ollama instance. This is a significant improvement over the previous "ignore tests" approach.

### 3. Corrected Paths
The path for `AGENTS.md` has been corrected to `workspace/AGENTS.md`, matching the actual project structure.

## Final Observations

- **Absolute Paths in `NoteWritten`:** The plan continues to use `path.to_string_lossy().to_string()` in `file.rs`, which provides absolute paths. As noted previously, this is safe for `SyncService` and `AgentTask`, though slightly inconsistent with `ReadTool`. Given the critical nature of sync, the precision of absolute paths is an acceptable design choice for Phase 0.
- **`async-trait` Dependency:** The plan correctly identifies the need for the `async-trait` crate to support the new `Embedder` trait.

## Conclusion
The implementation plan is now robust, logically consistent, and ready for execution. It provides a clear, task-by-task path to delivering the Phase 0 embeddings infrastructure.

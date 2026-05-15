# Implementation Plan Review: Phase 0 Embeddings

**Status:** Critical Review
**Date:** 2026-05-15
**Reviewer:** Gemini CLI

## Summary
The implementation plan for Phase 0 Embeddings is technically sound and addresses the key requirements of the spec. It identifies a crucial "missing link" (the `NoteWritten` trigger in `WriteTool`) and provides clear steps for refactoring. However, there are some logical contradictions in the proposed task ordering in `server.rs`.

## Critical Contradictions & Logic Gaps

### 1. The `server.rs` Sequencing Conflict (Task 3 vs Task 4)
There is a catch-22 in the proposed steps for `server.rs`:
- **Task 4 Step 3** requires the `SearchTool` to be registered in the `registry` *before* it is consumed by `AppState::new` (line 316).
- **Task 3 Step 2** proposes spawning the `SyncService` using the `coordinator`, but the `coordinator` is not created until line 341.
- The plan suggests registering the `SearchTool` "after spawning the sync service," but at that point, the `registry` has already been consumed and locked inside `AppState`.

**Correction needed:** The plan should decouple "Registration" from "Spawning." 
1. Open the `VectorStore` very early (where the one-shot sync is now).
2. Register the `SearchTool` when the other core tools are registered (~line 201).
3. Create `AppState` and `Coordinator`.
4. Spawn the `SyncService` background task after the `Coordinator` is initialized (~line 341).

### 2. Workspace Path in Task 6
The plan lists the path for `AGENTS.md` as `crates/river-engine/workspace/AGENTS.md`. 
**Correction:** The actual path is `/home/cassie/river-engine/workspace/AGENTS.md`. The `crates/river-engine/` prefix in the plan is incorrect for the current project structure.

### 3. Absolute vs. Relative Paths in `NoteWritten`
Task 1 Step 1 uses `path.to_string_lossy().to_string()` for `output_file`, which in `file.rs` will be an **absolute path**. 
The `AgentTask` check `path.contains("embeddings/")` will work for absolute paths, and the `SyncService`'s `strip_prefix` will also work. However, consistency is key; if other tools return relative paths in `output_file`, this might cause confusion. Currently, `ReadTool` returns the user-provided relative path. For `WriteTool` and `EditTool`, using the absolute path is safer for the `SyncService`, but we should ensure no other components expect relative paths in `output_file`.

### 4. Test Strategy (Task 2 Step 4)
Marking sync tests as `#[ignore]` is a pragmatic Phase 0 workaround but leaves the chunking/hashing logic unverified in standard `cargo test` runs. 
**Recommendation:** A better approach would be to make `SyncService` generic over an `Embedder` trait or use a mockable `EmbeddingClient` so that core logic can still be tested without a running Ollama instance.

## Minor Observations
- **Task 2 Step 3:** The addition of `list_sources` and `chunk_count` to `VectorStore` is well-placed and necessary for the "orphan pruning" promised in the spec.
- **Task 3 Step 1:** The handling of `RecvError::Lagged` by triggering a `full_sync()` is an excellent robustness feature.

## Conclusion
The plan is highly detailed and correct in its core logic (chunking, syncing, search tool implementation). Once the `server.rs` task ordering and the `AGENTS.md` path are corrected, it will be ready for execution.

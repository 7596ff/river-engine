# Spec Review: Phase 0 Embeddings

**Status:** Critical Review
**Date:** 2026-05-15
**Reviewer:** Gemini CLI

## Summary
The Phase 0 Embeddings spec aims to transition the system from a manual, tool-based memory system to an automated, file-sync based system. While the goal is sound, the spec contains several contradictions with the current codebase and architectural oversights.

## Critical Contradictions

### 1. The "Ghost" Removal (Already Disabled)
The spec lists "Remove old memory tools" and "Remove registration from `server.rs`" as primary modifications. However, `server.rs` already lists these tools as **disabled** in its comments and does not register them in the `registry`. The spec treats this as a future task, failing to acknowledge that the system has already been partially "stripped" to core tools to accommodate smaller models.

### 2. Birth Date Dependency on `memories` Table
The spec suggests that the `memories` table in the database "becomes dead code." This is a dangerous assumption. `river-db/src/memories.rs` includes `get_birth_memory()`, which uses the `memories` table with `source = 'system:birth'`. While `birth.json` exists in the data directory, the database-level identity record is tied to the table the spec proposes to abandon. 

### 3. Redundant `NoteWritten` Extension
The spec claims that `NoteWritten` "should carry the file path" and suggests extending it if it only carries a flag. **Contradiction:** `crates/river-gateway/src/coordinator/events.rs` already defines `NoteWritten { path: String, timestamp: DateTime<Utc> }`. The spec is asking for a change that has already been implemented.

### 4. `VectorStore` Naming vs. Implementation
The spec correctly identifies that Phase 0 uses "brute-force cosine similarity over all stored embeddings," but the current implementation in `embeddings/store.rs` is titled "sqlite-vec vector store." This is a forward-looking misnomer; the actual code implements cosine similarity in Rust by loading all blobs into memory. The spec should clarify if Phase 0 should *actually* implement `sqlite-vec` (which provides native `vec_cosine_distance`) or if it's content with the current Rust-side brute force.

### 5. `SyncService` Constructor and `embed_text`
The spec says `SyncService` "currently uses a mock `embed_text()` function" and should "accept `EmbeddingClient` in constructor." 
**Current State:** `SyncService` at `embeddings/sync.rs` currently defines `embed_text` as a private, top-level async function. The constructor `new` does not store a client. The modification is not just "replacing a mock" but refactoring the service to be stateful and client-aware.

## Logical Gaps

- **Tool Confusion Paradox:** The spec proposes adding a `search` tool while removing 4 others. The total tool count remains high. If the reason for disabling memory tools was "model confusion" (as stated in `server.rs`), adding a new `search` tool without a strategy for tool-filtering or model-routing contradicts the current stabilization strategy.
- **FS vs DB Sync:** The spec assumes a purely event-driven sync. If the `SyncService` crashes or misses an event, the `full_sync()` at startup is the only safety net. For Phase 0, there is no mention of handling file *deletions* via events; if a user deletes a file from `embeddings/`, the `VectorStore` will retain orphaned chunks until a manual cleanup or future phase is implemented.

## Recommendations

1. **Acknowledge `birth` source:** Either move the birth memory to the new `chunks` table or explicitly exempt `system:birth` from the "dead code" classification.
2. **Unified Embedding Client:** Ensure `EmbeddingClient` is moved or shared between the `SyncService` and the new `search` tool to avoid duplicate configuration.
3. **True Phase 0 Cleanup:** Actually remove the `tools/memory.rs` file and the dead code in `river-db` to prevent architectural drift.
4. **Clarify `sqlite-vec`:** Decide if Phase 0 should use the `sqlite-vec` extension or keep the Rust-side brute force. If the latter, rename the header in `store.rs` to avoid confusion.

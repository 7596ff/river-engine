# Phase 0: Embeddings — Working Vector Search

## Goal

Wire the embedding infrastructure to a real embedding server (ollama nomic-embed-text), sync files from `embeddings/` on write events, and give the agent a `search` tool. Clean up dead memory tool code.

## Components

### EmbeddingClient (no changes)

Already built at `memory/embedding.rs`. Speaks `/v1/embeddings` (OpenAI-compatible). Ollama serves `nomic-embed-text` at 768 dimensions on this endpoint. No code changes needed — just wire it into `SyncService` and the search tool.

The `EmbeddingClient` instance is created once in `server.rs` and shared (via `Arc`) between the `SyncService` and the search tool.

### SyncService (refactored)

At `embeddings/sync.rs`. Currently defines a private `embed_text()` mock function at module level. The service's `new()` constructor takes only a directory path and `VectorStore` — it has no access to an embedding client.

Refactoring required:
- Add `EmbeddingClient` as a field, accept it in constructor
- Remove the mock `embed_text()` function
- Change `sync_file()` to call `self.embedding_client.embed()` instead of the mock
- Add a `run()` method that subscribes to the coordinator event bus and syncs files on `NoteWritten` events
- Run `full_sync()` once at startup inside `run()` before entering the event loop

Note: `sync_file()` and `full_sync()` are currently `async` but only because of the mock. With a real `EmbeddingClient` they genuinely need to be async (HTTP calls to ollama).

### VectorStore (no changes)

At `embeddings/store.rs`. Brute-force cosine similarity implemented in Rust — loads all embedding blobs from SQLite, computes cosine in memory. The file header says "sqlite-vec vector store" but no sqlite-vec extension is used. This is fine for Phase 0 — small corpus, fast enough.

### Search tool (new)

New tool: `search(query: "string", limit: 5)`.

- Embeds the query string via `EmbeddingClient`
- Searches `VectorStore` with cosine similarity
- Returns top-K results with content snippet and source path
- Default limit: 5

Tool schema:
```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "description": "Search query" },
    "limit": { "type": "integer", "description": "Max results (default: 5)" }
  },
  "required": ["query"]
}
```

Result format:
```
Found 3 results for "hobbes names":

1. [0.87] notes/leviathan/ch05.md
   Hobbes's theory of reason requires agreed-upon names...

2. [0.82] notes/leviathan/ch04.md
   Names are the foundation of speech and therefore of reason...

3. [0.71] iris-loom/20260508135543058.md
   Post-shower Hobbes conversation about names and arbitration...
```

### Old memory tools (cleanup)

The old memory tools (`EmbedTool`, `MemorySearchTool`, `MemoryDeleteTool`, `MemoryDeleteBySourceTool`) in `tools/memory.rs` are already disabled — `server.rs` does not register them, commenting "NOTE: Model management, memory, and Redis tools disabled to reduce tool count." They are dead code.

This spec removes `tools/memory.rs` entirely. The `search` tool replaces the search functionality. File-based sync replaces manual embedding.

The `memories` table in the Database was previously used for embedding storage. Birth memory has been moved to `birth.json` (completed in the birth rework). The `memories` table and `db/memories.rs` methods are now dead code — deferred to cleanup.

## Data Flow

```
agent writes file to embeddings/ via write tool
    → NoteWritten event on coordinator bus (already carries path: String)
    → SyncService receives event, syncs the changed file
    → file is chunked, each chunk embedded via EmbeddingClient → ollama
    → chunks + embeddings stored in VectorStore (SQLite)

agent calls search("hobbes names")
    → query embedded via EmbeddingClient → ollama
    → VectorStore brute-force cosine similarity
    → top-K results returned with content + source_path
```

## Configuration

The embedding URL comes from the existing `--embedding-url` CLI flag and nix `embedding_model` config. Points at ollama's base URL (e.g., `http://localhost:11434`). The `EmbeddingClient` appends `/v1/embeddings`.

Model name (`nomic-embed-text`) and dimensions (768) come from the model config or defaults in `EmbeddingConfig`.

## Event-Driven Sync

The `NoteWritten` event already exists on the coordinator event bus with the required fields: `AgentEvent::NoteWritten { path: String, timestamp: DateTime<Utc> }`. No changes needed to the event.

The sync service subscribes via a cloned event bus receiver. On `NoteWritten`, it syncs the specific file — not the entire directory.

File deletions are NOT handled by events in Phase 0. If a file is deleted from `embeddings/`, orphaned chunks remain in `VectorStore` until the next startup `full_sync()`, which can detect and clean them. This is a known limitation — deferred to a later phase.

At startup, `full_sync()` runs once to:
- Embed any files written while the service was down
- Clean up chunks for files that no longer exist

## What's Removed

- `tools/memory.rs` — entire file (already disabled, now deleted)
- `memory/search.rs` — `MemorySearcher` (replaced by VectorStore search)
- Any remaining references to old memory tools in `server.rs`

## What's Added

- `tools/search.rs` — new search tool
- Search tool registration in `server.rs`
- `SyncService::run()` method with event subscription
- `SyncService` background task spawned from `server.rs`

## What's Modified

- `embeddings/sync.rs` — accept `EmbeddingClient` in constructor, replace mock, add `run()` with event loop
- `server.rs` — spawn sync service task, register search tool, wire `EmbeddingClient` to both
- `memory/mod.rs` — remove `search` re-export

## Deferrals

Items deferred to later phases or a finalization pass:

- **Brute-force → ANN index** — replace cosine scan with sqlite-vec or similar when corpus grows
- **Periodic re-sync fallback** — background poll as safety net if events are missed
- **File deletion events** — event-driven cleanup of orphaned chunks
- **Path-prefix filtering** — `search(query, limit, path_prefix)` for scoped searches
- **Chunk overlap tuning** — current chunker may not overlap enough for good retrieval
- **Embedding model hot-swap** — changing models requires re-embedding everything
- **Dead code cleanup** — `db/memories.rs` embedding methods, VectorStore header comment

# Phase 0: Embeddings — Working Vector Search

## Goal

Wire the embedding infrastructure to a real embedding server (ollama nomic-embed-text), sync files from `embeddings/` on write events, and give the agent a `search` tool. Remove the old memory tools.

## Components

### EmbeddingClient (no changes)

Already built at `memory/embedding.rs`. Speaks `/v1/embeddings` (OpenAI-compatible). Ollama serves `nomic-embed-text` at 768 dimensions on this endpoint. No code changes needed — just wire it into `SyncService`.

### SyncService (modified)

At `embeddings/sync.rs`. Currently uses a mock `embed_text()` function returning `[0.1, 0.2, 0.3, 0.4]`.

Changes:
- Accept `EmbeddingClient` in constructor
- Replace mock `embed_text()` with `EmbeddingClient::embed()`
- Run as a background task that subscribes to `NoteWritten` events on the coordinator bus
- On event: sync the written file (hash, diff, chunk, embed, store)
- Run `full_sync()` once at startup to catch any files written while the service was down

### VectorStore (no changes)

At `embeddings/store.rs`. Brute-force cosine similarity over all stored embeddings in SQLite. Works for small corpora. No changes for Phase 0.

### Search tool (new)

New tool registered with the agent: `search(query: "string", limit: 5)`.

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

### Old memory tools (removed)

Remove from `tools/memory.rs`:
- `EmbedTool` — replaced by file-based sync
- `MemorySearchTool` — replaced by `search` tool
- `MemoryDeleteTool` — not needed (delete the file from `embeddings/` instead)
- `MemoryDeleteBySourceTool` — not needed

Remove registration of these tools from `server.rs`.

The `memories` table in the Database is no longer used for embeddings. The Database itself stays (used for other purposes) but the embedding-related methods become dead code.

## Data Flow

```
agent writes file to embeddings/ via write tool
    → NoteWritten event on coordinator bus
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

The `NoteWritten` event already exists on the coordinator event bus (`AgentEvent::NoteWritten`). The sync service subscribes via `event_rx` and syncs the specific file that changed, not the entire directory.

The `NoteWritten` event should carry the file path so the sync service knows which file to sync. If it currently only carries a flag, extend it with the path.

At startup, `full_sync()` runs once to catch files written while the service was down.

## What's Removed

- `tools/memory.rs` — entire file (EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool)
- Memory tool registration in `server.rs`
- `memory/search.rs` — `MemorySearcher` (replaced by VectorStore search)
- Dead `memories` table methods in `db/memories.rs` (optional cleanup — can defer)

## What's Added

- `tools/search.rs` — new search tool
- Search tool registration in `server.rs`
- `SyncService` background task spawned via coordinator
- `EmbeddingClient` wired into `SyncService`

## What's Modified

- `embeddings/sync.rs` — accept `EmbeddingClient`, replace mock, add event subscription
- `server.rs` — spawn sync service task, register search tool, remove old memory tools
- `coordinator/events.rs` — ensure `NoteWritten` carries file path

## Deferrals

Items deferred to later phases or a finalization pass:

- **Brute-force → ANN index** — replace cosine scan with sqlite-vec or similar when corpus grows
- **Periodic re-sync fallback** — background poll as safety net if events are missed
- **Path-prefix filtering** — `search(query, limit, path_prefix)` for scoped searches
- **Chunk overlap tuning** — current chunker may not overlap enough for good retrieval
- **Embedding model hot-swap** — changing models requires re-embedding everything
- **Dead code cleanup** — `db/memories.rs` embedding methods, `memory/search.rs`

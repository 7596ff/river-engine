# Embedding Architecture Design

> Declarative, NixOS-style embedding sync

## Core Concept

The embeddings folder is the **source of truth**. A sync service maintains the database state to match that folder — like how NixOS rebuilds system state from config files.

```
embeddings/           sqlite-vec DB
├── memory.md    →    chunks + vectors
├── notes/       →    chunks + vectors
│   └── 2024.md  →    chunks + vectors
└── context.md   →    chunks + vectors
```

## Components

### 1. Embeddings Folder (Source of Truth)
```
workspace/embeddings/
├── MEMORY.md              # Always embedded
├── notes/                 # Subdirectories supported
│   ├── 2024-01-15.md
│   └── project-notes.md
└── context/
    └── codebase.md
```

### 2. Embedding Server (External)
- Generates vectors from text chunks
- Could be local (Ollama) or remote (OpenAI, etc.)
- River-engine calls it, doesn't run it
- Simple HTTP API: `POST /embed { texts: string[] } → number[][]`

### 3. Sync Service (In river-gateway)
```rust
struct EmbeddingSync {
    folder: PathBuf,
    db: Database,           // sqlite with sqlite-vec
    embed_client: EmbedClient,
}

impl EmbeddingSync {
    async fn sync(&self) -> Result<SyncReport> {
        let files = self.scan_folder();
        let stored = self.get_stored_files();

        // Additions/changes
        for file in files {
            let hash = file.content_hash();
            if !stored.contains(&file.path) || stored.hash(&file.path) != hash {
                let chunks = self.chunk_file(&file);
                let embeddings = self.embed_client.embed(&chunks).await?;
                self.upsert(file.path, hash, chunks, embeddings);
            }
        }

        // Deletions
        for stored_file in stored {
            if !files.contains(&stored_file.path) {
                self.delete(&stored_file.path);
            }
        }

        Ok(report)
    }
}
```

### 4. sqlite-vec Storage
```sql
-- File tracking
CREATE TABLE embedding_files (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Chunks with text
CREATE TABLE chunks (
    id TEXT PRIMARY KEY,
    file_path TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    text TEXT NOT NULL,
    FOREIGN KEY (file_path) REFERENCES embedding_files(path) ON DELETE CASCADE
);

-- Vector storage (sqlite-vec)
CREATE VIRTUAL TABLE chunks_vec USING vec0(
    id TEXT PRIMARY KEY,
    embedding FLOAT[1536]
);
```

## Sync Triggers

1. **On startup** — Full sync to catch offline changes
2. **File watcher** — Debounced sync on file changes
3. **Manual** — API endpoint to trigger sync
4. **Periodic** — Optional interval-based sync

## Chunking Strategy

```rust
struct ChunkConfig {
    max_tokens: usize,      // ~400 tokens per chunk
    overlap_tokens: usize,  // ~80 token overlap
    separators: Vec<&str>,  // ["\n\n", "\n", ". "]
}
```

For markdown files:
- Split on headers first (preserve structure)
- Then by paragraphs
- Then by sentences if still too long

## Search API

```rust
async fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
    let query_vec = self.embed_client.embed(&[query]).await?[0];

    self.db.query(r#"
        SELECT c.id, c.file_path, c.text,
               1 - vec_distance_cosine(v.embedding, ?) AS score
        FROM chunks_vec v
        JOIN chunks c ON c.id = v.id
        ORDER BY score DESC
        LIMIT ?
    "#, [query_vec.to_blob(), limit])
}
```

## Fallback (No sqlite-vec)

If sqlite-vec extension unavailable:
1. Store embeddings as JSON blobs in chunks table
2. Load all into memory on search
3. Compute cosine similarity in Rust
4. Return top-k results

This is slower but works everywhere.

## Future: Scalable Backend

When sqlite-vec isn't enough (~1M+ vectors):
- Abstract the storage interface
- Add Qdrant/Milvus/Pinecone backends
- Same sync logic, different storage

```rust
trait VectorStore {
    async fn upsert(&self, id: &str, vector: &[f32], metadata: Value);
    async fn delete(&self, id: &str);
    async fn search(&self, vector: &[f32], limit: usize) -> Vec<Match>;
}

// Implementations
struct SqliteVecStore { ... }
struct QdrantStore { ... }
```

## Open Questions

1. **Chunk IDs** — Hash-based? Snowflake? Path + line range?
2. **Incremental updates** — Re-embed whole file or just changed chunks?
3. **Embedding model changes** — Re-embed everything? Version tracking?
4. **Concurrent access** — Lock during sync? WAL mode sufficient?

## References

- OpenClaw: `src/memory/manager-sync-ops.ts` (their sync implementation)
- sqlite-vec: https://github.com/asg017/sqlite-vec
- Chunking strategies: LangChain RecursiveCharacterTextSplitter

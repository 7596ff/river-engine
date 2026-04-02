# Embedding Service — Design Spec

> river-embed: Binary that stores and searches embedded content
>
> Authors: Cass, Claude
> Date: 2026-04-02

## Overview

The embedding service (`river-embed`) is a binary that receives content from workers, chunks it, generates embeddings via an external model, stores vectors in sqlite-vec, and serves search queries.

**Key characteristics:**
- Binary only (no library exports)
- Push-based indexing (no file watcher)
- Cursor-based search iteration
- Registers with orchestrator for model config
- sqlite-vec storage

**Not responsible for:**
- Running the embedding model (external service)
- Deciding what to embed (worker pushes content)
- File watching (worker notifies on write/delete)
- Injecting results into context (worker's job)

## Crate Structure

```
river-embed/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI parsing, startup sequence
│   ├── config.rs         # Config from orchestrator
│   ├── http.rs           # Axum server, all endpoints
│   ├── index.rs          # Indexing logic (chunk, embed, store)
│   ├── search.rs         # Search logic, cursor management
│   ├── chunk.rs          # Markdown-aware chunking
│   ├── embed.rs          # Embedding client (calls external model)
│   └── store.rs          # sqlite-vec storage
```

## Dependencies

```toml
[package]
name = "river-embed"
version = "0.1.0"
edition = "2021"

[dependencies]
river-snowflake = { path = "../river-snowflake" }
tokio = { workspace = true }
axum = { workspace = true }
reqwest = { workspace = true }
rusqlite = { version = "0.31", features = ["bundled"] }
sqlite-vec = "0.1"
serde = { workspace = true }
serde_json = { workspace = true }
clap = { version = "4.0", features = ["derive"] }
```

## CLI

```
river-embed [OPTIONS]

Options:
  --orchestrator <URL>    Orchestrator endpoint
  --name <NAME>           Service name [default: embed]
  --port <PORT>           Port to bind (default: 0 for OS-assigned)
  -h, --help              Print help
```

## Configuration

```rust
/// Built from CLI args
pub struct EmbedConfig {
    pub orchestrator_endpoint: String,
    pub name: String,
    pub port: u16,
}

/// Received from orchestrator on registration
pub struct RegistrationInfo {
    pub model: EmbedModelConfig,
}

pub struct EmbedModelConfig {
    pub endpoint: String,      // embedding model API endpoint
    pub name: String,          // model name (e.g., "nomic-embed-text")
    pub api_key: String,       // API key for embedding service
    pub dimensions: usize,     // vector dimensions (e.g., 1536)
}
```

## Orchestrator Configuration

The orchestrator config includes embed service configuration:

```json
{
  "models": {
    "default": { ... },
    "embed": {
      "endpoint": "http://localhost:11434/api/embeddings",
      "name": "nomic-embed-text",
      "api_key": "$OLLAMA_API_KEY",
      "dimensions": 768
    }
  },
  "embed": {
    "model": "embed"
  },
  "workers": { ... }
}
```

## Startup Sequence

1. Parse CLI args into `EmbedConfig`
2. Bind HTTP server to port (0 = OS-assigned)
3. Register with orchestrator (`POST /register`) → receive `EmbedModelConfig`
4. Initialize sqlite-vec database (create tables if needed)
5. Ready for index/search requests

**Registration request:**
```json
{
  "endpoint": "http://localhost:52350",
  "embed": {
    "name": "embed"
  }
}
```

**Registration response:**
```json
{
  "accepted": true,
  "model": {
    "endpoint": "http://localhost:11434/api/embeddings",
    "name": "nomic-embed-text",
    "api_key": "...",
    "dimensions": 768
  }
}
```

## HTTP API

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/register` | Register with orchestrator |
| POST | `/index` | Index file content |
| DELETE | `/source/{path}` | Remove file's chunks |
| POST | `/search` | Start search, return first result + cursor |
| POST | `/next` | Continue search with cursor |
| GET | `/health` | Health check |

### POST /index

Index content from a source file.

```json
// Request
{
  "source": "notes/api.md",
  "content": "# API Documentation\n\n## Rate Limiting\n\n..."
}

// Response 200
{
  "indexed": true,
  "chunks": 5
}
```

**Behavior:**
1. Hash content, check if changed from stored hash
2. If changed or new: delete existing chunks for source
3. Chunk content (header-aware markdown splitting)
4. Generate embeddings for each chunk
5. Store chunks + vectors in sqlite-vec

### DELETE /source/{path}

Remove all chunks for a source file.

```
DELETE /source/notes%2Fapi.md
```

```json
// Response 200
{
  "deleted": true,
  "chunks": 5
}
```

### POST /search

Start a search, return first result and cursor.

```json
// Request
{
  "query": "rate limiting"
}

// Response 200
{
  "cursor": "emb_abc123",
  "result": {
    "id": "0000000000123456-1a2b3c4d5e6f7890",
    "content": "## Rate Limiting\n\nThe API enforces rate limits...",
    "source": "notes/api.md:15-42",
    "score": 0.92
  },
  "remaining": 12
}
```

**Behavior:**
1. Generate embedding for query
2. Vector search in sqlite-vec, get top-k results
3. Create cursor with query vector and offset
4. Return first result

### POST /next

Continue search with cursor.

```json
// Request
{
  "cursor": "emb_abc123"
}

// Response 200 (more results)
{
  "cursor": "emb_abc123",
  "result": {
    "id": "...",
    "content": "...",
    "source": "notes/patterns.md:5-20",
    "score": 0.87
  },
  "remaining": 11
}

// Response 200 (no more results)
{
  "cursor": "emb_abc123",
  "result": null,
  "remaining": 0
}
```

**Cursor expiration:** Cursors expire after 5 minutes of inactivity. Expired cursor returns 404.

### GET /health

```json
{
  "status": "ok",
  "sources": 15,
  "chunks": 127
}
```

## Types

### Chunk

```rust
pub struct Chunk {
    pub id: String,              // snowflake ID (SnowflakeType::Embedding)
    pub source_path: String,     // "notes/api.md"
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
    pub embedding: Vec<f32>,
}
```

### Search Types

```rust
pub struct SearchResult {
    pub id: String,
    pub content: String,
    pub source: String,          // "notes/api.md:15-42"
    pub score: f32,
}

pub struct SearchResponse {
    pub cursor: String,
    pub result: Option<SearchResult>,
    pub remaining: usize,
}

pub struct Cursor {
    pub id: String,              // random hex string (e.g., "emb_a1b2c3d4")
    pub query_embedding: Vec<f32>,
    pub offset: usize,
    pub expires_at: Instant,
}
```

## Storage

### sqlite-vec Schema

```sql
-- Source file tracking
CREATE TABLE sources (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Chunk text and metadata
CREATE TABLE chunks (
    id TEXT PRIMARY KEY,
    source_path TEXT NOT NULL,
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    text TEXT NOT NULL,
    FOREIGN KEY (source_path) REFERENCES sources(path) ON DELETE CASCADE
);

-- Vector storage (sqlite-vec)
-- Dimension is configured from model config, not hardcoded
CREATE VIRTUAL TABLE chunks_vec USING vec0(
    id TEXT PRIMARY KEY,
    embedding FLOAT[{dimensions}]  -- e.g., 768 for nomic-embed-text
);
```

The table is created at startup using the `dimensions` value from the model config. The 768 shown above is an example.

### Search Query

```sql
SELECT c.id, c.source_path, c.line_start, c.line_end, c.text,
       vec_distance_cosine(v.embedding, ?) AS distance
FROM chunks_vec v
JOIN chunks c ON c.id = v.id
ORDER BY distance ASC
LIMIT ?
OFFSET ?
```

## Chunking

Header-aware markdown chunking:

```rust
pub struct ChunkConfig {
    pub max_tokens: usize,       // ~400 tokens per chunk
    pub overlap_lines: usize,    // lines of overlap between chunks
}
```

**Algorithm:**
1. Split on top-level headers (`#`, `##`)
2. If section > max_tokens, split on paragraphs (`\n\n`)
3. If paragraph > max_tokens, split on sentences
4. Track line numbers for each chunk
5. Add overlap from previous chunk's end

**Chunk source format:** `{path}:{line_start}-{line_end}`

Example: `notes/api.md:15-42`

## Embedding Client

Calls external embedding service:

```rust
pub struct EmbedClient {
    client: reqwest::Client,
    config: EmbedModelConfig,
}

impl EmbedClient {
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}
```

**Request format:**

Supports Ollama and OpenAI-compatible endpoints:

*Ollama:*
```json
POST {endpoint}
{
  "model": "nomic-embed-text",
  "prompt": "text to embed"
}
// Response: { "embedding": [0.1, 0.2, ...] }
```

*OpenAI-compatible:*
```json
POST {endpoint}
{
  "model": "text-embedding-3-small",
  "input": "text to embed"
}
// Response: { "data": [{ "embedding": [0.1, 0.2, ...] }] }
```

The embed client auto-detects format based on response structure. For batch embedding, multiple requests are made concurrently.

## Worker Integration

### Tools

Two new tools in the worker:

```rust
/// Start a search, returns first result + cursor
pub struct SearchEmbeddingsTool {
    pub query: String,
}

/// Continue search with cursor
pub struct NextEmbeddingTool {
    pub cursor: String,
}
```

### Write Hook

`WriteTool` notifies embed server when writing to `embeddings/`:

```rust
// In WriteTool execution
if path.starts_with("embeddings/") {
    let relative_path = path.strip_prefix("embeddings/").unwrap();
    embed_client.index(relative_path, &content).await?;
}
```

### Delete Hook

`DeleteTool` notifies embed server when deleting from `embeddings/`:

```rust
// In DeleteTool execution (new tool)
if path.starts_with("embeddings/") {
    let relative_path = path.strip_prefix("embeddings/").unwrap();
    embed_client.delete(relative_path).await?;
}
```

### Startup Scan

On worker startup, scan `workspace/embeddings/` and push all files:

```rust
async fn sync_embeddings_on_startup(workspace: &Path, embed_client: &EmbedClient) {
    let embed_dir = workspace.join("embeddings");
    for entry in walkdir::WalkDir::new(&embed_dir) {
        if entry.path().extension() == Some("md") {
            let content = fs::read_to_string(entry.path())?;
            let relative = entry.path().strip_prefix(&embed_dir)?;
            embed_client.index(relative.to_str()?, &content).await?;
        }
    }
}
```

## Error Handling

**Index errors:**
- Embedding model unreachable → return 503, content not indexed
- Invalid content (empty) → return 400

**Search errors:**
- Embedding model unreachable → return 503
- Invalid cursor → return 404
- Expired cursor → return 404 with message

**Storage errors:**
- Database locked → retry with backoff
- Corruption → return 500, log error

## Related Documents

- `docs/research/embedding-architecture.md` — Original research
- `docs/superpowers/specs/2026-04-01-worker-design.md` — Worker tools
- `docs/superpowers/specs/2026-04-01-orchestrator-design.md` — Orchestrator config
- `docs/superpowers/specs/2026-04-01-snowflake-server-design.md` — Snowflake IDs

# Phase 0: Embeddings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the embedding infrastructure to ollama's nomic-embed-text, sync files from `embeddings/` on write events, and give the agent a `search` tool.

**Architecture:** `EmbeddingClient` (already built) is wired into `SyncService` (refactored from mock). `SyncService` runs as a coordinator task, subscribing to `NoteWritten` events and running `full_sync()` at startup. A new `search` tool queries the `VectorStore`. Old dead memory tools are removed.

**Tech Stack:** Rust, reqwest (embedding API), rusqlite (VectorStore), tokio (async sync service)

**Key discovery:** The `NoteWritten` event is currently only triggered when `ToolResult::output_file` contains "embeddings/", but the `WriteTool` never sets `output_file`. The write tool must be updated to signal when it writes to embeddings/.

---

### Task 1: Fix NoteWritten trigger for write tool

**Files:**
- Modify: `crates/river-gateway/src/tools/file.rs`

The `WriteTool` currently returns `ToolResult::success()` with no `output_file`. The `NoteWritten` event in `agent/task.rs` checks `r.output_file` for paths containing "embeddings/". Without `output_file`, writes to `embeddings/` never trigger sync.

- [ ] **Step 1: Update WriteTool to set output_file when writing**

In `crates/river-gateway/src/tools/file.rs`, change the WriteTool's `execute` return from:

```rust
Ok(ToolResult::success(format!(
    "Wrote {} bytes to {:?}",
    content.len(),
    path
)))
```

to:

```rust
Ok(ToolResult::with_file(
    format!("Wrote {} bytes to {:?}", content.len(), path),
    path.to_string_lossy().to_string(),
))
```

This sets `output_file` on every write, and the existing check in `agent/task.rs` (line ~445) will fire `NoteWritten` when the path contains "embeddings/".

- [ ] **Step 2: Do the same for EditTool**

Find the EditTool's success return and update it similarly:

```rust
Ok(ToolResult::with_file(
    format!("Edited {:?}: replaced {} bytes", path, new_string.len()),
    path.to_string_lossy().to_string(),
))
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway -- tools::file`

Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "fix: WriteTool and EditTool set output_file for NoteWritten trigger"
```

---

### Task 2: Refactor SyncService to use EmbeddingClient

**Files:**
- Modify: `crates/river-gateway/src/embeddings/sync.rs`

- [ ] **Step 1: Remove mock and add EmbeddingClient field**

In `crates/river-gateway/src/embeddings/sync.rs`:

Remove the mock function:
```rust
/// Mock embedding function - will be replaced with real embedding client
async fn embed_text(_content: &str) -> Result<Vec<f32>, String> {
    Ok(vec![0.1, 0.2, 0.3, 0.4])
}
```

Add import and field:
```rust
use crate::memory::EmbeddingClient;
```

Update the struct and constructor:
```rust
pub struct SyncService {
    embeddings_dir: PathBuf,
    store: VectorStore,
    chunker: Chunker,
    embedding_client: EmbeddingClient,
}

impl SyncService {
    pub fn new(embeddings_dir: PathBuf, store: VectorStore, embedding_client: EmbeddingClient) -> Self {
        Self {
            embeddings_dir,
            store,
            chunker: Chunker::default(),
            embedding_client,
        }
    }
```

- [ ] **Step 2: Replace mock call in sync_file**

Change the embedding call in `sync_file` from:
```rust
let embedding = embed_text(&chunk.content).await?;
```
to:
```rust
let embedding = self.embedding_client.embed(&chunk.content).await
    .map_err(|e| format!("Embedding failed: {}", e))?;
```

- [ ] **Step 3: Add orphan pruning to full_sync**

Add a method to `VectorStore` to list all source paths, then add pruning to `full_sync`:

In `crates/river-gateway/src/embeddings/store.rs`, add:

```rust
/// List all unique source paths in the store
pub fn list_sources(&self) -> Result<Vec<String>, String> {
    let conn = self.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT DISTINCT source_path FROM chunks")
        .map_err(|e| e.to_string())?;
    let sources = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(sources)
}

/// Get total chunk count
pub fn chunk_count(&self) -> Result<usize, String> {
    let conn = self.conn.lock().map_err(|e| e.to_string())?;
    let count: usize = conn
        .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    Ok(count)
}
```

Then update `full_sync` in `sync.rs`:

```rust
pub async fn full_sync(&self) -> Result<SyncStats, String> {
    let mut stats = SyncStats::default();

    // Prune orphaned chunks (files that no longer exist)
    if let Ok(sources) = self.store.list_sources() {
        for source in sources {
            let full_path = self.embeddings_dir.join(&source);
            if !full_path.exists() {
                self.store.delete_source(&source)?;
                stats.pruned += 1;
                tracing::info!(source = %source, "Pruned orphaned chunks");
            }
        }
    }

    // Sync existing files
    let files = self.list_markdown_files()?;
    for path in files {
        match self.sync_file(&path).await {
            Ok(changed) => {
                if changed {
                    stats.updated += 1;
                } else {
                    stats.skipped += 1;
                }
            }
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "Failed to sync file");
                stats.errors += 1;
            }
        }
    }

    // Log corpus size warning
    if let Ok(count) = self.store.chunk_count() {
        if count > 1000 {
            tracing::warn!(chunks = count, "Corpus size exceeds recommended limit for brute-force search");
        } else {
            tracing::info!(chunks = count, "Sync complete");
        }
    }

    Ok(stats)
}
```

Add `pruned` to `SyncStats`:

```rust
#[derive(Debug, Default)]
pub struct SyncStats {
    pub updated: usize,
    pub skipped: usize,
    pub errors: usize,
    pub pruned: usize,
}
```

- [ ] **Step 4: Make SyncService generic over an Embedder trait**

Define an `Embedder` trait so tests can use a mock:

In `crates/river-gateway/src/embeddings/sync.rs`, add:

```rust
/// Trait for embedding text — allows mocking in tests
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}

#[async_trait::async_trait]
impl Embedder for EmbeddingClient {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        EmbeddingClient::embed(self, text)
            .await
            .map_err(|e| e.to_string())
    }
}
```

Make `SyncService` generic:

```rust
pub struct SyncService<E: Embedder> {
    embeddings_dir: PathBuf,
    store: VectorStore,
    chunker: Chunker,
    embedder: E,
}

impl<E: Embedder> SyncService<E> {
    pub fn new(embeddings_dir: PathBuf, store: VectorStore, embedder: E) -> Self {
        Self {
            embeddings_dir,
            store,
            chunker: Chunker::default(),
            embedder,
        }
    }
    // ... rest of impl unchanged, use self.embedder.embed() instead of self.embedding_client.embed()
}
```

Add `async-trait` to the gateway's `Cargo.toml` if not already present.

- [ ] **Step 4b: Create MockEmbedder for tests**

In the test module:

```rust
struct MockEmbedder;

#[async_trait::async_trait]
impl Embedder for MockEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, String> {
        Ok(vec![0.1, 0.2, 0.3, 0.4])
    }
}
```

Update existing tests to use `MockEmbedder`:

```rust
#[tokio::test]
async fn test_sync_file() {
    let temp = TempDir::new().unwrap();
    // ... existing setup ...
    let store = VectorStore::open_in_memory().unwrap();
    let sync = SyncService::new(temp.path().to_path_buf(), store, MockEmbedder);
    // ... rest unchanged
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-gateway -- embeddings`

Expected: Ignored tests skip, others pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: wire EmbeddingClient into SyncService, add orphan pruning and chunk_count"
```

---

### Task 3: SyncService event loop

**Files:**
- Modify: `crates/river-gateway/src/embeddings/sync.rs`
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Add run() method to SyncService**

Add the event loop to `SyncService`:

```rust
use crate::coordinator::events::{AgentEvent, CoordinatorEvent};
use tokio::sync::broadcast;

impl SyncService {
    /// Run the sync service: full sync at startup, then listen for NoteWritten events
    pub async fn run(self, mut event_rx: broadcast::Receiver<CoordinatorEvent>) {
        // Initial full sync
        match self.full_sync().await {
            Ok(stats) => {
                tracing::info!(
                    updated = stats.updated,
                    skipped = stats.skipped,
                    pruned = stats.pruned,
                    errors = stats.errors,
                    "Initial embedding sync complete"
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "Initial embedding sync failed");
            }
        }

        // Event loop
        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(AgentEvent::NoteWritten { path, .. })) => {
                    let file_path = std::path::PathBuf::from(&path);
                    if file_path.exists() {
                        match self.sync_file(&file_path).await {
                            Ok(true) => tracing::info!(path = %path, "Synced file on write event"),
                            Ok(false) => tracing::debug!(path = %path, "File unchanged"),
                            Err(e) => tracing::error!(path = %path, error = %e, "Failed to sync file on write event"),
                        }
                    }
                }
                Ok(CoordinatorEvent::Shutdown) => {
                    tracing::info!("Sync service shutting down");
                    break;
                }
                Ok(_) => {} // Ignore other events
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "Sync service lagged, running full sync");
                    let _ = self.full_sync().await;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("Event bus closed, sync service stopping");
                    break;
                }
            }
        }
    }
}
```

- [ ] **Step 2: Open VectorStore and register SearchTool early in server.rs**

In `server.rs`, the VectorStore must be opened and the SearchTool registered BEFORE `AppState` consumes the registry. The SyncService is spawned AFTER the coordinator is created.

Early in server.rs (where the one-shot sync currently lives, ~line 91), open the VectorStore and store it for later:

```rust
// Open vector store if embeddings are configured
let vector_store = if config.embedding_url.is_some() {
    let vectors_db_path = config.data_dir.join("vectors.db");
    match VectorStore::open(&vectors_db_path) {
        Ok(store) => {
            tracing::info!("Opened vector store at {:?}", vectors_db_path);
            Some(store)
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to open vector store");
            None
        }
    }
} else {
    None
};
```

Then register the SearchTool alongside other tools (~line 201), before the registry is locked:

```rust
// Register search tool if embeddings are configured
if let (Some(store), Some(ref embed_client)) = (&vector_store, &embedding_client) {
    registry.register(Box::new(SearchTool::new(
        store.clone(),
        Arc::new(embed_client.clone()),
    )));
    tracing::info!("Registered search tool");
}
```

Remove the old one-shot sync code.

- [ ] **Step 2b: Spawn SyncService after coordinator is created**

After the coordinator is created and agent/spectator tasks are spawned (~line 422), spawn the sync service:

```rust
// Spawn embedding sync service
if let (Some(store), Some(ref embed_client)) = (vector_store, &embedding_client) {
    let sync_service = SyncService::new(
        embeddings_dir.clone(),
        store,
        embed_client.clone(),
    );
    let sync_rx = coordinator.bus().subscribe();
    coordinator.spawn_task("sync", move |_| async move {
        sync_service.run(sync_rx).await;
    });
    tracing::info!("Spawned embedding sync service");
}
```

- [ ] **Step 3: Build and verify**

Run: `cargo build -p river-gateway`

Expected: Compiles.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: SyncService event loop — subscribes to NoteWritten, spawned via coordinator"
```

---

### Task 4: Search tool

**Files:**
- Create: `crates/river-gateway/src/tools/search.rs`
- Modify: `crates/river-gateway/src/tools/mod.rs`
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Create search tool**

Create `crates/river-gateway/src/tools/search.rs`:

```rust
//! Semantic search tool — searches embeddings via VectorStore

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::registry::{Tool, ToolResult};
use crate::embeddings::VectorStore;
use crate::memory::EmbeddingClient;

pub struct SearchTool {
    store: VectorStore,
    embedding_client: Arc<EmbeddingClient>,
}

impl SearchTool {
    pub fn new(store: VectorStore, embedding_client: Arc<EmbeddingClient>) -> Self {
        Self {
            store,
            embedding_client,
        }
    }
}

impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Semantic search over embedded files. Finds content similar in meaning to the query, unlike grep which matches exact text."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default: 5)" }
            },
            "required": ["query"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: query"))?;

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        // Embed the query
        let embedding = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.embedding_client.embed(query))
        })?;

        // Search
        let results = self
            .store
            .search(&embedding, limit)
            .map_err(|e| RiverError::tool(format!("Search failed: {}", e)))?;

        if results.is_empty() {
            return Ok(ToolResult::success("No results found."));
        }

        let mut output = format!("Found {} results for \"{}\":\n\n", results.len(), query);
        for (i, result) in results.iter().enumerate() {
            let snippet: String = result.content.chars().take(200).collect();
            output.push_str(&format!(
                "{}. [{:.2}] {}\n   {}\n\n",
                i + 1,
                result.similarity,
                result.source_path,
                snippet,
            ));
        }

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_tool_schema() {
        let store = VectorStore::open_in_memory().unwrap();
        let client = Arc::new(EmbeddingClient::new(
            crate::memory::EmbeddingConfig::default(),
        ));
        let tool = SearchTool::new(store, client);
        assert_eq!(tool.name(), "search");
        let params = tool.parameters();
        assert!(params["properties"]["query"].is_object());
        assert!(params["properties"]["limit"].is_object());
    }
}
```

- [ ] **Step 2: Register in mod.rs**

Add to `crates/river-gateway/src/tools/mod.rs`:

```rust
pub mod search;
```

And update the re-exports to include `SearchTool`:

```rust
pub use search::SearchTool;
```

- [ ] **Step 3: Register search tool in server.rs**

The SearchTool registration was already handled in Task 3 Step 2 (registered early alongside other tools before the registry is locked). Verify it compiles and the tool appears in the tool list.

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway -- tools::search`

Expected: Schema test passes.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add search tool — semantic search over VectorStore embeddings"
```

---

### Task 5: Remove dead memory tools

**Files:**
- Delete: `crates/river-gateway/src/tools/memory.rs`
- Modify: `crates/river-gateway/src/tools/mod.rs`
- Delete: `crates/river-gateway/src/memory/search.rs`
- Modify: `crates/river-gateway/src/memory/mod.rs`
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Delete tools/memory.rs**

```bash
rm crates/river-gateway/src/tools/memory.rs
```

- [ ] **Step 2: Remove from tools/mod.rs**

Remove `pub mod memory;` and any re-exports of `EmbedTool`, `MemorySearchTool`, etc.

- [ ] **Step 3: Delete memory/search.rs**

```bash
rm crates/river-gateway/src/memory/search.rs
```

- [ ] **Step 4: Update memory/mod.rs**

Change from:
```rust
mod embedding;
mod search;

pub use embedding::{EmbeddingClient, EmbeddingConfig};
pub use search::{MemorySearcher, SearchResult};
```

to:

```rust
mod embedding;

pub use embedding::{EmbeddingClient, EmbeddingConfig};
```

- [ ] **Step 5: Remove any remaining references in server.rs**

Search for and remove any commented-out memory tool registration or references to the old tools. The tools were already disabled but there may be commented code or imports.

- [ ] **Step 6: Build and test**

Run: `cargo build -p river-gateway && cargo test -p river-gateway`

Expected: Compiles and all tests pass. Some tests in the deleted files are gone — that's expected.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "cleanup: remove dead memory tools (EmbedTool, MemorySearchTool, MemorySearcher)"
```

---

### Task 6: Update AGENTS.md

**Files:**
- Modify: `workspace/AGENTS.md`

- [ ] **Step 1: Add search tool documentation**

Add a section to `workspace/AGENTS.md` after the Tools section:

```markdown
## Search

You have a `search` tool for semantic search over files in your `embeddings/` directory. Unlike `grep` which matches exact text, `search` finds content that is similar in meaning to your query.

- `grep` — exact text matching, fast, works on any file in the workspace
- `search` — semantic similarity, finds related content even with different wording, only searches files in `embeddings/`

Files you write to `embeddings/` are automatically indexed. Use `search` when you're looking for concepts or topics rather than specific strings.
```

- [ ] **Step 2: Commit**

```bash
git add -A && git commit -m "docs: add search tool to AGENTS.md — semantic vs text search"
```

---

### Task 7: Full workspace build and test

**Files:** None (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build`

Expected: Clean build.

- [ ] **Step 2: Full test suite**

Run: `cargo test`

Expected: All tests pass.

- [ ] **Step 3: Push**

```bash
git push
```

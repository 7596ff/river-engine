# river-embed Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix critical performance and correctness issues in river-embed including sqlite-vec integration for O(log n) vector search, async-safe locking, cursor offset bug, foreign key enforcement, chunking algorithm completion, and operational improvements.

**Architecture:** The river-embed service is an HTTP server providing vector search capabilities. It stores document chunks with embeddings in SQLite, exposes `/index`, `/search`, `/next`, and `/health` endpoints via Axum, and uses an external embedding service for generating vectors. The fixes will replace the O(n) full-table-scan search with sqlite-vec virtual table queries, convert std::sync::Mutex to tokio::sync::Mutex for async safety, fix the cursor pagination bug, and add proper chunking/cleanup/timeout handling.

**Tech Stack:** Rust, Axum (HTTP), rusqlite + sqlite-vec (vector storage), tokio (async runtime), reqwest (HTTP client)

---

## File Structure

```
crates/river-embed/
  Cargo.toml          # Add sqlite-vec dependency
  src/
    main.rs           # Add graceful shutdown, cursor cleanup task
    http.rs           # Switch to tokio::sync::Mutex, extract index logic, configurable search limit
    store.rs          # sqlite-vec integration, foreign keys pragma
    search.rs         # Fix cursor offset bug, add cleanup_expired(), remove dead code
    chunk.rs          # Add paragraph and sentence splitting
    embed.rs          # Add request timeout
    index.rs          # Move indexing logic here from http.rs
    config.rs         # (unchanged)
```

---

## Task 1: Add sqlite-vec Dependency

**File:** `/home/cassie/river-engine/crates/river-embed/Cargo.toml`

- [ ] **Step 1.1:** Add sqlite-vec dependency

In `Cargo.toml`, add the sqlite-vec dependency under `[dependencies]`:

```toml
# Vector search
sqlite-vec = "0.1"
```

**Commit:** `feat(river-embed): add sqlite-vec dependency for vector search`

---

## Task 2: Enable Foreign Keys and Integrate sqlite-vec in Store

**File:** `/home/cassie/river-engine/crates/river-embed/src/store.rs`

- [ ] **Step 2.1:** Add sqlite-vec import and enable foreign keys pragma

At the top of `store.rs`, add the import for sqlite-vec. In the `open` function, add the foreign keys pragma after opening the connection:

```rust
use rusqlite::{params, Connection, LoadExtensionGuard};
use sqlite_vec::sqlite3_vec_init;
use std::path::Path;
use zerocopy::IntoBytes;

// ... existing error types ...

impl Store {
    /// Open or create the database.
    pub fn open(path: impl AsRef<Path>, dimensions: usize) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;

        // Enable foreign keys
        conn.execute("PRAGMA foreign_keys = ON", [])?;

        // Load sqlite-vec extension
        unsafe {
            let _guard: LoadExtensionGuard = conn.load_extension_enable()?;
            sqlite_vec::load(&conn)?;
        }

        let store = Self { conn, dimensions };
        store.init_schema()?;
        Ok(store)
    }
```

- [ ] **Step 2.2:** Create the virtual table in init_schema

Update `init_schema` to create the sqlite-vec virtual table after the regular tables:

```rust
    fn init_schema(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sources (
                path TEXT PRIMARY KEY,
                hash TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                text TEXT NOT NULL,
                embedding BLOB NOT NULL,
                FOREIGN KEY (source_path) REFERENCES sources(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_chunks_source ON chunks(source_path);
            "#,
        )?;

        // Create virtual table for vector search
        let create_vec_table = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_vec USING vec0(id TEXT PRIMARY KEY, embedding FLOAT[{}])",
            self.dimensions
        );
        self.conn.execute(&create_vec_table, [])?;

        Ok(())
    }
```

- [ ] **Step 2.3:** Update insert_chunk to also insert into the virtual table

Modify `insert_chunk` to insert into both the regular `chunks` table and the `chunks_vec` virtual table:

```rust
    /// Insert a chunk with its embedding.
    pub fn insert_chunk(
        &self,
        id: &str,
        source_path: &str,
        line_start: usize,
        line_end: usize,
        text: &str,
        embedding: &[f32],
    ) -> Result<(), StoreError> {
        if embedding.len() != self.dimensions {
            return Err(StoreError::DimensionMismatch {
                expected: self.dimensions,
                actual: embedding.len(),
            });
        }

        let embedding_bytes = embedding.as_bytes();

        // Insert into main chunks table
        self.conn.execute(
            "INSERT INTO chunks (id, source_path, line_start, line_end, text, embedding) VALUES (?, ?, ?, ?, ?, ?)",
            params![id, source_path, line_start as i64, line_end as i64, text, embedding_bytes],
        )?;

        // Insert into vector table
        self.conn.execute(
            "INSERT INTO chunks_vec (id, embedding) VALUES (?, ?)",
            params![id, embedding_bytes],
        )?;

        Ok(())
    }
```

- [ ] **Step 2.4:** Update delete_source to also delete from virtual table

Modify `delete_source` to also clean up the virtual table:

```rust
    /// Delete all chunks for a source.
    pub fn delete_source(&self, path: &str) -> Result<usize, StoreError> {
        // Get chunk IDs first (for deleting from vec table)
        let mut stmt = self.conn.prepare("SELECT id FROM chunks WHERE source_path = ?")?;
        let ids: Vec<String> = stmt
            .query_map([path], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Delete from vector table
        for id in &ids {
            self.conn.execute("DELETE FROM chunks_vec WHERE id = ?", [id])?;
        }

        // Delete chunks (cascade will handle this if foreign keys work, but be explicit)
        self.conn
            .execute("DELETE FROM chunks WHERE source_path = ?", [path])?;

        // Delete source
        self.conn
            .execute("DELETE FROM sources WHERE path = ?", [path])?;

        Ok(ids.len())
    }
```

- [ ] **Step 2.5:** Rewrite search to use sqlite-vec

Replace the O(n) search with sqlite-vec KNN query:

```rust
    /// Search for similar chunks using sqlite-vec.
    pub fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SearchHit>, StoreError> {
        if query_embedding.len() != self.dimensions {
            return Err(StoreError::DimensionMismatch {
                expected: self.dimensions,
                actual: query_embedding.len(),
            });
        }

        let query_bytes = query_embedding.as_bytes();

        // Use sqlite-vec for KNN search with offset
        // We fetch limit + offset results and skip the first offset
        let fetch_count = limit + offset;

        let mut stmt = self.conn.prepare(
            r#"
            SELECT v.id, v.distance, c.source_path, c.line_start, c.line_end, c.text
            FROM chunks_vec v
            JOIN chunks c ON v.id = c.id
            WHERE v.embedding MATCH ?
            ORDER BY v.distance
            LIMIT ?
            "#,
        )?;

        let hits: Vec<SearchHit> = stmt
            .query_map(params![query_bytes, fetch_count as i64], |row| {
                Ok(SearchHit {
                    id: row.get(0)?,
                    distance: row.get(1)?,
                    source_path: row.get(2)?,
                    line_start: row.get::<_, i64>(3)? as usize,
                    line_end: row.get::<_, i64>(4)? as usize,
                    text: row.get(5)?,
                })
            })?
            .filter_map(|r| r.ok())
            .skip(offset)
            .take(limit)
            .collect();

        Ok(hits)
    }
```

- [ ] **Step 2.6:** Remove the helper functions that are no longer needed

Delete the `bytes_to_floats` and `cosine_distance` functions at the bottom of the file as they are no longer used.

**Commit:** `feat(river-embed): integrate sqlite-vec for O(log n) vector search`

---

## Task 3: Fix Cursor Offset Bug

**File:** `/home/cassie/river-engine/crates/river-embed/src/search.rs`

- [ ] **Step 3.1:** Fix cursor creation to start at offset 1

The bug is that `/search` returns the first result (offset 0), but creates a cursor with `offset: 0`. When `/next` is called, it returns offset 0 again (duplicate). Fix by initializing cursor with `offset: 1`:

```rust
    /// Create a new cursor.
    pub fn create(&self, query_embedding: Vec<f32>, total_results: usize) -> String {
        let id = generate_cursor_id();
        let cursor = Cursor {
            id: id.clone(),
            query_embedding,
            offset: 1,  // Start at 1 since /search already returned offset 0
            total_results,
            expires_at: Instant::now() + self.ttl,
        };

        let mut cursors = self.cursors.write().unwrap();
        cursors.insert(id.clone(), cursor);
        id
    }
```

- [ ] **Step 3.2:** Remove dead code (Cursor.id field)

The `id` field in `Cursor` struct is set but never read. Remove it:

```rust
/// Internal cursor state.
pub struct Cursor {
    pub query_embedding: Vec<f32>,
    pub offset: usize,
    pub total_results: usize,
    pub expires_at: Instant,
}
```

Update `create` method to not set the id field:

```rust
    /// Create a new cursor.
    pub fn create(&self, query_embedding: Vec<f32>, total_results: usize) -> String {
        let id = generate_cursor_id();
        let cursor = Cursor {
            query_embedding,
            offset: 1,  // Start at 1 since /search already returned offset 0
            total_results,
            expires_at: Instant::now() + self.ttl,
        };

        let mut cursors = self.cursors.write().unwrap();
        cursors.insert(id.clone(), cursor);
        id
    }
```

- [ ] **Step 3.3:** Add cleanup_expired method for background cleanup task

Add a public method to clean up expired cursors:

```rust
    /// Remove all expired cursors.
    pub fn cleanup_expired(&self) {
        let mut cursors = self.cursors.write().unwrap();
        let now = Instant::now();
        cursors.retain(|_, cursor| cursor.expires_at > now);
    }
```

- [ ] **Step 3.4:** Write test for cursor offset fix

Add a test at the bottom of search.rs:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_starts_at_offset_1() {
        let manager = CursorManager::new(Duration::from_secs(60));
        let embedding = vec![1.0, 2.0, 3.0];

        let cursor_id = manager.create(embedding.clone(), 10);

        // First advance should return offset 1, not 0
        let (_, offset, remaining) = manager.advance(&cursor_id).unwrap();
        assert_eq!(offset, 1, "First /next call should return offset 1");
        assert_eq!(remaining, 8, "After returning offset 1, 8 results remain (2-9)");

        // Second advance should return offset 2
        let (_, offset, _) = manager.advance(&cursor_id).unwrap();
        assert_eq!(offset, 2);
    }

    #[test]
    fn test_cleanup_expired() {
        let manager = CursorManager::new(Duration::from_millis(1));
        let embedding = vec![1.0, 2.0, 3.0];

        let cursor_id = manager.create(embedding.clone(), 10);

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(10));

        // Cleanup
        manager.cleanup_expired();

        // Cursor should be gone
        assert!(manager.advance(&cursor_id).is_none());
    }
}
```

**Commit:** `fix(river-embed): cursor offset bug and add cleanup method`

---

## Task 4: Switch to tokio::sync::Mutex

**File:** `/home/cassie/river-engine/crates/river-embed/src/http.rs`

- [ ] **Step 4.1:** Update imports and AppState to use tokio::sync::Mutex

Replace the std::sync::Mutex with tokio::sync::Mutex:

```rust
//! HTTP server and endpoints.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use river_snowflake::{AgentBirth, GeneratorCache};

use crate::embed::EmbedClient;
use crate::index::IndexError;
use crate::search::{hit_to_result, CursorManager, SearchResponse};
use crate::store::Store;

/// Shared application state.
pub struct AppState {
    pub store: Mutex<Store>,
    pub embed_client: EmbedClient,
    pub cursor_manager: CursorManager,
    pub id_cache: GeneratorCache,
    pub birth: AgentBirth,
}
```

- [ ] **Step 4.2:** Update handle_index to use async lock

Change all `.lock().unwrap()` calls to `.lock().await`:

```rust
async fn handle_index(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IndexRequest>,
) -> impl IntoResponse {
    let result = crate::index::index_content(&state, &req.source, &req.content, state.birth).await;

    match result {
        Ok((indexed, chunks)) => (
            StatusCode::OK,
            Json(IndexResponse { indexed, chunks }),
        )
            .into_response(),
        Err(IndexError::EmptyContent) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "empty content".into(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

- [ ] **Step 4.3:** Update handle_delete to use async lock

```rust
async fn handle_delete(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let store = state.store.lock().await;

    match store.delete_source(&path) {
        Ok(count) => (
            StatusCode::OK,
            Json(DeleteResponse {
                deleted: true,
                chunks: count,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

- [ ] **Step 4.4:** Add configurable search limit constant

Add a constant at the top of http.rs and use it in handle_search:

```rust
/// Maximum number of results to fetch per search.
const MAX_SEARCH_RESULTS: usize = 100;
```

- [ ] **Step 4.5:** Update handle_search to use async lock

```rust
async fn handle_search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    // Generate query embedding
    let query_embedding = match state.embed_client.embed_one(&req.query).await {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    };

    // Search store
    let hits = {
        let store = state.store.lock().await;
        match store.search(&query_embedding, MAX_SEARCH_RESULTS, 0) {
            Ok(h) => h,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response()
            }
        }
    };

    let total = hits.len();
    let first_result = hits.into_iter().next().map(|h| {
        hit_to_result(
            h.id,
            h.source_path,
            h.line_start,
            h.line_end,
            h.text,
            h.distance,
        )
    });

    // Create cursor
    let cursor = state.cursor_manager.create(query_embedding, total);

    (
        StatusCode::OK,
        Json(SearchResponse {
            cursor,
            result: first_result,
            remaining: total.saturating_sub(1),
        }),
    )
        .into_response()
}
```

- [ ] **Step 4.6:** Update handle_next to use async lock

```rust
async fn handle_next(
    State(state): State<Arc<AppState>>,
    Json(req): Json<NextRequest>,
) -> impl IntoResponse {
    let Some((query_embedding, offset, remaining)) = state.cursor_manager.advance(&req.cursor)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "cursor not found or expired".into(),
            }),
        )
            .into_response();
    };

    let hits = {
        let store = state.store.lock().await;
        match store.search(&query_embedding, 1, offset) {
            Ok(h) => h,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response()
            }
        }
    };

    let result = hits.into_iter().next().map(|h| {
        hit_to_result(
            h.id,
            h.source_path,
            h.line_start,
            h.line_end,
            h.text,
            h.distance,
        )
    });

    (
        StatusCode::OK,
        Json(SearchResponse {
            cursor: req.cursor,
            result,
            remaining,
        }),
    )
        .into_response()
}
```

- [ ] **Step 4.7:** Update handle_health to use async lock

```rust
async fn handle_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let store = state.store.lock().await;

    match store.counts() {
        Ok((sources, chunks)) => Json(HealthResponse {
            status: "ok".into(),
            sources,
            chunks,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
```

- [ ] **Step 4.8:** Remove index_content_async from http.rs (will be moved to index.rs)

Delete the entire `index_content_async` function from `http.rs`.

**Commit:** `refactor(river-embed): switch to tokio::sync::Mutex for async safety`

---

## Task 5: Move Indexing Logic to index.rs

**File:** `/home/cassie/river-engine/crates/river-embed/src/index.rs`

- [ ] **Step 5.1:** Add imports and move the indexing function

Rewrite `index.rs` to contain the actual indexing logic:

```rust
//! Indexing logic.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use river_snowflake::SnowflakeType;

use crate::chunk::{chunk_markdown, ChunkConfig};
use crate::embed::EmbedError;
use crate::http::AppState;
use crate::store::StoreError;

#[derive(Debug)]
pub enum IndexError {
    Embed(EmbedError),
    Store(StoreError),
    EmptyContent,
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Embed(e) => write!(f, "embedding error: {}", e),
            Self::Store(e) => write!(f, "store error: {}", e),
            Self::EmptyContent => write!(f, "empty content"),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<EmbedError> for IndexError {
    fn from(e: EmbedError) -> Self {
        Self::Embed(e)
    }
}

impl From<StoreError> for IndexError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// Index content into the vector store.
pub async fn index_content(
    state: &AppState,
    source: &str,
    content: &str,
    birth: river_snowflake::AgentBirth,
) -> Result<(bool, usize), IndexError> {
    if content.trim().is_empty() {
        return Err(IndexError::EmptyContent);
    }

    // Hash content
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    // Check if update needed
    let needs_update = {
        let store = state.store.lock().await;
        store.needs_update(source, &hash)?
    };

    if !needs_update {
        return Ok((false, 0));
    }

    // Delete existing chunks
    {
        let store = state.store.lock().await;
        store.delete_source(source)?;
    }

    // Chunk content
    let config = ChunkConfig::default();
    let text_chunks = chunk_markdown(content, &config);

    if text_chunks.is_empty() {
        return Ok((true, 0));
    }

    // Generate embeddings (async)
    let texts: Vec<String> = text_chunks.iter().map(|c| c.text.clone()).collect();
    let embeddings = state.embed_client.embed(&texts).await?;

    // Store source and chunks
    {
        let store = state.store.lock().await;
        store.upsert_source(source, &hash)?;

        for (chunk, embedding) in text_chunks.iter().zip(embeddings.iter()) {
            let id = state.id_cache.next_id(birth, SnowflakeType::Embedding);
            store.insert_chunk(
                &id.to_string(),
                source,
                chunk.line_start,
                chunk.line_end,
                &chunk.text,
                embedding,
            )?;
        }
    }

    Ok((true, text_chunks.len()))
}
```

**Commit:** `refactor(river-embed): move indexing logic to index.rs`

---

## Task 6: Complete Chunking Algorithm

**File:** `/home/cassie/river-engine/crates/river-embed/src/chunk.rs`

- [ ] **Step 6.1:** Add paragraph splitting

Update `chunk_markdown` to split on paragraph boundaries (`\n\n`) before resorting to token-based splitting:

```rust
//! Markdown-aware chunking.

/// Configuration for chunking.
pub struct ChunkConfig {
    /// Maximum tokens per chunk (~400 tokens).
    pub max_tokens: usize,
    /// Lines of overlap between chunks.
    pub overlap_lines: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_tokens: 400,
            overlap_lines: 2,
        }
    }
}

/// A chunk of text with source information.
#[derive(Debug, Clone)]
pub struct TextChunk {
    pub text: String,
    pub line_start: usize,
    pub line_end: usize,
}

/// Chunk markdown content into smaller pieces.
///
/// Uses 3-level chunking:
/// 1. Split on headers
/// 2. Split on paragraphs (\n\n)
/// 3. Split on sentences for oversized content
pub fn chunk_markdown(content: &str, config: &ChunkConfig) -> Vec<TextChunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return vec![];
    }

    let mut chunks = Vec::new();
    let mut current_chunk_lines: Vec<&str> = Vec::new();
    let mut current_start = 1; // 1-indexed
    let mut current_tokens = 0;

    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        let line_tokens = estimate_tokens(line);

        // Check if this is a header
        let is_header = line.starts_with('#');

        // Check if this is a paragraph break
        let is_paragraph_break = line.trim().is_empty()
            && !current_chunk_lines.is_empty()
            && current_tokens > config.max_tokens / 2;

        // If we hit a header or paragraph break and have content, consider starting new chunk
        if (is_header || is_paragraph_break) && !current_chunk_lines.is_empty() {
            // For paragraph breaks, only split if we have substantial content
            if is_header || current_tokens > config.max_tokens / 2 {
                chunks.push(TextChunk {
                    text: current_chunk_lines.join("\n"),
                    line_start: current_start,
                    line_end: line_num - 1,
                });

                // Add overlap from previous chunk
                let overlap_start = current_chunk_lines.len().saturating_sub(config.overlap_lines);
                current_chunk_lines = current_chunk_lines[overlap_start..].to_vec();
                current_start = line_num.saturating_sub(config.overlap_lines);
                current_tokens = current_chunk_lines.iter().map(|l| estimate_tokens(l)).sum();
            }
        }

        // Check if adding this line would exceed max tokens
        if current_tokens + line_tokens > config.max_tokens && !current_chunk_lines.is_empty() {
            // Try to split at sentence boundary first
            if let Some((before, after, split_line)) = try_sentence_split(&current_chunk_lines, config) {
                chunks.push(TextChunk {
                    text: before,
                    line_start: current_start,
                    line_end: current_start + split_line,
                });

                current_chunk_lines = vec![];
                for part in after.lines() {
                    current_chunk_lines.push(Box::leak(part.to_string().into_boxed_str()));
                }
                current_start = current_start + split_line + 1;
                current_tokens = estimate_tokens(&after);
            } else {
                // No good sentence boundary, just split at token limit
                chunks.push(TextChunk {
                    text: current_chunk_lines.join("\n"),
                    line_start: current_start,
                    line_end: line_num - 1,
                });

                // Add overlap
                let overlap_start = current_chunk_lines.len().saturating_sub(config.overlap_lines);
                current_chunk_lines = current_chunk_lines[overlap_start..].to_vec();
                current_start = line_num.saturating_sub(config.overlap_lines);
                current_tokens = current_chunk_lines.iter().map(|l| estimate_tokens(l)).sum();
            }
        }

        current_chunk_lines.push(line);
        current_tokens += line_tokens;
    }

    // Add final chunk
    if !current_chunk_lines.is_empty() {
        chunks.push(TextChunk {
            text: current_chunk_lines.join("\n"),
            line_start: current_start,
            line_end: lines.len(),
        });
    }

    chunks
}

/// Try to find a sentence boundary to split on.
/// Returns (text before split, text after split, line number of split).
fn try_sentence_split(lines: &[&str], config: &ChunkConfig) -> Option<(String, String, usize)> {
    let text = lines.join("\n");
    let target_tokens = config.max_tokens * 3 / 4; // Aim for 75% of max

    let mut current_tokens = 0;
    let mut best_split = None;

    // Look for sentence boundaries (. ! ?) followed by space or newline
    for (i, ch) in text.char_indices() {
        current_tokens = estimate_tokens(&text[..i]);

        if current_tokens >= target_tokens {
            // Look for sentence ending punctuation
            if (ch == '.' || ch == '!' || ch == '?') && current_tokens < config.max_tokens {
                // Check if followed by space/newline (not abbreviation)
                let rest = &text[i + ch.len_utf8()..];
                if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\n') {
                    best_split = Some(i + ch.len_utf8());
                }
            }
        }

        if current_tokens > config.max_tokens && best_split.is_some() {
            break;
        }
    }

    let split_pos = best_split?;
    let before = text[..split_pos].trim().to_string();
    let after = text[split_pos..].trim().to_string();

    // Count lines in before
    let lines_before = before.lines().count();

    Some((before, after, lines_before.saturating_sub(1)))
}

/// Estimate tokens in a string (rough approximation: ~4 chars per token).
fn estimate_tokens(s: &str) -> usize {
    (s.len() + 3) / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_simple() {
        let content = "# Header\n\nSome text here.\n\nMore text.";
        let config = ChunkConfig::default();
        let chunks = chunk_markdown(content, &config);

        assert!(!chunks.is_empty());
        assert!(chunks[0].text.contains("Header"));
    }

    #[test]
    fn test_chunk_preserves_lines() {
        let content = "Line 1\nLine 2\nLine 3";
        let config = ChunkConfig::default();
        let chunks = chunk_markdown(content, &config);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].line_start, 1);
        assert_eq!(chunks[0].line_end, 3);
    }

    #[test]
    fn test_chunk_splits_large_paragraphs() {
        // Create a large paragraph that exceeds max_tokens
        let large_text = "This is a sentence. ".repeat(100);
        let content = format!("# Header\n\n{}", large_text);
        let config = ChunkConfig {
            max_tokens: 50,
            overlap_lines: 1,
        };
        let chunks = chunk_markdown(&content, &config);

        // Should have multiple chunks
        assert!(chunks.len() > 1, "Large content should be split into multiple chunks");
    }

    #[test]
    fn test_chunk_splits_on_paragraphs() {
        let content = "# Header\n\nFirst paragraph with some text.\n\nSecond paragraph with more text.\n\nThird paragraph.";
        let config = ChunkConfig {
            max_tokens: 30,
            overlap_lines: 1,
        };
        let chunks = chunk_markdown(content, &config);

        // Should split on paragraph boundaries
        assert!(chunks.len() >= 2, "Should split on paragraph boundaries");
    }
}
```

**Commit:** `feat(river-embed): complete 3-level chunking algorithm`

---

## Task 7: Add Request Timeout to Embed Client

**File:** `/home/cassie/river-engine/crates/river-embed/src/embed.rs`

- [ ] **Step 7.1:** Add timeout configuration to EmbedClient

Update the `EmbedClient::new` to configure a timeout:

```rust
use std::time::Duration;

// ... existing code ...

impl EmbedClient {
    /// Create a new client with the given configuration.
    pub fn new(config: EmbedModelConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        Self { client, config }
    }
```

**Commit:** `fix(river-embed): add 30s timeout to embed client requests`

---

## Task 8: Add Cursor Cleanup Task and Graceful Shutdown

**File:** `/home/cassie/river-engine/crates/river-embed/src/main.rs`

- [ ] **Step 8.1:** Update imports for signal handling and background tasks

```rust
//! River Embed server binary.

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::Mutex;

mod chunk;
mod config;
mod embed;
mod http;
mod index;
mod search;
mod store;

use config::{EmbedServiceInfo, RegistrationRequest, RegistrationResponse};
use embed::EmbedClient;
use http::AppState;
use river_snowflake::{AgentBirth, GeneratorCache};
use search::CursorManager;
use store::Store;
```

- [ ] **Step 8.2:** Add shutdown signal handler function

Add a function to handle graceful shutdown:

```rust
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    eprintln!("Shutdown signal received");
}
```

- [ ] **Step 8.3:** Add cursor cleanup background task

In the main function, after creating the state and before starting the server, spawn the cleanup task:

```rust
    // Build state
    let state = Arc::new(AppState {
        store: Mutex::new(store),
        embed_client,
        cursor_manager: CursorManager::default(),
        id_cache: GeneratorCache::new(),
        birth,
    });

    // Spawn cursor cleanup task
    {
        let cursor_manager = state.cursor_manager.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                cursor_manager.cleanup_expired();
            }
        });
    }

    // Build router
    let app = http::router(state);

    eprintln!("Embed server listening on {}", local_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
```

- [ ] **Step 8.4:** Make CursorManager Clone

Update `search.rs` to derive Clone for CursorManager:

```rust
use std::sync::Arc;

/// Cursor manager with expiration.
#[derive(Clone)]
pub struct CursorManager {
    cursors: Arc<RwLock<HashMap<String, Cursor>>>,
    ttl: Duration,
}

impl CursorManager {
    /// Create a new cursor manager.
    pub fn new(ttl: Duration) -> Self {
        Self {
            cursors: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    // ... rest of methods unchanged ...
}
```

**Commit:** `feat(river-embed): add cursor cleanup task and graceful shutdown`

---

## Task 9: Add Store Tests

**File:** `/home/cassie/river-engine/crates/river-embed/src/store.rs`

- [ ] **Step 9.1:** Add test module with comprehensive tests

Add tests at the bottom of `store.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_db_path() -> String {
        format!("/tmp/river_embed_test_{}.db", rand::random::<u64>())
    }

    #[test]
    fn test_store_open_and_schema() {
        let path = temp_db_path();
        let store = Store::open(&path, 384).expect("Failed to open store");

        // Verify we can get counts
        let (sources, chunks) = store.counts().expect("Failed to get counts");
        assert_eq!(sources, 0);
        assert_eq!(chunks, 0);

        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_insert_and_search() {
        let path = temp_db_path();
        let store = Store::open(&path, 3).expect("Failed to open store");

        store.upsert_source("test.md", "hash123").expect("Failed to upsert source");

        let embedding = vec![1.0, 0.0, 0.0];
        store.insert_chunk("chunk1", "test.md", 1, 10, "Test content", &embedding)
            .expect("Failed to insert chunk");

        let query = vec![1.0, 0.0, 0.0];
        let results = store.search(&query, 10, 0).expect("Failed to search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "chunk1");
        assert_eq!(results[0].text, "Test content");

        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_foreign_key_cascade() {
        let path = temp_db_path();
        let store = Store::open(&path, 3).expect("Failed to open store");

        store.upsert_source("test.md", "hash123").expect("Failed to upsert source");

        let embedding = vec![1.0, 0.0, 0.0];
        store.insert_chunk("chunk1", "test.md", 1, 10, "Content 1", &embedding)
            .expect("Failed to insert chunk 1");
        store.insert_chunk("chunk2", "test.md", 11, 20, "Content 2", &embedding)
            .expect("Failed to insert chunk 2");

        let (_, chunks_before) = store.counts().expect("Failed to get counts");
        assert_eq!(chunks_before, 2);

        // Delete source - should cascade to chunks
        let deleted = store.delete_source("test.md").expect("Failed to delete source");
        assert_eq!(deleted, 2);

        let (sources, chunks_after) = store.counts().expect("Failed to get counts");
        assert_eq!(sources, 0);
        assert_eq!(chunks_after, 0);

        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_needs_update() {
        let path = temp_db_path();
        let store = Store::open(&path, 3).expect("Failed to open store");

        // Initially needs update
        assert!(store.needs_update("test.md", "hash1").expect("Failed to check"));

        store.upsert_source("test.md", "hash1").expect("Failed to upsert");

        // Same hash - no update needed
        assert!(!store.needs_update("test.md", "hash1").expect("Failed to check"));

        // Different hash - needs update
        assert!(store.needs_update("test.md", "hash2").expect("Failed to check"));

        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_dimension_mismatch() {
        let path = temp_db_path();
        let store = Store::open(&path, 3).expect("Failed to open store");

        store.upsert_source("test.md", "hash123").expect("Failed to upsert source");

        // Wrong dimension
        let embedding = vec![1.0, 0.0]; // Only 2 dimensions, expected 3
        let result = store.insert_chunk("chunk1", "test.md", 1, 10, "Test", &embedding);

        assert!(matches!(result, Err(StoreError::DimensionMismatch { expected: 3, actual: 2 })));

        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_search_ordering() {
        let path = temp_db_path();
        let store = Store::open(&path, 3).expect("Failed to open store");

        store.upsert_source("test.md", "hash123").expect("Failed to upsert source");

        // Insert chunks with different embeddings
        store.insert_chunk("chunk1", "test.md", 1, 10, "Exact match", &[1.0, 0.0, 0.0])
            .expect("Failed to insert");
        store.insert_chunk("chunk2", "test.md", 11, 20, "Partial match", &[0.7, 0.7, 0.0])
            .expect("Failed to insert");
        store.insert_chunk("chunk3", "test.md", 21, 30, "No match", &[0.0, 0.0, 1.0])
            .expect("Failed to insert");

        let query = vec![1.0, 0.0, 0.0];
        let results = store.search(&query, 10, 0).expect("Failed to search");

        assert_eq!(results.len(), 3);
        // First result should be exact match (lowest distance)
        assert_eq!(results[0].id, "chunk1");

        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_search_with_offset() {
        let path = temp_db_path();
        let store = Store::open(&path, 3).expect("Failed to open store");

        store.upsert_source("test.md", "hash123").expect("Failed to upsert source");

        for i in 0..5 {
            store.insert_chunk(
                &format!("chunk{}", i),
                "test.md",
                i * 10,
                (i + 1) * 10,
                &format!("Content {}", i),
                &[1.0 - (i as f32 * 0.1), 0.0, 0.0],
            ).expect("Failed to insert");
        }

        // Get first 2
        let results = store.search(&[1.0, 0.0, 0.0], 2, 0).expect("Failed to search");
        assert_eq!(results.len(), 2);

        // Get next 2 (offset 2)
        let results_offset = store.search(&[1.0, 0.0, 0.0], 2, 2).expect("Failed to search");
        assert_eq!(results_offset.len(), 2);

        // They should be different
        assert_ne!(results[0].id, results_offset[0].id);

        fs::remove_file(&path).ok();
    }
}
```

**Commit:** `test(river-embed): add comprehensive store tests`

---

## Task 10: Integration Test

**File:** `/home/cassie/river-engine/crates/river-embed/tests/integration.rs`

- [ ] **Step 10.1:** Create integration test file

Create a new integration test:

```rust
//! Integration tests for river-embed.

use std::time::Duration;

// Note: These tests require a running embedding service.
// For CI, consider mocking the embed client.

#[cfg(test)]
mod tests {
    use super::*;

    // This test verifies that the /next endpoint doesn't return
    // the same result as /search (cursor offset bug fix)
    #[tokio::test]
    #[ignore] // Requires running service
    async fn test_cursor_does_not_duplicate_first_result() {
        // This would require setting up the full service
        // For now, the unit tests in search.rs cover this
    }

    // Performance test - requires many chunks
    #[tokio::test]
    #[ignore] // Expensive test
    async fn test_search_performance_with_many_chunks() {
        // Would verify O(log n) performance with sqlite-vec
    }
}
```

**Commit:** `test(river-embed): add integration test scaffold`

---

## Task 11: Final Verification

- [ ] **Step 11.1:** Run all tests

```bash
cd crates/river-embed && cargo test
```

- [ ] **Step 11.2:** Check for warnings

```bash
cd crates/river-embed && cargo clippy
```

- [ ] **Step 11.3:** Verify build

```bash
cargo build -p river-embed
```

**Commit:** `chore(river-embed): fix any remaining clippy warnings`

---

## Verification Checklist

- [ ] sqlite-vec integrated and working
- [ ] Vector search is O(log n) not O(n)
- [ ] tokio::sync::Mutex used (no blocking)
- [ ] Cursor offset starts at 1 after /search
- [ ] Foreign keys enabled and cascade works
- [ ] Chunking handles large paragraphs
- [ ] Cursor cleanup task running
- [ ] Embed client has timeout
- [ ] Store tests added
- [ ] All tests pass

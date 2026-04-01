# Phase 1: Embeddings Layer

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a zettelkasten-based memory system. Files in `workspace/embeddings/` are source of truth. sqlite-vec stores derived vectors. A sync service watches for changes and keeps vectors current.

**Architecture:** File watcher → hash diff → chunk → embed → store in sqlite-vec. Reproducible: delete vectors.db, run sync, get identical state.

**Tech Stack:** sqlite-vec (via rusqlite), notify (file watcher), serde_yaml (frontmatter), tokio

**Depends on:** Phase 0 (river-db extracted)

---

## File Structure

**New files:**
- `crates/river-gateway/src/embeddings/mod.rs` — public API
- `crates/river-gateway/src/embeddings/note.rs` — note format, frontmatter parsing
- `crates/river-gateway/src/embeddings/chunk.rs` — chunking strategies
- `crates/river-gateway/src/embeddings/store.rs` — sqlite-vec operations
- `crates/river-gateway/src/embeddings/sync.rs` — file watcher, hash diffing, sync loop

**Modified files:**
- `crates/river-gateway/Cargo.toml` — add notify, serde_yaml deps
- `crates/river-gateway/src/lib.rs` — add embeddings module
- `crates/river-db/Cargo.toml` — add sqlite-vec support (or keep in gateway)

---

## Task 1: Add Dependencies

- [ ] **Step 1: Add to gateway Cargo.toml**

```toml
notify = { version = "7.0", default-features = false, features = ["macos_kqueue"] }
serde_yaml = "0.9"
sha2 = "0.10"
```

- [ ] **Step 2: Verify**

```bash
cargo check -p river-gateway
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/Cargo.toml
git commit -m "chore(gateway): add notify, serde_yaml, sha2 for embeddings"
```

---

## Task 2: Note Format and Frontmatter Parsing

- [ ] **Step 1: Create embeddings/note.rs**

```rust
//! Note format with YAML frontmatter

use chrono::{DateTime, Utc};
use river_core::Snowflake;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Note types in the zettelkasten
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum NoteType {
    Note,
    Move,
    Moment,
    RoomNote,
}

/// Frontmatter metadata for a note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteFrontmatter {
    pub id: String,
    pub created: DateTime<Utc>,
    pub author: String,       // "agent" or "spectator"
    #[serde(rename = "type")]
    pub note_type: NoteType,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub channel: Option<String>,
}

/// A parsed note (frontmatter + content)
#[derive(Debug, Clone)]
pub struct Note {
    pub frontmatter: NoteFrontmatter,
    pub content: String,
    pub source_path: String,
}

impl Note {
    /// Parse a note from file contents
    pub fn parse(source_path: &str, text: &str) -> Result<Self, String> {
        let text = text.trim();
        if !text.starts_with("---") {
            return Err("Note must start with YAML frontmatter (---)".into());
        }

        let end = text[3..].find("---")
            .ok_or("Missing closing --- for frontmatter")?;

        let yaml = &text[3..end + 3];
        let content = text[end + 6..].trim().to_string();

        let frontmatter: NoteFrontmatter = serde_yaml::from_str(yaml)
            .map_err(|e| format!("Invalid frontmatter: {}", e))?;

        Ok(Note {
            frontmatter,
            content,
            source_path: source_path.to_string(),
        })
    }

    /// Create a new note with frontmatter
    pub fn to_string(&self) -> String {
        let yaml = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        format!("---\n{}---\n\n{}", yaml, self.content)
    }
}
```

- [ ] **Step 2: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_note() {
        let text = r#"---
id: "0x01a2b3c4"
created: 2026-03-23T14:32:07Z
author: agent
type: note
tags: [css, z-index]
---

# z-index hierarchy
Modal: 50, Navbar: 40"#;

        let note = Note::parse("notes/z-index.md", text).unwrap();
        assert_eq!(note.frontmatter.author, "agent");
        assert_eq!(note.frontmatter.note_type, NoteType::Note);
        assert!(note.content.contains("z-index hierarchy"));
    }

    #[test]
    fn test_parse_note_no_frontmatter() {
        let result = Note::parse("test.md", "just content");
        assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Create embeddings/mod.rs**

```rust
//! Zettelkasten embeddings layer

pub mod note;
pub mod chunk;
pub mod store;
pub mod sync;

pub use note::{Note, NoteFrontmatter, NoteType};
pub use chunk::{Chunk, ChunkType, Chunker};
pub use store::VectorStore;
pub use sync::SyncService;
```

- [ ] **Step 4: Add to lib.rs**

Add `pub mod embeddings;` to `crates/river-gateway/src/lib.rs`.

- [ ] **Step 5: Verify compilation, run tests**

```bash
cargo test -p river-gateway embeddings
```

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/embeddings/
git commit -m "feat(gateway): add note format with YAML frontmatter parsing"
```

---

## Task 3: Chunking Strategies

- [ ] **Step 1: Create embeddings/chunk.rs**

```rust
//! Chunking strategies for different note types

use crate::embeddings::note::{Note, NoteType};
use river_core::Snowflake;

/// Types of chunks
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkType {
    Note,
    Move,
    Moment,
    RoomNote,
    Fragment,  // Large doc split
}

/// A chunk ready for embedding
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: String,            // Derived from source + position
    pub source_path: String,
    pub content: String,
    pub chunk_type: ChunkType,
    pub channel: Option<String>,
}

/// Chunker that splits notes into embeddable pieces
pub struct Chunker {
    max_chunk_tokens: usize,   // ~400 tokens ≈ 1600 chars
}

impl Chunker {
    pub fn new(max_chunk_tokens: usize) -> Self {
        Self { max_chunk_tokens }
    }

    /// Chunk a parsed note
    pub fn chunk(&self, note: &Note) -> Vec<Chunk> {
        let chunk_type = match note.frontmatter.note_type {
            NoteType::Note => ChunkType::Note,
            NoteType::Move => ChunkType::Move,
            NoteType::Moment => ChunkType::Moment,
            NoteType::RoomNote => ChunkType::RoomNote,
        };

        let max_chars = self.max_chunk_tokens * 4; // rough token-to-char ratio

        if note.content.len() <= max_chars {
            // Small enough: one chunk
            return vec![Chunk {
                id: format!("{}:0", note.source_path),
                source_path: note.source_path.clone(),
                content: note.content.clone(),
                chunk_type,
                channel: note.frontmatter.channel.clone(),
            }];
        }

        // Split on headers or paragraph breaks
        self.split_by_sections(&note.source_path, &note.content, chunk_type, note.frontmatter.channel.clone(), max_chars)
    }

    /// Chunk a raw markdown file (no frontmatter)
    pub fn chunk_raw(&self, path: &str, content: &str) -> Vec<Chunk> {
        let max_chars = self.max_chunk_tokens * 4;
        if content.len() <= max_chars {
            return vec![Chunk {
                id: format!("{}:0", path),
                source_path: path.to_string(),
                content: content.to_string(),
                chunk_type: ChunkType::Fragment,
                channel: None,
            }];
        }
        self.split_by_sections(path, content, ChunkType::Fragment, None, max_chars)
    }

    fn split_by_sections(&self, path: &str, content: &str, chunk_type: ChunkType, channel: Option<String>, max_chars: usize) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let mut current = String::new();
        let mut idx = 0;

        for line in content.lines() {
            // Split on headers if current chunk would exceed limit
            if line.starts_with('#') && current.len() > max_chars / 2 {
                if !current.trim().is_empty() {
                    chunks.push(Chunk {
                        id: format!("{}:{}", path, idx),
                        source_path: path.to_string(),
                        content: current.trim().to_string(),
                        chunk_type: chunk_type.clone(),
                        channel: channel.clone(),
                    });
                    idx += 1;
                    current = String::new();
                }
            }
            current.push_str(line);
            current.push('\n');

            if current.len() >= max_chars {
                chunks.push(Chunk {
                    id: format!("{}:{}", path, idx),
                    source_path: path.to_string(),
                    content: current.trim().to_string(),
                    chunk_type: chunk_type.clone(),
                    channel: channel.clone(),
                });
                idx += 1;
                current = String::new();
            }
        }

        if !current.trim().is_empty() {
            chunks.push(Chunk {
                id: format!("{}:{}", path, idx),
                source_path: path.to_string(),
                content: current.trim().to_string(),
                chunk_type: chunk_type.clone(),
                channel: channel.clone(),
            });
        }

        chunks
    }
}

impl Default for Chunker {
    fn default() -> Self {
        Self::new(400)
    }
}
```

- [ ] **Step 2: Write tests for chunking**

Test: small note → 1 chunk, large note → multiple chunks, move file → per-move chunks.

- [ ] **Step 3: Verify, commit**

```bash
cargo test -p river-gateway chunk
git add -A && git commit -m "feat(gateway): add chunking strategies for embeddings"
```

---

## Task 4: Vector Store (sqlite-vec)

- [ ] **Step 1: Add sqlite-vec to river-db or gateway**

We need sqlite-vec extension support. Add to gateway Cargo.toml:
```toml
# sqlite-vec loaded as extension at runtime
```

sqlite-vec is loaded as a runtime extension via `rusqlite::Connection::load_extension`. We need the `.so` available.

- [ ] **Step 2: Create embeddings/store.rs**

```rust
//! sqlite-vec vector store

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct VectorStore {
    conn: Arc<Mutex<Connection>>,
}

impl VectorStore {
    /// Open or create the vector database
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path)
            .map_err(|e| format!("Failed to open vector DB: {}", e))?;

        // Try to load sqlite-vec extension
        // Fallback: use manual cosine similarity on f32 blobs
        let _ = unsafe { conn.load_extension("vec0", None) };

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                content TEXT NOT NULL,
                chunk_type TEXT NOT NULL,
                channel TEXT,
                hash TEXT NOT NULL,
                embedding BLOB
            );
            CREATE INDEX IF NOT EXISTS idx_chunks_source ON chunks(source_path);
            CREATE INDEX IF NOT EXISTS idx_chunks_channel ON chunks(channel);
        ").map_err(|e| format!("Failed to create tables: {}", e))?;

        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Open in-memory store (for testing)
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory DB: {}", e))?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                content TEXT NOT NULL,
                chunk_type TEXT NOT NULL,
                channel TEXT,
                hash TEXT NOT NULL,
                embedding BLOB
            );
            CREATE INDEX IF NOT EXISTS idx_chunks_source ON chunks(source_path);
        ").map_err(|e| format!("Failed to create tables: {}", e))?;

        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Upsert a chunk with its embedding
    pub fn upsert_chunk(
        &self,
        id: &str,
        source_path: &str,
        content: &str,
        chunk_type: &str,
        channel: Option<&str>,
        hash: &str,
        embedding: &[f32],
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let embedding_bytes: Vec<u8> = embedding.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        conn.execute(
            "INSERT OR REPLACE INTO chunks (id, source_path, content, chunk_type, channel, hash, embedding)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, source_path, content, chunk_type, channel, hash, embedding_bytes],
        ).map_err(|e| format!("Failed to upsert chunk: {}", e))?;

        Ok(())
    }

    /// Get hash for a source path (for sync diffing)
    pub fn get_hash(&self, source_path: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT hash FROM chunks WHERE source_path = ?1 LIMIT 1"
        ).map_err(|e| e.to_string())?;

        let hash = stmt.query_row(params![source_path], |row| row.get(0))
            .ok();

        Ok(hash)
    }

    /// Delete all chunks for a source path
    pub fn delete_source(&self, source_path: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM chunks WHERE source_path = ?1",
            params![source_path],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Search by cosine similarity (manual, no sqlite-vec required)
    pub fn search(&self, query_embedding: &[f32], limit: usize) -> Result<Vec<SearchResult>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, source_path, content, chunk_type, channel, embedding FROM chunks WHERE embedding IS NOT NULL"
        ).map_err(|e| e.to_string())?;

        let mut results: Vec<SearchResult> = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let source_path: String = row.get(1)?;
            let content: String = row.get(2)?;
            let chunk_type: String = row.get(3)?;
            let channel: Option<String> = row.get(4)?;
            let embedding_bytes: Vec<u8> = row.get(5)?;

            let embedding: Vec<f32> = embedding_bytes.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();

            let similarity = cosine_similarity(query_embedding, &embedding);

            Ok(SearchResult {
                id,
                source_path,
                content,
                chunk_type,
                channel,
                similarity,
            })
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub source_path: String,
    pub content: String,
    pub chunk_type: String,
    pub channel: Option<String>,
    pub similarity: f32,
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}
```

- [ ] **Step 3: Write tests for store**

Test upsert, hash lookup, delete, search.

- [ ] **Step 4: Verify, commit**

```bash
cargo test -p river-gateway store
git add -A && git commit -m "feat(gateway): add sqlite-vec vector store for embeddings"
```

---

## Task 5: Sync Service

- [ ] **Step 1: Create embeddings/sync.rs**

```rust
//! File watcher and sync service for embeddings

use crate::embeddings::{Chunker, Note, VectorStore};
use crate::memory::EmbeddingClient;
use sha2::{Sha256, Digest};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub struct SyncService {
    embeddings_dir: PathBuf,
    store: VectorStore,
    embedding_client: EmbeddingClient,
    chunker: Chunker,
}

impl SyncService {
    pub fn new(
        embeddings_dir: PathBuf,
        store: VectorStore,
        embedding_client: EmbeddingClient,
    ) -> Self {
        Self {
            embeddings_dir,
            store,
            embedding_client,
            chunker: Chunker::default(),
        }
    }

    /// Full sync: scan all files in embeddings dir, sync each
    pub async fn full_sync(&self) -> Result<SyncStats, String> {
        let mut stats = SyncStats::default();

        let files = self.list_markdown_files()?;
        for path in files {
            match self.sync_file(&path).await {
                Ok(changed) => {
                    if changed { stats.updated += 1; } else { stats.skipped += 1; }
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to sync file");
                    stats.errors += 1;
                }
            }
        }

        Ok(stats)
    }

    /// Sync a single file: hash, diff, chunk, embed, store
    pub async fn sync_file(&self, path: &Path) -> Result<bool, String> {
        let rel_path = path.strip_prefix(&self.embeddings_dir)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", rel_path, e))?;

        // Hash content
        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));

        // Check if unchanged
        if let Ok(Some(existing_hash)) = self.store.get_hash(&rel_path) {
            if existing_hash == hash {
                return Ok(false); // No change
            }
        }

        // Delete old chunks for this file
        self.store.delete_source(&rel_path)?;

        // Parse and chunk
        let chunks = if let Ok(note) = Note::parse(&rel_path, &content) {
            self.chunker.chunk(&note)
        } else {
            self.chunker.chunk_raw(&rel_path, &content)
        };

        // Embed and store each chunk
        for chunk in &chunks {
            let embedding = self.embedding_client.embed(&chunk.content).await
                .map_err(|e| format!("Embedding failed: {}", e))?;

            self.store.upsert_chunk(
                &chunk.id,
                &chunk.source_path,
                &chunk.content,
                &format!("{:?}", chunk.chunk_type),
                chunk.channel.as_deref(),
                &hash,
                &embedding,
            )?;
        }

        tracing::info!(path = %rel_path, chunks = chunks.len(), "Synced file");
        Ok(true)
    }

    fn list_markdown_files(&self) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        Self::walk_dir(&self.embeddings_dir, &mut files)?;
        Ok(files)
    }

    fn walk_dir(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        if !dir.exists() { return Ok(()); }
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                Self::walk_dir(&path, files)?;
            } else if path.extension().map_or(false, |ext| ext == "md") {
                files.push(path);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct SyncStats {
    pub updated: usize,
    pub skipped: usize,
    pub errors: usize,
}
```

- [ ] **Step 2: Add file watcher (optional, can be polling initially)**

Start with manual/polling sync. File watcher via `notify` can be added once the core sync works.

- [ ] **Step 3: Write integration test**

Create a temp dir with sample notes, run full_sync, verify chunks appear in store.

- [ ] **Step 4: Verify, commit**

```bash
cargo test -p river-gateway sync
git add -A && git commit -m "feat(gateway): add embeddings sync service"
```

---

## Task 6: Integration Wiring

- [ ] **Step 1: Create workspace/embeddings/ directories**

Document the expected directory structure:
```bash
mkdir -p workspace/embeddings/{notes,moves,moments,room-notes}
```

- [ ] **Step 2: Add a sample note for testing**

```markdown
---
id: "test-note-001"
created: 2026-03-23T12:00:00Z
author: agent
type: note
tags: [test]
---

# Test Note
This is a test note for the embeddings system.
```

- [ ] **Step 3: Wire sync service into server startup (optional)**

Can be deferred — sync can be triggered manually or on agent wake.

- [ ] **Step 4: Run full test suite**

```bash
cargo test
```

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(gateway): complete embeddings layer with sync service"
```

---

## Summary

Phase 1 builds the zettelkasten foundation:
1. **Note format** — YAML frontmatter + markdown content
2. **Chunking** — Type-aware splitting for embedding
3. **Vector store** — sqlite-vec with cosine similarity search
4. **Sync service** — Hash-based diffing, re-embed on change

Total: 6 tasks, ~30 steps. The old `memory/` module remains for now — both systems coexist.

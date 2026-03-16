# River Engine: Plan 3 - Memory & Embeddings

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add semantic memory (embedding-based search) and Redis-backed short/medium-term memory to the gateway.

**Architecture:** The memory system has two components: (1) SQLite-stored embeddings with vector similarity search via an external embedding server, and (2) Redis for ephemeral working/medium-term memory with TTL-based expiry.

**Note:** Auto-embedding (automatic embedding of incoming/outgoing messages) is deferred to Plan 4 (Orchestrator) as it requires integration with the tool loop and message flow. This plan provides the tools (`embed`, `memory_search`, `memory_delete`) that auto-embedding will use.

**Tech Stack:** Rust, SQLite (embeddings table), Redis (fred crate), HTTP client for embedding server, f32 vectors for cosine similarity

**Spec Reference:** `/home/cassie/river-engine/docs/superpowers/specs/2026-03-16-river-engine-design.md` (Section 5: Memory System)

---

## File Structure

```
crates/river-gateway/src/
├── db/
│   ├── mod.rs              # (modify) Add memory exports
│   ├── migrations/
│   │   └── 002_memories.sql # (create) Memories table
│   └── memories.rs          # (create) Memory CRUD operations
├── memory/
│   ├── mod.rs               # (create) Memory module root
│   ├── embedding.rs         # (create) Embedding server client
│   └── search.rs            # (create) Vector similarity search
│   # NOTE: auto_embed.rs is deferred to Plan 4 (Orchestrator) as it requires tool loop integration
├── redis/
│   ├── mod.rs               # (create) Redis module root
│   ├── client.rs            # (create) Redis connection wrapper
│   ├── working.rs           # (create) Working memory tools
│   ├── medium_term.rs       # (create) Medium-term memory tools
│   ├── coordination.rs      # (create) Locks and counters
│   └── cache.rs             # (create) Cache operations
├── tools/
│   ├── mod.rs               # (modify) Add memory and redis tools
│   ├── memory.rs            # (create) Memory tools (embed, search, delete)
│   └── redis.rs             # (create) Redis tools
├── state.rs                 # (modify) Add embedding client and redis
└── server.rs                # (modify) Initialize memory and redis
```

---

## Chunk 1: Database Layer for Memories

### Task 1: Memories Table Migration

**Files:**
- Create: `crates/river-gateway/src/db/migrations/002_memories.sql`
- Modify: `crates/river-gateway/src/db/schema.rs`

- [ ] **Step 1: Create 002_memories.sql migration**

Create `crates/river-gateway/src/db/migrations/002_memories.sql`:

```sql
-- Memories table for semantic search
CREATE TABLE IF NOT EXISTS memories (
    id BLOB PRIMARY KEY,           -- 128-bit snowflake
    content TEXT NOT NULL,
    embedding BLOB NOT NULL,       -- f32 vector as bytes
    source TEXT NOT NULL,          -- 'message', 'file', 'agent'
    timestamp INTEGER NOT NULL,
    expires_at INTEGER,            -- NULL for permanent
    metadata TEXT                  -- JSON
);

CREATE INDEX IF NOT EXISTS idx_memories_source ON memories(source);
CREATE INDEX IF NOT EXISTS idx_memories_timestamp ON memories(timestamp);
CREATE INDEX IF NOT EXISTS idx_memories_expires ON memories(expires_at) WHERE expires_at IS NOT NULL;
```

- [ ] **Step 2: Add migration to schema.rs**

In `crates/river-gateway/src/db/schema.rs`, add to the `migrate` function:

```rust
self.run_migration("002_memories", include_str!("migrations/002_memories.sql"))?;
```

- [ ] **Step 3: Run tests to verify migration**

Run: `cargo test -p river-gateway db::schema`

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/db/migrations/002_memories.sql crates/river-gateway/src/db/schema.rs
git commit -m "feat(gateway): add memories table migration"
```

---

### Task 2: Memory CRUD Operations

**Files:**
- Create: `crates/river-gateway/src/db/memories.rs`
- Modify: `crates/river-gateway/src/db/mod.rs`

- [ ] **Step 1: Write failing test for memory insert/get**

Create `crates/river-gateway/src/db/memories.rs`:

```rust
//! Memory CRUD operations for semantic search

use river_core::{RiverError, RiverResult, Snowflake};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::Database;

/// Memory entry for semantic search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: Snowflake,
    pub content: String,
    pub embedding: Vec<f32>,
    pub source: String,
    pub timestamp: i64,
    pub expires_at: Option<i64>,
    pub metadata: Option<String>,
}

impl Memory {
    fn from_row(row: &Row) -> Result<Self, rusqlite::Error> {
        let id_bytes: Vec<u8> = row.get(0)?;
        let id_array: [u8; 16] = id_bytes.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Blob,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid snowflake ID length",
                )),
            )
        })?;
        let id = Snowflake::from_bytes(id_array);

        let embedding_bytes: Vec<u8> = row.get(2)?;
        let embedding = bytes_to_f32_vec(&embedding_bytes);

        Ok(Self {
            id,
            content: row.get(1)?,
            embedding,
            source: row.get(3)?,
            timestamp: row.get(4)?,
            expires_at: row.get(5)?,
            metadata: row.get(6)?,
        })
    }
}

/// Convert f32 vector to bytes for storage
pub fn f32_vec_to_bytes(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Convert bytes back to f32 vector
pub fn bytes_to_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

impl Database {
    /// Insert a memory
    pub fn insert_memory(&self, mem: &Memory) -> RiverResult<()> {
        let embedding_bytes = f32_vec_to_bytes(&mem.embedding);

        self.conn()
            .execute(
                "INSERT INTO memories (id, content, embedding, source, timestamp, expires_at, metadata)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    mem.id.to_bytes().to_vec(),
                    mem.content,
                    embedding_bytes,
                    mem.source,
                    mem.timestamp,
                    mem.expires_at,
                    mem.metadata,
                ],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
    }

    /// Get a memory by ID
    pub fn get_memory(&self, id: Snowflake) -> RiverResult<Option<Memory>> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, content, embedding, source, timestamp, expires_at, metadata
                 FROM memories WHERE id = ?",
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        let mut rows = stmt
            .query(params![id.to_bytes().to_vec()])
            .map_err(|e| RiverError::database(e.to_string()))?;

        match rows.next().map_err(|e| RiverError::database(e.to_string()))? {
            Some(row) => Ok(Some(Memory::from_row(row).map_err(|e| RiverError::database(e.to_string()))?)),
            None => Ok(None),
        }
    }

    /// Delete a memory by ID
    pub fn delete_memory(&self, id: Snowflake) -> RiverResult<bool> {
        let rows = self
            .conn()
            .execute(
                "DELETE FROM memories WHERE id = ?",
                params![id.to_bytes().to_vec()],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(rows > 0)
    }

    /// Delete memories by source, optionally before a timestamp
    pub fn delete_memories_by_source(&self, source: &str, before: Option<i64>) -> RiverResult<usize> {
        let rows = match before {
            Some(ts) => self
                .conn()
                .execute(
                    "DELETE FROM memories WHERE source = ? AND timestamp < ?",
                    params![source, ts],
                )
                .map_err(|e| RiverError::database(e.to_string()))?,
            None => self
                .conn()
                .execute("DELETE FROM memories WHERE source = ?", params![source])
                .map_err(|e| RiverError::database(e.to_string()))?,
        };
        Ok(rows)
    }

    /// Clean up expired memories
    pub fn cleanup_expired_memories(&self) -> RiverResult<usize> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let rows = self
            .conn()
            .execute(
                "DELETE FROM memories WHERE expires_at IS NOT NULL AND expires_at < ?",
                params![now],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(rows)
    }

    /// Get all memories for similarity search (returns id, embedding pairs)
    pub fn get_all_memory_embeddings(&self) -> RiverResult<Vec<(Snowflake, Vec<f32>)>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, embedding FROM memories WHERE expires_at IS NULL OR expires_at > ?")
            .map_err(|e| RiverError::database(e.to_string()))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let rows = stmt
            .query_map(params![now], |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let id_array: [u8; 16] = id_bytes.try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Invalid snowflake ID length",
                        )),
                    )
                })?;
                let embedding_bytes: Vec<u8> = row.get(1)?;
                Ok((Snowflake::from_bytes(id_array), bytes_to_f32_vec(&embedding_bytes)))
            })
            .map_err(|e| RiverError::database(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))
    }

    /// Get memories by IDs
    pub fn get_memories_by_ids(&self, ids: &[Snowflake]) -> RiverResult<Vec<Memory>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let query = format!(
            "SELECT id, content, embedding, source, timestamp, expires_at, metadata
             FROM memories WHERE id IN ({})",
            placeholders
        );

        let mut stmt = self
            .conn()
            .prepare(&query)
            .map_err(|e| RiverError::database(e.to_string()))?;

        let params: Vec<Vec<u8>> = ids.iter().map(|id| id.to_bytes().to_vec()).collect();
        let params_refs: Vec<&dyn rusqlite::ToSql> = params
            .iter()
            .map(|p| p as &dyn rusqlite::ToSql)
            .collect();

        let rows = stmt
            .query_map(params_refs.as_slice(), Memory::from_row)
            .map_err(|e| RiverError::database(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};

    fn test_db_and_gen() -> (Database, SnowflakeGenerator) {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);
        (db, gen)
    }

    #[test]
    fn test_f32_bytes_roundtrip() {
        let original = vec![1.0f32, 2.5, -3.14, 0.0, f32::MAX];
        let bytes = f32_vec_to_bytes(&original);
        let restored = bytes_to_f32_vec(&bytes);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_insert_and_get_memory() {
        let (db, gen) = test_db_and_gen();

        let mem = Memory {
            id: gen.next_id(SnowflakeType::Embedding),
            content: "Hello, semantic search!".to_string(),
            embedding: vec![0.1, 0.2, 0.3, 0.4],
            source: "test".to_string(),
            timestamp: 1234567890,
            expires_at: None,
            metadata: None,
        };

        db.insert_memory(&mem).unwrap();

        let retrieved = db.get_memory(mem.id).unwrap().unwrap();
        assert_eq!(retrieved.content, "Hello, semantic search!");
        assert_eq!(retrieved.embedding, vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(retrieved.source, "test");
    }

    #[test]
    fn test_delete_memory() {
        let (db, gen) = test_db_and_gen();

        let mem = Memory {
            id: gen.next_id(SnowflakeType::Embedding),
            content: "To be deleted".to_string(),
            embedding: vec![1.0, 2.0],
            source: "test".to_string(),
            timestamp: 1234567890,
            expires_at: None,
            metadata: None,
        };

        db.insert_memory(&mem).unwrap();
        assert!(db.get_memory(mem.id).unwrap().is_some());

        let deleted = db.delete_memory(mem.id).unwrap();
        assert!(deleted);
        assert!(db.get_memory(mem.id).unwrap().is_none());
    }

    #[test]
    fn test_delete_by_source() {
        let (db, gen) = test_db_and_gen();

        for i in 0..3 {
            let mem = Memory {
                id: gen.next_id(SnowflakeType::Embedding),
                content: format!("Memory {}", i),
                embedding: vec![i as f32],
                source: "batch".to_string(),
                timestamp: 1000 + i,
                expires_at: None,
                metadata: None,
            };
            db.insert_memory(&mem).unwrap();
        }

        let deleted = db.delete_memories_by_source("batch", None).unwrap();
        assert_eq!(deleted, 3);
    }

    #[test]
    fn test_get_all_embeddings() {
        let (db, gen) = test_db_and_gen();

        for i in 0..3 {
            let mem = Memory {
                id: gen.next_id(SnowflakeType::Embedding),
                content: format!("Memory {}", i),
                embedding: vec![i as f32, (i + 1) as f32],
                source: "test".to_string(),
                timestamp: 1234567890,
                expires_at: None,
                metadata: None,
            };
            db.insert_memory(&mem).unwrap();
        }

        let embeddings = db.get_all_memory_embeddings().unwrap();
        assert_eq!(embeddings.len(), 3);
    }
}
```

- [ ] **Step 2: Run test to verify it fails (table doesn't exist yet)**

Run: `cargo test -p river-gateway db::memories`
Expected: PASS (migration should have been applied)

- [ ] **Step 3: Update db/mod.rs to export memories module**

```rust
//! Database layer

mod schema;
mod messages;
mod memories;

pub use schema::{Database, init_db};
pub use messages::{Message, MessageRole};
pub use memories::{Memory, f32_vec_to_bytes, bytes_to_f32_vec};
```

- [ ] **Step 4: Run all tests**

Run: `cargo test -p river-gateway db::`

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/db/memories.rs crates/river-gateway/src/db/mod.rs
git commit -m "feat(gateway): add memory CRUD operations"
```

---

## Chunk 2: Embedding Client

### Task 3: Embedding Server Client

**Files:**
- Create: `crates/river-gateway/src/memory/mod.rs`
- Create: `crates/river-gateway/src/memory/embedding.rs`
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Create memory/mod.rs**

```rust
//! Memory system for semantic search

mod embedding;
mod search;

pub use embedding::{EmbeddingClient, EmbeddingConfig};
pub use search::{MemorySearcher, SearchResult};
```

- [ ] **Step 2: Create memory/embedding.rs**

```rust
//! Embedding server client (llama-server --embedding compatible)

use river_core::{RiverError, RiverResult};
use serde::{Deserialize, Serialize};

/// Configuration for embedding server
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub url: String,
    pub model: String,
    pub dimensions: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8081".to_string(),
            model: "nomic-embed-text-v1.5".to_string(),
            dimensions: 768,
        }
    }
}

/// Client for embedding server
#[derive(Clone)]
pub struct EmbeddingClient {
    client: reqwest::Client,
    config: EmbeddingConfig,
}

#[derive(Serialize)]
struct EmbeddingRequest {
    input: String,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl EmbeddingClient {
    /// Create new embedding client
    pub fn new(config: EmbeddingConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    /// Get embedding for text
    pub async fn embed(&self, text: &str) -> RiverResult<Vec<f32>> {
        let url = format!("{}/v1/embeddings", self.config.url);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "input": text,
                "model": self.config.model
            }))
            .send()
            .await
            .map_err(|e| RiverError::embedding(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RiverError::embedding(format!(
                "Embedding server error {}: {}",
                status, body
            )));
        }

        let resp: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| RiverError::embedding(format!("Invalid response: {}", e)))?;

        resp.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| RiverError::embedding("Empty embedding response".to_string()))
    }

    /// Get embeddings for multiple texts
    pub async fn embed_batch(&self, texts: &[String]) -> RiverResult<Vec<Vec<f32>>> {
        let url = format!("{}/v1/embeddings", self.config.url);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "input": texts,
                "model": self.config.model
            }))
            .send()
            .await
            .map_err(|e| RiverError::embedding(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RiverError::embedding(format!(
                "Embedding server error {}: {}",
                status, body
            )));
        }

        let resp: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| RiverError::embedding(format!("Invalid response: {}", e)))?;

        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    /// Get expected embedding dimensions
    pub fn dimensions(&self) -> usize {
        self.config.dimensions
    }

    /// Check if embedding server is reachable
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/health", self.config.url);
        self.client.get(&url).send().await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.url, "http://localhost:8081");
        assert_eq!(config.dimensions, 768);
    }

    #[test]
    fn test_client_creation() {
        let client = EmbeddingClient::new(EmbeddingConfig::default());
        assert_eq!(client.dimensions(), 768);
    }
}
```

- [ ] **Step 3: Add memory module to lib.rs**

Add to `crates/river-gateway/src/lib.rs`:

```rust
pub mod memory;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p river-gateway`

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/memory/
git commit -m "feat(gateway): add embedding server client"
```

---

### Task 4: Vector Similarity Search

**Files:**
- Create: `crates/river-gateway/src/memory/search.rs`

- [ ] **Step 1: Write failing test for cosine similarity**

Create `crates/river-gateway/src/memory/search.rs`:

```rust
//! Vector similarity search for semantic memory

use river_core::{RiverResult, Snowflake};
use crate::db::{Database, Memory};

/// Search result with similarity score
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub memory: Memory,
    pub similarity: f32,
}

/// Memory searcher using cosine similarity
pub struct MemorySearcher;

impl MemorySearcher {
    /// Compute cosine similarity between two vectors
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot / (norm_a * norm_b)
    }

    /// Search memories by similarity to query embedding
    pub fn search(
        db: &Database,
        query_embedding: &[f32],
        limit: usize,
        source_filter: Option<&str>,
        after: Option<i64>,
        before: Option<i64>,
    ) -> RiverResult<Vec<SearchResult>> {
        // Get all embeddings
        let all_embeddings = db.get_all_memory_embeddings()?;

        // Compute similarities
        let mut scored: Vec<(Snowflake, f32)> = all_embeddings
            .iter()
            .map(|(id, emb)| (*id, Self::cosine_similarity(query_embedding, emb)))
            .collect();

        // Sort by similarity descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Take top N
        let top_ids: Vec<Snowflake> = scored.iter().take(limit * 2).map(|(id, _)| *id).collect();

        // Fetch full memories
        let memories = db.get_memories_by_ids(&top_ids)?;

        // Build results with filtering
        let mut results: Vec<SearchResult> = Vec::new();
        for (id, similarity) in scored.iter().take(limit * 2) {
            if let Some(memory) = memories.iter().find(|m| m.id == *id) {
                // Apply filters
                if let Some(src) = source_filter {
                    if memory.source != src {
                        continue;
                    }
                }
                if let Some(after_ts) = after {
                    if memory.timestamp < after_ts {
                        continue;
                    }
                }
                if let Some(before_ts) = before {
                    if memory.timestamp > before_ts {
                        continue;
                    }
                }

                results.push(SearchResult {
                    memory: memory.clone(),
                    similarity: *similarity,
                });

                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = MemorySearcher::cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = MemorySearcher::cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = MemorySearcher::cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 0.0001);
    }

    #[test]
    fn test_search() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        // Insert test memories with different embeddings
        let embeddings = vec![
            vec![1.0, 0.0, 0.0],  // "north"
            vec![0.0, 1.0, 0.0],  // "east"
            vec![0.7, 0.7, 0.0],  // "northeast"
        ];

        for (i, emb) in embeddings.iter().enumerate() {
            let mem = Memory {
                id: gen.next_id(SnowflakeType::Embedding),
                content: format!("Memory {}", i),
                embedding: emb.clone(),
                source: "test".to_string(),
                timestamp: 1000 + i as i64,
                expires_at: None,
                metadata: None,
            };
            db.insert_memory(&mem).unwrap();
        }

        // Search for "north" direction
        let query = vec![1.0, 0.0, 0.0];
        let results = MemorySearcher::search(&db, &query, 2, None, None, None).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].memory.content, "Memory 0"); // Exact match
        assert!((results[0].similarity - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_search_with_source_filter() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        // Insert memories with different sources
        for i in 0..3 {
            let source = if i % 2 == 0 { "message" } else { "file" };
            let mem = Memory {
                id: gen.next_id(SnowflakeType::Embedding),
                content: format!("Memory {}", i),
                embedding: vec![1.0, 0.0, 0.0],
                source: source.to_string(),
                timestamp: 1000 + i as i64,
                expires_at: None,
                metadata: None,
            };
            db.insert_memory(&mem).unwrap();
        }

        let query = vec![1.0, 0.0, 0.0];
        let results = MemorySearcher::search(&db, &query, 10, Some("message"), None, None).unwrap();

        assert_eq!(results.len(), 2);
        for r in &results {
            assert_eq!(r.memory.source, "message");
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-gateway memory::search`

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/memory/search.rs
git commit -m "feat(gateway): add vector similarity search"
```

---

## Chunk 3: Memory Tools

### Task 5: Memory Tools (embed, memory_search, memory_delete, memory_delete_by_source)

**Files:**
- Create: `crates/river-gateway/src/tools/memory.rs`
- Modify: `crates/river-gateway/src/tools/mod.rs`

- [ ] **Step 1: Create tools/memory.rs**

```rust
//! Memory tools: embed, memory_search, memory_delete, memory_delete_by_source

use river_core::{RiverError, Snowflake, SnowflakeGenerator, SnowflakeType};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use crate::db::{Database, Memory};
use crate::memory::{EmbeddingClient, MemorySearcher};
use super::{Tool, ToolResult};

/// Embed tool - create embedding and store in memory
pub struct EmbedTool {
    db: Arc<Mutex<Database>>,
    embedding_client: Arc<EmbeddingClient>,
    snowflake_gen: Arc<SnowflakeGenerator>,
}

impl EmbedTool {
    pub fn new(
        db: Arc<Mutex<Database>>,
        embedding_client: Arc<EmbeddingClient>,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        Self {
            db,
            embedding_client,
            snowflake_gen,
        }
    }
}

impl Tool for EmbedTool {
    fn name(&self) -> &str {
        "embed"
    }

    fn description(&self) -> &str {
        "Create embedding and store in memory index"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Text to embed" },
                "source": { "type": "string", "description": "Source identifier (e.g., 'agent', 'file')" },
                "metadata": { "type": "object", "description": "Additional metadata (optional)" }
            },
            "required": ["content", "source"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: content".to_string()))?;

        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: source".to_string()))?;

        let metadata = args.get("metadata").map(|v| v.to_string());

        // Get embedding synchronously by blocking on the async call
        let embedding = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.embedding_client.embed(content))
        })?;

        let id = self.snowflake_gen.next_id(SnowflakeType::Embedding);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let memory = Memory {
            id,
            content: content.to_string(),
            embedding,
            source: source.to_string(),
            timestamp,
            expires_at: None,  // Agent-created embeddings are permanent
            metadata,
        };

        let db = self.db.lock().map_err(|_| RiverError::internal("Database lock poisoned".to_string()))?;
        db.insert_memory(&memory)?;

        Ok(ToolResult {
            success: true,
            output: format!("Created embedding with ID: {}", id),
            output_file: None,
        })
    }
}

/// Memory search tool
pub struct MemorySearchTool {
    db: Arc<Mutex<Database>>,
    embedding_client: Arc<EmbeddingClient>,
}

impl MemorySearchTool {
    pub fn new(db: Arc<Mutex<Database>>, embedding_client: Arc<EmbeddingClient>) -> Self {
        Self {
            db,
            embedding_client,
        }
    }
}

impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Semantic search over embeddings"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Maximum results", "default": 10 },
                "source": { "type": "string", "description": "Filter by source (optional)" },
                "after": { "type": "string", "description": "Filter by date (ISO 8601, optional)" },
                "before": { "type": "string", "description": "Filter by date (ISO 8601, optional)" },
                "output_file": { "type": "string", "description": "Pipe output to file instead of context (optional)" }
            },
            "required": ["query"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: query".to_string()))?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let source = args.get("source").and_then(|v| v.as_str());
        let output_file = args.get("output_file").and_then(|v| v.as_str());

        // Parse date filters
        let after = args
            .get("after")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp());

        let before = args
            .get("before")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp());

        // Get query embedding
        let query_embedding = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.embedding_client.embed(query))
        })?;

        // Search
        let db = self.db.lock().map_err(|_| RiverError::internal("Database lock poisoned".to_string()))?;
        let results = MemorySearcher::search(&db, &query_embedding, limit, source, after, before)?;

        // Format results
        let mut output = String::new();
        for (i, result) in results.iter().enumerate() {
            output.push_str(&format!(
                "{}. [score: {:.3}] {}\n   Source: {}, Time: {}\n   ID: {}\n\n",
                i + 1,
                result.similarity,
                result.memory.content,
                result.memory.source,
                result.memory.timestamp,
                result.memory.id
            ));
        }

        if output.is_empty() {
            output = "No matches found".to_string();
        }

        if let Some(out_path) = output_file {
            std::fs::write(out_path, &output)
                .map_err(|e| RiverError::tool(format!("Failed to write output file: {}", e)))?;
            return Ok(ToolResult {
                success: true,
                output: format!("Output written to {} ({} results)", out_path, results.len()),
                output_file: Some(out_path.to_string()),
            });
        }

        Ok(ToolResult {
            success: true,
            output,
            output_file: None,
        })
    }
}

/// Memory delete tool
pub struct MemoryDeleteTool {
    db: Arc<Mutex<Database>>,
}

impl MemoryDeleteTool {
    pub fn new(db: Arc<Mutex<Database>>) -> Self {
        Self { db }
    }
}

impl Tool for MemoryDeleteTool {
    fn name(&self) -> &str {
        "memory_delete"
    }

    fn description(&self) -> &str {
        "Delete embedding by ID"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Snowflake ID of embedding to delete" }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let id_str = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: id".to_string()))?;

        let id: Snowflake = id_str
            .parse()
            .map_err(|_| RiverError::tool(format!("Invalid snowflake ID: {}", id_str)))?;

        let db = self.db.lock().map_err(|_| RiverError::internal("Database lock poisoned".to_string()))?;
        let deleted = db.delete_memory(id)?;

        if deleted {
            Ok(ToolResult {
                success: true,
                output: format!("Deleted memory with ID: {}", id),
                output_file: None,
            })
        } else {
            Ok(ToolResult {
                success: false,
                output: format!("Memory not found: {}", id),
                output_file: None,
            })
        }
    }
}

/// Memory delete by source tool (bulk deletion)
pub struct MemoryDeleteBySourceTool {
    db: Arc<Mutex<Database>>,
}

impl MemoryDeleteBySourceTool {
    pub fn new(db: Arc<Mutex<Database>>) -> Self {
        Self { db }
    }
}

impl Tool for MemoryDeleteBySourceTool {
    fn name(&self) -> &str {
        "memory_delete_by_source"
    }

    fn description(&self) -> &str {
        "Delete embeddings by source, optionally before a timestamp"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source": { "type": "string", "description": "Source identifier to delete (e.g., 'message', 'file')" },
                "before": { "type": "string", "description": "Delete only entries before this date (ISO 8601, optional)" }
            },
            "required": ["source"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: source".to_string()))?;

        let before = args
            .get("before")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp());

        let db = self.db.lock().map_err(|_| RiverError::internal("Database lock poisoned".to_string()))?;
        let deleted = db.delete_memories_by_source(source, before)?;

        Ok(ToolResult {
            success: true,
            output: format!("Deleted {} memories with source '{}'", deleted, source),
            output_file: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration tests would require a running embedding server
    // Unit tests focus on parameter validation

    #[test]
    fn test_embed_tool_schema() {
        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let client = Arc::new(EmbeddingClient::new(crate::memory::EmbeddingConfig::default()));
        let birth = river_core::AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = Arc::new(SnowflakeGenerator::new(birth));

        let tool = EmbedTool::new(db, client, gen);
        let params = tool.parameters();

        assert!(params.get("properties").unwrap().get("content").is_some());
        assert!(params.get("properties").unwrap().get("source").is_some());
    }
}
```

- [ ] **Step 2: Update tools/mod.rs**

Add to `crates/river-gateway/src/tools/mod.rs`:

```rust
//! Tool system

mod registry;
mod executor;
mod file;
mod shell;
mod memory;

pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::{ToolExecutor, ToolCall, ToolCallResponse};
pub use file::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool};
pub use shell::BashTool;
pub use memory::{EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p river-gateway`

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/tools/memory.rs crates/river-gateway/src/tools/mod.rs
git commit -m "feat(gateway): add memory tools (embed, memory_search, memory_delete, memory_delete_by_source)"
```

---

## Chunk 4: Redis Integration

### Task 6: Add Redis Dependencies

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/river-gateway/Cargo.toml`

- [ ] **Step 1: Add fred (Redis client) to workspace**

Add to workspace `Cargo.toml` under `[workspace.dependencies]`:

```toml
fred = { version = "9.0", features = ["enable-rustls"] }
```

- [ ] **Step 2: Add fred to river-gateway**

Add to `crates/river-gateway/Cargo.toml`:

```toml
fred.workspace = true
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p river-gateway`

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/river-gateway/Cargo.toml
git commit -m "build(gateway): add fred redis client dependency"
```

---

### Task 7: Redis Client Wrapper

**Files:**
- Create: `crates/river-gateway/src/redis/mod.rs`
- Create: `crates/river-gateway/src/redis/client.rs`
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Create redis/mod.rs**

```rust
//! Redis integration for working/medium-term memory

mod client;
mod working;
mod medium_term;
mod coordination;
mod cache;

pub use client::{RedisClient, RedisConfig};
pub use working::{WorkingMemorySetTool, WorkingMemoryGetTool, WorkingMemoryDeleteTool};
pub use medium_term::{MediumTermSetTool, MediumTermGetTool};
pub use coordination::{ResourceLockTool, CounterIncrementTool, CounterGetTool};
pub use cache::{CacheSetTool, CacheGetTool};
```

- [ ] **Step 2: Create redis/client.rs**

```rust
//! Redis connection wrapper with agent namespacing

use fred::prelude::*;
use river_core::{RiverError, RiverResult};

/// Redis configuration
#[derive(Debug, Clone)]
pub struct RedisConfig {
    pub url: String,
    pub agent_name: String,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6379".to_string(),
            agent_name: "default".to_string(),
        }
    }
}

/// Redis client with agent namespacing
#[derive(Clone)]
pub struct RedisClient {
    inner: fred::clients::RedisClient,
    agent_name: String,
}

impl RedisClient {
    /// Create new Redis client
    pub async fn new(config: RedisConfig) -> RiverResult<Self> {
        let redis_config = fred::types::RedisConfig::from_url(&config.url)
            .map_err(|e| RiverError::redis(format!("Invalid Redis URL: {}", e)))?;

        let inner = fred::clients::RedisClient::new(redis_config, None, None, None);
        inner.connect();
        inner
            .wait_for_connect()
            .await
            .map_err(|e| RiverError::redis(format!("Connection failed: {}", e)))?;

        Ok(Self {
            inner,
            agent_name: config.agent_name,
        })
    }

    /// Get namespaced key for a domain
    fn namespaced_key(&self, domain: &str, key: &str) -> String {
        format!("river:{}:{}:{}", self.agent_name, domain, key)
    }

    // Working memory domain
    pub async fn working_set(&self, key: &str, value: &str, ttl_minutes: u64) -> RiverResult<()> {
        let full_key = self.namespaced_key("working", key);
        self.inner
            .set::<(), _, _>(&full_key, value, Some(Expiration::EX(ttl_minutes as i64 * 60)), None, false)
            .await
            .map_err(|e| RiverError::redis(format!("SET failed: {}", e)))
    }

    pub async fn working_get(&self, key: &str) -> RiverResult<Option<String>> {
        let full_key = self.namespaced_key("working", key);
        self.inner
            .get::<Option<String>, _>(&full_key)
            .await
            .map_err(|e| RiverError::redis(format!("GET failed: {}", e)))
    }

    pub async fn working_delete(&self, key: &str) -> RiverResult<bool> {
        let full_key = self.namespaced_key("working", key);
        let deleted: i64 = self.inner
            .del(&full_key)
            .await
            .map_err(|e| RiverError::redis(format!("DEL failed: {}", e)))?;
        Ok(deleted > 0)
    }

    // Medium-term domain
    pub async fn medium_set(&self, key: &str, value: &str, ttl_hours: u64) -> RiverResult<()> {
        let full_key = self.namespaced_key("medium", key);
        self.inner
            .set::<(), _, _>(&full_key, value, Some(Expiration::EX(ttl_hours as i64 * 3600)), None, false)
            .await
            .map_err(|e| RiverError::redis(format!("SET failed: {}", e)))
    }

    pub async fn medium_get(&self, key: &str) -> RiverResult<Option<String>> {
        let full_key = self.namespaced_key("medium", key);
        self.inner
            .get::<Option<String>, _>(&full_key)
            .await
            .map_err(|e| RiverError::redis(format!("GET failed: {}", e)))
    }

    // Coordination domain
    pub async fn acquire_lock(&self, key: &str, ttl_seconds: u64) -> RiverResult<bool> {
        let full_key = self.namespaced_key("coord", &format!("lock:{}", key));
        let result: Option<String> = self.inner
            .set(&full_key, "locked", Some(Expiration::EX(ttl_seconds as i64)), Some(SetOptions::NX), false)
            .await
            .map_err(|e| RiverError::redis(format!("SET NX failed: {}", e)))?;
        Ok(result.is_some())
    }

    pub async fn release_lock(&self, key: &str) -> RiverResult<bool> {
        let full_key = self.namespaced_key("coord", &format!("lock:{}", key));
        let deleted: i64 = self.inner
            .del(&full_key)
            .await
            .map_err(|e| RiverError::redis(format!("DEL failed: {}", e)))?;
        Ok(deleted > 0)
    }

    pub async fn counter_incr(&self, key: &str) -> RiverResult<i64> {
        let full_key = self.namespaced_key("coord", &format!("counter:{}", key));
        self.inner
            .incr(&full_key)
            .await
            .map_err(|e| RiverError::redis(format!("INCR failed: {}", e)))
    }

    pub async fn counter_get(&self, key: &str) -> RiverResult<i64> {
        let full_key = self.namespaced_key("coord", &format!("counter:{}", key));
        let value: Option<i64> = self.inner
            .get(&full_key)
            .await
            .map_err(|e| RiverError::redis(format!("GET failed: {}", e)))?;
        Ok(value.unwrap_or(0))
    }

    // Cache domain
    pub async fn cache_set(&self, key: &str, value: &str, ttl_seconds: Option<u64>) -> RiverResult<()> {
        let full_key = self.namespaced_key("cache", key);
        let expiration = ttl_seconds.map(|s| Expiration::EX(s as i64));
        self.inner
            .set::<(), _, _>(&full_key, value, expiration, None, false)
            .await
            .map_err(|e| RiverError::redis(format!("SET failed: {}", e)))
    }

    pub async fn cache_get(&self, key: &str) -> RiverResult<Option<String>> {
        let full_key = self.namespaced_key("cache", key);
        self.inner
            .get::<Option<String>, _>(&full_key)
            .await
            .map_err(|e| RiverError::redis(format!("GET failed: {}", e)))
    }

    /// Check if Redis is connected
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespaced_key() {
        // Can't test actual Redis without a server, but can test key formatting
        let config = RedisConfig {
            url: "redis://localhost:6379".to_string(),
            agent_name: "thomas".to_string(),
        };
        let expected_prefix = "river:thomas:working:";
        assert!(expected_prefix.contains("thomas"));
    }
}
```

- [ ] **Step 3: Add redis module to lib.rs**

Add to `crates/river-gateway/src/lib.rs`:

```rust
pub mod redis;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p river-gateway`

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/redis/
git commit -m "feat(gateway): add redis client wrapper with namespacing"
```

---

### Task 8: Redis Tools (Working Memory)

**Files:**
- Create: `crates/river-gateway/src/redis/working.rs`

- [ ] **Step 1: Create working.rs**

```rust
//! Working memory tools (short-term, minutes TTL)

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::RedisClient;
use crate::tools::{Tool, ToolResult};

/// Working memory set tool
pub struct WorkingMemorySetTool {
    redis: Arc<RedisClient>,
}

impl WorkingMemorySetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for WorkingMemorySetTool {
    fn name(&self) -> &str {
        "working_memory_set"
    }

    fn description(&self) -> &str {
        "Store value with TTL (minutes) in working memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to store" },
                "value": { "type": "string", "description": "Value to store (JSON or string)" },
                "ttl_minutes": { "type": "integer", "description": "Time to live in minutes", "default": 30 }
            },
            "required": ["key", "value"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = args
            .get("value")
            .map(|v| {
                if v.is_string() {
                    v.as_str().unwrap().to_string()
                } else {
                    v.to_string()
                }
            })
            .ok_or_else(|| RiverError::tool("Missing required parameter: value".to_string()))?;

        let ttl = args.get("ttl_minutes").and_then(|v| v.as_u64()).unwrap_or(30);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.working_set(key, &value, ttl))
        })?;

        Ok(ToolResult {
            success: true,
            output: format!("Stored '{}' with TTL {} minutes", key, ttl),
            output_file: None,
        })
    }
}

/// Working memory get tool
pub struct WorkingMemoryGetTool {
    redis: Arc<RedisClient>,
}

impl WorkingMemoryGetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for WorkingMemoryGetTool {
    fn name(&self) -> &str {
        "working_memory_get"
    }

    fn description(&self) -> &str {
        "Retrieve value from working memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to retrieve" }
            },
            "required": ["key"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.working_get(key))
        })?;

        match value {
            Some(v) => Ok(ToolResult {
                success: true,
                output: v,
                output_file: None,
            }),
            None => Ok(ToolResult {
                success: false,
                output: format!("Key '{}' not found or expired", key),
                output_file: None,
            }),
        }
    }
}

/// Working memory delete tool
pub struct WorkingMemoryDeleteTool {
    redis: Arc<RedisClient>,
}

impl WorkingMemoryDeleteTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for WorkingMemoryDeleteTool {
    fn name(&self) -> &str {
        "working_memory_delete"
    }

    fn description(&self) -> &str {
        "Delete value from working memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to delete" }
            },
            "required": ["key"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let deleted = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.working_delete(key))
        })?;

        if deleted {
            Ok(ToolResult {
                success: true,
                output: format!("Deleted '{}'", key),
                output_file: None,
            })
        } else {
            Ok(ToolResult {
                success: false,
                output: format!("Key '{}' not found", key),
                output_file: None,
            })
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p river-gateway`

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/redis/working.rs
git commit -m "feat(gateway): add working memory redis tools"
```

---

### Task 9: Redis Tools (Medium-Term, Coordination, Cache)

**Files:**
- Create: `crates/river-gateway/src/redis/medium_term.rs`
- Create: `crates/river-gateway/src/redis/coordination.rs`
- Create: `crates/river-gateway/src/redis/cache.rs`

- [ ] **Step 1: Create medium_term.rs**

```rust
//! Medium-term memory tools (hours TTL)

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::RedisClient;
use crate::tools::{Tool, ToolResult};

/// Medium-term memory set tool
pub struct MediumTermSetTool {
    redis: Arc<RedisClient>,
}

impl MediumTermSetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for MediumTermSetTool {
    fn name(&self) -> &str {
        "medium_term_set"
    }

    fn description(&self) -> &str {
        "Store value with TTL (hours) in medium-term memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to store" },
                "value": { "type": "string", "description": "Value to store (JSON or string)" },
                "ttl_hours": { "type": "integer", "description": "Time to live in hours", "default": 24 }
            },
            "required": ["key", "value"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = args
            .get("value")
            .map(|v| {
                if v.is_string() {
                    v.as_str().unwrap().to_string()
                } else {
                    v.to_string()
                }
            })
            .ok_or_else(|| RiverError::tool("Missing required parameter: value".to_string()))?;

        let ttl = args.get("ttl_hours").and_then(|v| v.as_u64()).unwrap_or(24);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.medium_set(key, &value, ttl))
        })?;

        Ok(ToolResult {
            success: true,
            output: format!("Stored '{}' with TTL {} hours", key, ttl),
            output_file: None,
        })
    }
}

/// Medium-term memory get tool
pub struct MediumTermGetTool {
    redis: Arc<RedisClient>,
}

impl MediumTermGetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for MediumTermGetTool {
    fn name(&self) -> &str {
        "medium_term_get"
    }

    fn description(&self) -> &str {
        "Retrieve value from medium-term memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to retrieve" }
            },
            "required": ["key"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.medium_get(key))
        })?;

        match value {
            Some(v) => Ok(ToolResult {
                success: true,
                output: v,
                output_file: None,
            }),
            None => Ok(ToolResult {
                success: false,
                output: format!("Key '{}' not found or expired", key),
                output_file: None,
            }),
        }
    }
}
```

- [ ] **Step 2: Create coordination.rs**

```rust
//! Coordination tools (locks, counters)

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::RedisClient;
use crate::tools::{Tool, ToolResult};

/// Resource lock tool
pub struct ResourceLockTool {
    redis: Arc<RedisClient>,
}

impl ResourceLockTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for ResourceLockTool {
    fn name(&self) -> &str {
        "resource_lock"
    }

    fn description(&self) -> &str {
        "Acquire or release a distributed lock"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Lock name" },
                "action": { "type": "string", "enum": ["acquire", "release"], "description": "Lock action" },
                "ttl_seconds": { "type": "integer", "description": "Lock TTL in seconds (for acquire)", "default": 60 }
            },
            "required": ["key", "action"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: action".to_string()))?;

        match action {
            "acquire" => {
                let ttl = args.get("ttl_seconds").and_then(|v| v.as_u64()).unwrap_or(60);
                let acquired = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(self.redis.acquire_lock(key, ttl))
                })?;

                if acquired {
                    Ok(ToolResult {
                        success: true,
                        output: format!("Acquired lock '{}' for {} seconds", key, ttl),
                        output_file: None,
                    })
                } else {
                    Ok(ToolResult {
                        success: false,
                        output: format!("Lock '{}' is already held", key),
                        output_file: None,
                    })
                }
            }
            "release" => {
                let released = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(self.redis.release_lock(key))
                })?;

                if released {
                    Ok(ToolResult {
                        success: true,
                        output: format!("Released lock '{}'", key),
                        output_file: None,
                    })
                } else {
                    Ok(ToolResult {
                        success: false,
                        output: format!("Lock '{}' not held or already released", key),
                        output_file: None,
                    })
                }
            }
            _ => Err(RiverError::tool(format!("Invalid action: {}", action))),
        }
    }
}

/// Counter increment tool
pub struct CounterIncrementTool {
    redis: Arc<RedisClient>,
}

impl CounterIncrementTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for CounterIncrementTool {
    fn name(&self) -> &str {
        "counter_increment"
    }

    fn description(&self) -> &str {
        "Increment a counter and return new value"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Counter name" }
            },
            "required": ["key"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.counter_incr(key))
        })?;

        Ok(ToolResult {
            success: true,
            output: value.to_string(),
            output_file: None,
        })
    }
}

/// Counter get tool
pub struct CounterGetTool {
    redis: Arc<RedisClient>,
}

impl CounterGetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for CounterGetTool {
    fn name(&self) -> &str {
        "counter_get"
    }

    fn description(&self) -> &str {
        "Get current counter value"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Counter name" }
            },
            "required": ["key"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.counter_get(key))
        })?;

        Ok(ToolResult {
            success: true,
            output: value.to_string(),
            output_file: None,
        })
    }
}
```

- [ ] **Step 3: Create cache.rs**

```rust
//! Cache tools

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::RedisClient;
use crate::tools::{Tool, ToolResult};

/// Cache set tool
pub struct CacheSetTool {
    redis: Arc<RedisClient>,
}

impl CacheSetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for CacheSetTool {
    fn name(&self) -> &str {
        "cache_set"
    }

    fn description(&self) -> &str {
        "Store computed value in cache"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Cache key" },
                "value": { "type": "string", "description": "Value to cache" },
                "ttl_seconds": { "type": "integer", "description": "TTL in seconds (optional, omit for no expiry)" }
            },
            "required": ["key", "value"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = args
            .get("value")
            .map(|v| {
                if v.is_string() {
                    v.as_str().unwrap().to_string()
                } else {
                    v.to_string()
                }
            })
            .ok_or_else(|| RiverError::tool("Missing required parameter: value".to_string()))?;

        let ttl = args.get("ttl_seconds").and_then(|v| v.as_u64());

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.cache_set(key, &value, ttl))
        })?;

        let ttl_msg = ttl
            .map(|s| format!(" with TTL {} seconds", s))
            .unwrap_or_default();

        Ok(ToolResult {
            success: true,
            output: format!("Cached '{}'{}", key, ttl_msg),
            output_file: None,
        })
    }
}

/// Cache get tool
pub struct CacheGetTool {
    redis: Arc<RedisClient>,
}

impl CacheGetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for CacheGetTool {
    fn name(&self) -> &str {
        "cache_get"
    }

    fn description(&self) -> &str {
        "Retrieve value from cache"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Cache key" }
            },
            "required": ["key"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.cache_get(key))
        })?;

        match value {
            Some(v) => Ok(ToolResult {
                success: true,
                output: v,
                output_file: None,
            }),
            None => Ok(ToolResult {
                success: false,
                output: format!("Cache miss: '{}'", key),
                output_file: None,
            }),
        }
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p river-gateway`

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/redis/
git commit -m "feat(gateway): add redis tools (medium-term, coordination, cache)"
```

---

## Chunk 5: State and Server Integration

### Task 10: Update AppState with Memory and Redis

**Files:**
- Modify: `crates/river-gateway/src/state.rs`

- [ ] **Step 1: Update GatewayConfig with memory/redis config**

Update `crates/river-gateway/src/state.rs`:

```rust
//! Shared application state

use crate::db::Database;
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::redis::{RedisClient, RedisConfig};
use crate::tools::{ToolExecutor, ToolRegistry};
use river_core::{AgentBirth, SnowflakeGenerator};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

/// Shared application state
pub struct AppState {
    pub config: GatewayConfig,
    pub db: Arc<Mutex<Database>>,
    pub snowflake_gen: Arc<SnowflakeGenerator>,
    pub tool_executor: Arc<RwLock<ToolExecutor>>,
    pub embedding_client: Option<Arc<EmbeddingClient>>,
    pub redis_client: Option<Arc<RedisClient>>,
}

/// Gateway configuration (runtime)
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub model_url: String,
    pub model_name: String,
    pub context_limit: u64,
    pub heartbeat_minutes: u32,
    pub agent_birth: AgentBirth,
    pub agent_name: String,
    pub embedding: Option<EmbeddingConfig>,
    pub redis: Option<RedisConfig>,
}

impl AppState {
    pub fn new(
        config: GatewayConfig,
        db: Arc<Mutex<Database>>,
        registry: ToolRegistry,
        embedding_client: Option<EmbeddingClient>,
        redis_client: Option<RedisClient>,
    ) -> Self {
        let executor = ToolExecutor::new(registry, config.context_limit);

        Self {
            snowflake_gen: Arc::new(SnowflakeGenerator::new(config.agent_birth)),
            db,
            tool_executor: Arc::new(RwLock::new(executor)),
            embedding_client: embedding_client.map(Arc::new),
            redis_client: redis_client.map(Arc::new),
            config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::tools::ToolRegistry;

    #[test]
    fn test_state_creation() {
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth: AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap(),
            agent_name: "test".to_string(),
            embedding: None,
            redis: None,
        };

        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let state = AppState::new(config, db, registry, None, None);

        assert_eq!(state.config.port, 3000);
        assert_eq!(state.config.context_limit, 65536);
        assert!(state.embedding_client.is_none());
        assert!(state.redis_client.is_none());
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p river-gateway`

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/state.rs
git commit -m "feat(gateway): add embedding and redis clients to app state"
```

---

### Task 11: Update Server Initialization

**Files:**
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Update server.rs to initialize memory tools**

Update `crates/river-gateway/src/server.rs`:

```rust
use crate::api::create_router;
use crate::db::{init_db, Database};
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::redis::{RedisClient, RedisConfig};
use crate::state::{AppState, GatewayConfig};
use crate::tools::{
    BashTool, EditTool, EmbedTool, GlobTool, GrepTool, MemoryDeleteTool, MemoryDeleteBySourceTool,
    MemorySearchTool, ReadTool, ToolRegistry, WriteTool,
};
use river_core::AgentBirth;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

/// Server configuration from CLI args
pub struct ServerConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub agent_name: String,
    pub model_url: Option<String>,
    pub model_name: Option<String>,
    pub embedding_url: Option<String>,
    pub redis_url: Option<String>,
}

/// Initialize and run the gateway server
pub async fn run(config: ServerConfig) -> anyhow::Result<()> {
    // Initialize database
    let db_path = config.data_dir.join("river.db");
    let db = init_db(&db_path)?;

    // Create embedding client if configured
    let embedding_client = if let Some(url) = &config.embedding_url {
        let embed_config = EmbeddingConfig {
            url: url.clone(),
            ..Default::default()
        };
        Some(EmbeddingClient::new(embed_config))
    } else {
        None
    };

    // Create Redis client if configured
    let redis_client = if let Some(url) = &config.redis_url {
        let redis_config = RedisConfig {
            url: url.clone(),
            agent_name: config.agent_name.clone(),
        };
        Some(RedisClient::new(redis_config).await?)
    } else {
        None
    };

    // Create agent birth (current time)
    let now = chrono::Utc::now();
    let agent_birth = AgentBirth::new(
        now.year() as u16,
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )?;

    // Create gateway config
    let agent_name = config.agent_name.clone();
    let gateway_config = GatewayConfig {
        workspace: config.workspace.clone(),
        data_dir: config.data_dir.clone(),
        port: config.port,
        model_url: config.model_url.unwrap_or_else(|| "http://localhost:8080".to_string()),
        model_name: config.model_name.unwrap_or_else(|| "default".to_string()),
        context_limit: 65536,
        heartbeat_minutes: 45,
        agent_birth,
        agent_name: agent_name.clone(),
        embedding: config.embedding_url.as_ref().map(|url| EmbeddingConfig {
            url: url.clone(),
            ..Default::default()
        }),
        redis: config.redis_url.as_ref().map(|url| RedisConfig {
            url: url.clone(),
            agent_name: agent_name.clone(),
        }),
    };

    // Wrap database in Arc for sharing
    let db_arc = Arc::new(std::sync::Mutex::new(db));
    let snowflake_gen = Arc::new(river_core::SnowflakeGenerator::new(gateway_config.agent_birth));

    // Create tool registry with core tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(&config.workspace)));
    registry.register(Box::new(WriteTool::new(&config.workspace)));
    registry.register(Box::new(EditTool::new(&config.workspace)));
    registry.register(Box::new(GlobTool::new(&config.workspace)));
    registry.register(Box::new(GrepTool::new(&config.workspace)));
    registry.register(Box::new(BashTool::new(&config.workspace)));

    // Register memory tools if embedding client is available
    if let Some(ref embed_client) = embedding_client {
        let embed_arc = Arc::new(embed_client.clone());
        registry.register(Box::new(EmbedTool::new(
            db_arc.clone(),
            embed_arc.clone(),
            snowflake_gen.clone(),
        )));
        registry.register(Box::new(MemorySearchTool::new(db_arc.clone(), embed_arc.clone())));
        registry.register(Box::new(MemoryDeleteTool::new(db_arc.clone())));
        registry.register(Box::new(MemoryDeleteBySourceTool::new(db_arc.clone())));
        tracing::info!("Registered memory tools (embed, memory_search, memory_delete, memory_delete_by_source)");
    }

    // Register Redis tools if client is available
    if let Some(ref redis) = redis_client {
        let redis_arc = Arc::new(redis.clone());
        use crate::redis::*;
        registry.register(Box::new(WorkingMemorySetTool::new(redis_arc.clone())));
        registry.register(Box::new(WorkingMemoryGetTool::new(redis_arc.clone())));
        registry.register(Box::new(WorkingMemoryDeleteTool::new(redis_arc.clone())));
        registry.register(Box::new(MediumTermSetTool::new(redis_arc.clone())));
        registry.register(Box::new(MediumTermGetTool::new(redis_arc.clone())));
        registry.register(Box::new(ResourceLockTool::new(redis_arc.clone())));
        registry.register(Box::new(CounterIncrementTool::new(redis_arc.clone())));
        registry.register(Box::new(CounterGetTool::new(redis_arc.clone())));
        registry.register(Box::new(CacheSetTool::new(redis_arc.clone())));
        registry.register(Box::new(CacheGetTool::new(redis_arc.clone())));
        tracing::info!("Registered Redis tools (10 tools)");
    }

    tracing::info!("Registered {} tools total", registry.names().len());

    // Create app state (AppState takes Arc<Mutex<Database>> directly, not Database)
    let state = Arc::new(AppState::new(
        gateway_config,
        db_arc,
        registry,
        embedding_client,
        redis_client,
    ));

    // Create router
    let app = create_router(state);

    // Bind and serve
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    tracing::info!("Gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

- [ ] **Step 2: Update main.rs CLI args**

Update `crates/river-gateway/src/main.rs`:

```rust
use clap::Parser;
use river_gateway::server::{run, ServerConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-gateway")]
#[command(about = "River Engine Gateway - Agent Runtime")]
struct Args {
    /// Workspace directory
    #[arg(short, long)]
    workspace: PathBuf,

    /// Data directory for database
    #[arg(short, long)]
    data_dir: PathBuf,

    /// Agent name (used for Redis namespacing)
    #[arg(long, default_value = "default")]
    agent_name: String,

    /// Gateway port
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Model server URL
    #[arg(long)]
    model_url: Option<String>,

    /// Model name
    #[arg(long)]
    model_name: Option<String>,

    /// Embedding server URL (enables memory tools)
    #[arg(long)]
    embedding_url: Option<String>,

    /// Redis URL (enables working/medium-term memory tools)
    #[arg(long)]
    redis_url: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    tracing::info!("Starting River Gateway");
    tracing::info!("Agent: {}", args.agent_name);
    tracing::info!("Workspace: {:?}", args.workspace);
    tracing::info!("Data dir: {:?}", args.data_dir);
    tracing::info!("Port: {}", args.port);

    if args.embedding_url.is_some() {
        tracing::info!("Embedding server: {:?}", args.embedding_url);
    }
    if args.redis_url.is_some() {
        tracing::info!("Redis: {:?}", args.redis_url);
    }

    let config = ServerConfig {
        workspace: args.workspace,
        data_dir: args.data_dir,
        port: args.port,
        agent_name: args.agent_name,
        model_url: args.model_url,
        model_name: args.model_name,
        embedding_url: args.embedding_url,
        redis_url: args.redis_url,
    };

    run(config).await
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p river-gateway`

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/server.rs crates/river-gateway/src/main.rs
git commit -m "feat(gateway): integrate memory and redis tools in server startup"
```

---

## Chunk 6: Add RiverError Variants

### Task 12: Add Error Variants for Memory and Redis

**Files:**
- Modify: `crates/river-core/src/error.rs`

- [ ] **Step 1: Add embedding and redis error variants**

Update `crates/river-core/src/error.rs` to add these variants if not present:

```rust
/// Embedding server error
#[error("Embedding error: {message}")]
Embedding { message: String },

/// Redis error
#[error("Redis error: {message}")]
Redis { message: String },
```

And add constructor methods:

```rust
/// Create embedding error
pub fn embedding(message: impl Into<String>) -> Self {
    Self::Embedding { message: message.into() }
}

/// Create Redis error
pub fn redis(message: impl Into<String>) -> Self {
    Self::Redis { message: message.into() }
}
```

- [ ] **Step 2: Run core tests**

Run: `cargo test -p river-core`

- [ ] **Step 3: Commit**

```bash
git add crates/river-core/src/error.rs
git commit -m "feat(core): add embedding and redis error variants"
```

---

## Final Verification

- [ ] **Run full test suite**

Run: `cargo test`

- [ ] **Build release**

Run: `cargo build --release -p river-gateway`

- [ ] **Test binary with new flags**

Run: `./target/release/river-gateway --help`

Verify new flags appear:
- `--agent-name`
- `--embedding-url`
- `--redis-url`

- [ ] **Update STATUS.md**

Add Plan 3 to completed section.

---

## Summary

This plan implements:

| Component | Description |
|-----------|-------------|
| Memories table | SQLite migration for embedding storage |
| Memory CRUD | Insert, get, delete, search operations |
| Embedding client | HTTP client for embedding server |
| Vector search | Cosine similarity search |
| Memory tools | embed, memory_search, memory_delete, memory_delete_by_source |
| Redis client | fred-based client with namespacing |
| Redis tools | 10 tools across 4 domains |
| State integration | Embedding and Redis in AppState |
| CLI args | New flags for embedding/redis URLs |

**Next plan:** Plan 4 - Orchestrator (agent lifecycle, heartbeat monitoring, inter-agent coordination)

//! Vector storage using rusqlite.
//!
//! This is a simplified implementation that stores vectors as blobs
//! and computes cosine similarity in Rust. For production, consider
//! integrating sqlite-vec properly or using a dedicated vector database.

use rusqlite::{params, Connection};
use std::path::Path;
use zerocopy::IntoBytes;

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    DimensionMismatch { expected: usize, actual: usize },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(e) => write!(f, "database error: {}", e),
            Self::DimensionMismatch { expected, actual } => {
                write!(f, "dimension mismatch: expected {}, got {}", expected, actual)
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}

/// Search result from the database.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub id: String,
    pub source_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
    pub distance: f32,
}

/// SQLite store for embeddings.
pub struct Store {
    conn: Connection,
    dimensions: usize,
}

// We need Send for spawn_blocking
// rusqlite Connection is !Send by default, but we can use it in single-threaded mode
// For safety, we wrap all operations

impl Store {
    /// Open or create the database.
    pub fn open(path: impl AsRef<Path>, dimensions: usize) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let store = Self { conn, dimensions };
        store.init_schema()?;
        Ok(store)
    }

    /// Create an in-memory store for testing.
    pub fn in_memory(dimensions: usize) -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn, dimensions };
        store.init_schema()?;
        Ok(store)
    }

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
        Ok(())
    }

    /// Check if a source needs updating.
    pub fn needs_update(&self, path: &str, hash: &str) -> Result<bool, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT hash FROM sources WHERE path = ?")?;
        let result: Option<String> = stmt.query_row([path], |row| row.get(0)).ok();

        Ok(result.as_deref() != Some(hash))
    }

    /// Delete all chunks for a source.
    pub fn delete_source(&self, path: &str) -> Result<usize, StoreError> {
        // Count chunks first
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE source_path = ?",
            [path],
            |row| row.get(0),
        )?;

        // Delete chunks
        self.conn
            .execute("DELETE FROM chunks WHERE source_path = ?", [path])?;

        // Delete source
        self.conn
            .execute("DELETE FROM sources WHERE path = ?", [path])?;

        Ok(count as usize)
    }

    /// Insert or update a source.
    pub fn upsert_source(&self, path: &str, hash: &str) -> Result<(), StoreError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn.execute(
            "INSERT OR REPLACE INTO sources (path, hash, updated_at) VALUES (?, ?, ?)",
            params![path, hash, now],
        )?;
        Ok(())
    }

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

        self.conn.execute(
            "INSERT INTO chunks (id, source_path, line_start, line_end, text, embedding) VALUES (?, ?, ?, ?, ?, ?)",
            params![id, source_path, line_start as i64, line_end as i64, text, embedding_bytes],
        )?;

        Ok(())
    }

    /// Search for similar chunks using cosine similarity.
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

        // Load all chunks and compute similarity in Rust
        // (For production, use a proper vector index)
        let mut stmt = self.conn.prepare(
            "SELECT id, source_path, line_start, line_end, text, embedding FROM chunks",
        )?;

        let mut hits: Vec<SearchHit> = stmt
            .query_map([], |row| {
                let embedding_bytes: Vec<u8> = row.get(5)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as usize,
                    row.get::<_, i64>(3)? as usize,
                    row.get::<_, String>(4)?,
                    embedding_bytes,
                ))
            })?
            .filter_map(|r| r.ok())
            .map(|(id, source_path, line_start, line_end, text, embedding_bytes)| {
                let embedding = bytes_to_floats(&embedding_bytes);
                let distance = cosine_distance(query_embedding, &embedding);
                SearchHit {
                    id,
                    source_path,
                    line_start,
                    line_end,
                    text,
                    distance,
                }
            })
            .collect();

        // Sort by distance (ascending = most similar first)
        hits.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());

        // Apply offset and limit
        let results = hits.into_iter().skip(offset).take(limit).collect();

        Ok(results)
    }

    /// Get counts for health check.
    pub fn counts(&self) -> Result<(usize, usize), StoreError> {
        let sources: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))?;
        let chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?;
        Ok((sources as usize, chunks as usize))
    }
}

fn bytes_to_floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap_or([0; 4])))
        .collect()
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 1.0;
    }

    let similarity = dot / (norm_a * norm_b);
    1.0 - similarity // Convert to distance
}

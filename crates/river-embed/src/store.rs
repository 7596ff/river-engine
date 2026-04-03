//! Vector storage using rusqlite with sqlite-vec for efficient vector search.

use rusqlite::{ffi::sqlite3_auto_extension, params, Connection};
use std::path::Path;
use std::sync::Once;
use zerocopy::IntoBytes;

static SQLITE_VEC_INIT: Once = Once::new();

/// Initialize sqlite-vec extension (called once per process).
fn init_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| {
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

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
        // Initialize sqlite-vec extension (once per process)
        init_sqlite_vec();

        let conn = Connection::open(path)?;

        // Enable foreign keys
        conn.execute("PRAGMA foreign_keys = ON", [])?;

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

        // Create virtual table for vector search
        let create_vec_table = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_vec USING vec0(id TEXT PRIMARY KEY, embedding FLOAT[{}])",
            self.dimensions
        );
        self.conn.execute(&create_vec_table, [])?;

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

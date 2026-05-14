//! sqlite-vec vector store

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct VectorStore {
    conn: Arc<Mutex<Connection>>,
}

impl VectorStore {
    /// Open or create the vector database
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn =
            Connection::open(path).map_err(|e| format!("Failed to open vector DB: {}", e))?;

        conn.execute_batch(
            "
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
        ",
        )
        .map_err(|e| format!("Failed to create tables: {}", e))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open in-memory store (for testing)
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory DB: {}", e))?;

        conn.execute_batch(
            "
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
        ",
        )
        .map_err(|e| format!("Failed to create tables: {}", e))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
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
        let embedding_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

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
        let mut stmt = conn
            .prepare("SELECT hash FROM chunks WHERE source_path = ?1 LIMIT 1")
            .map_err(|e| e.to_string())?;

        let hash = stmt.query_row(params![source_path], |row| row.get(0)).ok();

        Ok(hash)
    }

    /// Delete all chunks for a source path
    pub fn delete_source(&self, source_path: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM chunks WHERE source_path = ?1",
            params![source_path],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Search by cosine similarity
    pub fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, source_path, content, chunk_type, channel, embedding FROM chunks WHERE embedding IS NOT NULL"
        ).map_err(|e| e.to_string())?;

        let mut results: Vec<SearchResult> = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let source_path: String = row.get(1)?;
                let content: String = row.get(2)?;
                let chunk_type: String = row.get(3)?;
                let channel: Option<String> = row.get(4)?;
                let embedding_bytes: Vec<u8> = row.get(5)?;

                let embedding: Vec<f32> = embedding_bytes
                    .chunks_exact(4)
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
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_and_get_hash() {
        let store = VectorStore::open_in_memory().unwrap();
        store
            .upsert_chunk(
                "test:0",
                "test.md",
                "content",
                "Note",
                None,
                "abc123",
                &[0.1, 0.2],
            )
            .unwrap();
        let hash = store.get_hash("test.md").unwrap();
        assert_eq!(hash, Some("abc123".to_string()));
    }

    #[test]
    fn test_delete_source() {
        let store = VectorStore::open_in_memory().unwrap();
        store
            .upsert_chunk("test:0", "test.md", "content", "Note", None, "abc", &[0.1])
            .unwrap();
        store.delete_source("test.md").unwrap();
        let hash = store.get_hash("test.md").unwrap();
        assert!(hash.is_none());
    }

    #[test]
    fn test_cosine_similarity() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 0.001);
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_search() {
        let store = VectorStore::open_in_memory().unwrap();
        store
            .upsert_chunk("a:0", "a.md", "content a", "Note", None, "h1", &[1.0, 0.0])
            .unwrap();
        store
            .upsert_chunk("b:0", "b.md", "content b", "Note", None, "h2", &[0.0, 1.0])
            .unwrap();

        let results = store.search(&[1.0, 0.0], 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source_path, "a.md"); // Most similar
    }
}

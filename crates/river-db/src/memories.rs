//! Memory CRUD operations for semantic search

use river_core::{RiverError, RiverResult, Snowflake};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use crate::schema::Database;

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

        match rows
            .next()
            .map_err(|e| RiverError::database(e.to_string()))?
        {
            Some(row) => Ok(Some(
                Memory::from_row(row).map_err(|e| RiverError::database(e.to_string()))?,
            )),
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
    pub fn delete_memories_by_source(
        &self,
        source: &str,
        before: Option<i64>,
    ) -> RiverResult<usize> {
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
            .prepare(
                "SELECT id, embedding FROM memories WHERE expires_at IS NULL OR expires_at > ?",
            )
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
                Ok((
                    Snowflake::from_bytes(id_array),
                    bytes_to_f32_vec(&embedding_bytes),
                ))
            })
            .map_err(|e| RiverError::database(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))
    }

    /// Get the birth memory (first memory, source = "system:birth")
    /// Returns the memory if it exists, which encodes the AgentBirth in its Snowflake ID
    pub fn get_birth_memory(&self) -> RiverResult<Option<Memory>> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, content, embedding, source, timestamp, expires_at, metadata
                 FROM memories WHERE source = 'system:birth' LIMIT 1",
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        let mut rows = stmt
            .query([])
            .map_err(|e| RiverError::database(e.to_string()))?;

        match rows
            .next()
            .map_err(|e| RiverError::database(e.to_string()))?
        {
            Some(row) => Ok(Some(
                Memory::from_row(row).map_err(|e| RiverError::database(e.to_string()))?,
            )),
            None => Ok(None),
        }
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
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params.iter().map(|p| p as &dyn rusqlite::ToSql).collect();

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

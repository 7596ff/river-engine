//! Context CRUD operations for conversation persistence

use river_core::{RiverError, RiverResult, Snowflake};
use rusqlite::{params, Row};

use super::Database;

/// Context entry for conversation persistence
#[derive(Debug, Clone)]
pub struct Context {
    pub id: Snowflake,
    pub archived_at: Option<Snowflake>,
    pub token_count: Option<i64>,
    pub summary: Option<String>,
    pub blob: Option<Vec<u8>>,
}

impl Context {
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

        let archived_at = match row.get::<_, Option<Vec<u8>>>(1)? {
            Some(bytes) => {
                let array: [u8; 16] = bytes.try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Invalid archived_at snowflake length",
                        )),
                    )
                })?;
                Some(Snowflake::from_bytes(array))
            }
            None => None,
        };

        Ok(Self {
            id,
            archived_at,
            token_count: row.get(2)?,
            summary: row.get(3)?,
            blob: row.get(4)?,
        })
    }

    /// Check if context is active (not archived)
    pub fn is_active(&self) -> bool {
        self.blob.is_none()
    }
}

impl Database {
    /// Insert a new active context
    pub fn insert_context(&self, id: Snowflake) -> RiverResult<()> {
        self.conn()
            .execute(
                "INSERT INTO contexts (id, archived_at, token_count, summary, blob)
                 VALUES (?, NULL, NULL, NULL, NULL)",
                params![id.to_bytes().to_vec()],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
    }

    /// Get the latest context (highest ID)
    pub fn get_latest_context(&self) -> RiverResult<Option<Context>> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, archived_at, token_count, summary, blob
                 FROM contexts ORDER BY id DESC LIMIT 1",
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        let mut rows = stmt
            .query([])
            .map_err(|e| RiverError::database(e.to_string()))?;

        match rows.next().map_err(|e| RiverError::database(e.to_string()))? {
            Some(row) => Ok(Some(Context::from_row(row).map_err(|e| RiverError::database(e.to_string()))?)),
            None => Ok(None),
        }
    }

    /// Archive a context with metadata and JSONL blob
    pub fn archive_context(
        &self,
        id: Snowflake,
        archived_at: Snowflake,
        token_count: i64,
        summary: String,
        blob: Vec<u8>,
    ) -> RiverResult<()> {
        self.conn()
            .execute(
                "UPDATE contexts
                 SET archived_at = ?, token_count = ?, summary = ?, blob = ?
                 WHERE id = ?",
                params![
                    archived_at.to_bytes().to_vec(),
                    token_count,
                    summary,
                    blob,
                    id.to_bytes().to_vec(),
                ],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
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
    fn test_insert_and_get() {
        let (db, gen) = test_db_and_gen();

        let ctx_id = gen.next_id(SnowflakeType::Context);
        db.insert_context(ctx_id).unwrap();

        let retrieved = db.get_latest_context().unwrap().unwrap();
        assert_eq!(retrieved.id, ctx_id);
        assert!(retrieved.is_active());
        assert!(retrieved.archived_at.is_none());
        assert!(retrieved.token_count.is_none());
        assert!(retrieved.summary.is_none());
        assert!(retrieved.blob.is_none());
    }

    #[test]
    fn test_archive() {
        let (db, gen) = test_db_and_gen();

        let ctx_id = gen.next_id(SnowflakeType::Context);
        db.insert_context(ctx_id).unwrap();

        let archived_at = gen.next_id(SnowflakeType::Context);
        let token_count = 5000;
        let summary = "Test summary".to_string();
        let blob = b"JSONL content".to_vec();

        db.archive_context(ctx_id, archived_at, token_count, summary.clone(), blob.clone())
            .unwrap();

        let retrieved = db.get_latest_context().unwrap().unwrap();
        assert_eq!(retrieved.id, ctx_id);
        assert!(!retrieved.is_active());
        assert_eq!(retrieved.archived_at, Some(archived_at));
        assert_eq!(retrieved.token_count, Some(token_count));
        assert_eq!(retrieved.summary, Some(summary));
        assert_eq!(retrieved.blob, Some(blob));
    }

    #[test]
    fn test_get_latest_returns_newest() {
        let (db, gen) = test_db_and_gen();

        let ctx1 = gen.next_id(SnowflakeType::Context);
        let ctx2 = gen.next_id(SnowflakeType::Context);
        let ctx3 = gen.next_id(SnowflakeType::Context);

        db.insert_context(ctx1).unwrap();
        db.insert_context(ctx2).unwrap();
        db.insert_context(ctx3).unwrap();

        let latest = db.get_latest_context().unwrap().unwrap();
        assert_eq!(latest.id, ctx3);
    }

    #[test]
    fn test_no_context_returns_none() {
        let (db, _gen) = test_db_and_gen();
        let result = db.get_latest_context().unwrap();
        assert!(result.is_none());
    }
}

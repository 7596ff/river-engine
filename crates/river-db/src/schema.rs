use river_core::{RiverError, RiverResult};
use rusqlite::Connection;
use std::path::Path;

/// Database wrapper
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create database at path
    pub fn open(path: &Path) -> RiverResult<Self> {
        let conn = Connection::open(path).map_err(|e| RiverError::database(e.to_string()))?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open in-memory database (for testing)
    pub fn open_in_memory() -> RiverResult<Self> {
        let conn =
            Connection::open_in_memory().map_err(|e| RiverError::database(e.to_string()))?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Run migrations
    fn migrate(&self) -> RiverResult<()> {
        self.conn
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                applied_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            ",
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        self.run_migration("001_messages", include_str!("migrations/001_messages.sql"))?;
        self.run_migration("002_memories", include_str!("migrations/002_memories.sql"))?;
        self.run_migration("003_contexts", include_str!("migrations/003_contexts.sql"))?;
        self.run_migration("004_moves", include_str!("migrations/004_moves.sql"))?;
        Ok(())
    }

    fn run_migration(&self, name: &str, sql: &str) -> RiverResult<()> {
        let applied: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM migrations WHERE name = ?)",
                [name],
                |row| row.get(0),
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        if !applied {
            self.conn
                .execute_batch(sql)
                .map_err(|e| RiverError::database(e.to_string()))?;
            self.conn
                .execute("INSERT INTO migrations (name) VALUES (?)", [name])
                .map_err(|e| RiverError::database(e.to_string()))?;
            tracing::info!("Applied migration: {}", name);
        }
        Ok(())
    }

    /// Get connection reference
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Initialize database at path
pub fn init_db(path: &Path) -> RiverResult<Database> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Database::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.conn().is_autocommit());
    }

    #[test]
    fn test_migrations_idempotent() {
        let db = Database::open_in_memory().unwrap();
        // Running migrate again should not fail
        db.migrate().unwrap();
    }
}

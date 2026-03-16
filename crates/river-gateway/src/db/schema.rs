use rusqlite::{Connection, Result};
use std::path::Path;

/// Database wrapper
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create database at path
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open in-memory database (for testing)
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Run migrations
    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                applied_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            "
        )?;

        self.run_migration("001_messages", include_str!("migrations/001_messages.sql"))?;
        Ok(())
    }

    fn run_migration(&self, name: &str, sql: &str) -> Result<()> {
        let applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM migrations WHERE name = ?)",
            [name],
            |row| row.get(0),
        )?;

        if !applied {
            self.conn.execute_batch(sql)?;
            self.conn.execute(
                "INSERT INTO migrations (name) VALUES (?)",
                [name],
            )?;
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
pub fn init_db(path: &Path) -> Result<Database> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
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

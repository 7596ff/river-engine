//! Move CRUD operations

use river_core::{Snowflake, RiverError, RiverResult};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use crate::schema::Database;

/// A move: structural summary of one agent turn
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Move {
    pub id: Snowflake,
    pub channel: String,
    pub turn_number: u64,
    pub summary: String,
    pub tool_calls: Option<String>, // JSON
    pub created_at: i64,
}

impl Move {
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

        Ok(Self {
            id,
            channel: row.get(1)?,
            turn_number: row.get::<_, i64>(2)? as u64,
            summary: row.get(3)?,
            tool_calls: row.get(4)?,
            created_at: row.get(5)?,
        })
    }
}

impl Database {
    /// Insert a move
    pub fn insert_move(&self, m: &Move) -> RiverResult<()> {
        self.conn()
            .execute(
                "INSERT INTO moves (id, channel, turn_number, summary, tool_calls, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    m.id.to_bytes().to_vec(),
                    m.channel,
                    m.turn_number as i64,
                    m.summary,
                    m.tool_calls,
                    m.created_at,
                ],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
    }

    /// Get moves for a channel, ordered by turn_number ascending
    pub fn get_moves(&self, channel: &str, limit: usize) -> RiverResult<Vec<Move>> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, channel, turn_number, summary, tool_calls, created_at
                 FROM moves
                 WHERE channel = ?
                 ORDER BY turn_number ASC
                 LIMIT ?",
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        let moves = stmt
            .query_map(params![channel, limit as i64], Move::from_row)
            .map_err(|e| RiverError::database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(moves)
    }

    /// Get highest turn number with a move for a channel
    pub fn get_max_turn(&self, channel: &str) -> RiverResult<Option<u64>> {
        let result: Option<i64> = self
            .conn()
            .query_row(
                "SELECT MAX(turn_number) FROM moves WHERE channel = ?",
                params![channel],
                |row| row.get(0),
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(result.map(|n| n as u64))
    }

    /// Count moves for a channel
    pub fn count_moves(&self, channel: &str) -> RiverResult<usize> {
        let count: i64 = self
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM moves WHERE channel = ?",
                params![channel],
                |row| row.get(0),
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn test_gen() -> SnowflakeGenerator {
        SnowflakeGenerator::new(AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap())
    }

    #[test]
    fn test_move_insert_and_query() {
        let db = test_db();
        let gen = test_gen();

        let m = Move {
            id: gen.next_id(SnowflakeType::Embedding),
            channel: "general".to_string(),
            turn_number: 1,
            summary: "User asked about X, agent explored files".to_string(),
            tool_calls: Some(r#"["read","glob"]"#.to_string()),
            created_at: 1000,
        };

        db.insert_move(&m).unwrap();

        let moves = db.get_moves("general", 100).unwrap();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].summary, "User asked about X, agent explored files");
        assert_eq!(moves[0].turn_number, 1);
    }

    #[test]
    fn test_get_moves_ordered_by_turn() {
        let db = test_db();
        let gen = test_gen();

        for turn in [3, 1, 2] {
            let m = Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: turn,
                summary: format!("Turn {}", turn),
                tool_calls: None,
                created_at: 1000 + turn as i64,
            };
            db.insert_move(&m).unwrap();
        }

        let moves = db.get_moves("general", 100).unwrap();
        assert_eq!(moves.len(), 3);
        assert_eq!(moves[0].turn_number, 1);
        assert_eq!(moves[1].turn_number, 2);
        assert_eq!(moves[2].turn_number, 3);
    }

    #[test]
    fn test_get_max_turn_empty() {
        let db = test_db();
        assert_eq!(db.get_max_turn("general").unwrap(), None);
    }

    #[test]
    fn test_get_max_turn() {
        let db = test_db();
        let gen = test_gen();

        for turn in [1, 5, 3] {
            let m = Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: turn,
                summary: format!("Turn {}", turn),
                tool_calls: None,
                created_at: 1000,
            };
            db.insert_move(&m).unwrap();
        }

        assert_eq!(db.get_max_turn("general").unwrap(), Some(5));
        assert_eq!(db.get_max_turn("other").unwrap(), None);
    }

    #[test]
    fn test_count_moves() {
        let db = test_db();
        let gen = test_gen();

        assert_eq!(db.count_moves("general").unwrap(), 0);

        for turn in 1..=3 {
            let m = Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: turn,
                summary: format!("Turn {}", turn),
                tool_calls: None,
                created_at: 1000,
            };
            db.insert_move(&m).unwrap();
        }

        assert_eq!(db.count_moves("general").unwrap(), 3);
        assert_eq!(db.count_moves("other").unwrap(), 0);
    }
}

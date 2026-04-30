//! Message CRUD operations

use river_core::{Snowflake, RiverError, RiverResult};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use crate::schema::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Snowflake,
    pub session_id: String,
    pub role: MessageRole,
    pub content: Option<String>,
    pub tool_calls: Option<String>,  // JSON
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
    pub created_at: i64,
    pub metadata: Option<String>,    // JSON
    pub turn_number: u64,
}

impl Message {
    fn from_row(row: &Row) -> Result<Self, rusqlite::Error> {
        let id_bytes: Vec<u8> = row.get(0)?;
        let id_array: [u8; 16] = id_bytes.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Blob,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid snowflake ID length"))
            )
        })?;
        let id = Snowflake::from_bytes(id_array);

        let role_str: String = row.get(2)?;
        let role = match MessageRole::from_str(&role_str) {
            Some(r) => r,
            None => {
                tracing::warn!("Invalid message role in database: {}, defaulting to User", role_str);
                MessageRole::User
            }
        };

        Ok(Self {
            id,
            session_id: row.get(1)?,
            role,
            content: row.get(3)?,
            tool_calls: row.get(4)?,
            tool_call_id: row.get(5)?,
            name: row.get(6)?,
            created_at: row.get(7)?,
            metadata: row.get(8)?,
            turn_number: row.get::<_, i64>(9)? as u64,
        })
    }
}

impl Database {
    /// Insert a message
    pub fn insert_message(&self, msg: &Message) -> RiverResult<()> {
        self.conn().execute(
            "INSERT INTO messages (id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                msg.id.to_bytes().to_vec(),
                msg.session_id,
                msg.role.as_str(),
                msg.content,
                msg.tool_calls,
                msg.tool_call_id,
                msg.name,
                msg.created_at,
                msg.metadata,
                msg.turn_number as i64,
            ],
        ).map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
    }

    /// Get messages for a session, ordered by creation time
    pub fn get_session_messages(&self, session_id: &str, limit: usize) -> RiverResult<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
             FROM messages
             WHERE session_id = ?
             ORDER BY created_at DESC
             LIMIT ?"
        ).map_err(|e| RiverError::database(e.to_string()))?;

        let messages = stmt.query_map(params![session_id, limit as i64], Message::from_row)
            .map_err(|e| RiverError::database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))?;

        // Reverse to get chronological order
        Ok(messages.into_iter().rev().collect())
    }

    /// Get recent messages across all sessions
    pub fn get_recent_messages(&self, limit: usize) -> RiverResult<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
             FROM messages
             ORDER BY created_at DESC
             LIMIT ?"
        ).map_err(|e| RiverError::database(e.to_string()))?;

        let messages = stmt.query_map(params![limit as i64], Message::from_row)
            .map_err(|e| RiverError::database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(messages.into_iter().rev().collect())
    }

    /// Get messages with turn_number > the given turn, ordered chronologically
    pub fn get_messages_above_turn(&self, session_id: &str, turn: u64) -> RiverResult<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
             FROM messages
             WHERE session_id = ? AND turn_number > ?
             ORDER BY created_at ASC"
        ).map_err(|e| RiverError::database(e.to_string()))?;

        let messages = stmt.query_map(params![session_id, turn as i64], Message::from_row)
            .map_err(|e| RiverError::database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(messages)
    }

    /// Get messages for specific turn numbers, ordered chronologically
    pub fn get_messages_for_turns(&self, session_id: &str, turns: &[u64]) -> RiverResult<Vec<Message>> {
        if turns.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = turns.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
             FROM messages
             WHERE session_id = ? AND turn_number IN ({})
             ORDER BY created_at ASC",
            placeholders.join(", ")
        );

        let mut stmt = self.conn().prepare(&sql)
            .map_err(|e| RiverError::database(e.to_string()))?;

        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params_vec.push(Box::new(session_id.to_string()));
        for t in turns {
            params_vec.push(Box::new(*t as i64));
        }
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let messages = stmt.query_map(params_refs.as_slice(), Message::from_row)
            .map_err(|e| RiverError::database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(messages)
    }

    /// Get distinct turn numbers below a threshold, ordered newest first
    pub fn get_distinct_turns_below(&self, session_id: &str, below_turn: u64, limit: usize) -> RiverResult<Vec<u64>> {
        let mut stmt = self.conn().prepare(
            "SELECT DISTINCT turn_number FROM messages
             WHERE session_id = ? AND turn_number < ?
             ORDER BY turn_number DESC
             LIMIT ?"
        ).map_err(|e| RiverError::database(e.to_string()))?;

        let turns = stmt.query_map(params![session_id, below_turn as i64, limit as i64], |row| {
            row.get::<_, i64>(0).map(|n| n as u64)
        })
        .map_err(|e| RiverError::database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(turns)
    }

    /// Get messages for a specific turn in a session
    pub fn get_turn_messages(&self, session_id: &str, turn_number: u64) -> RiverResult<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
             FROM messages
             WHERE session_id = ? AND turn_number = ?
             ORDER BY created_at"
        ).map_err(|e| RiverError::database(e.to_string()))?;

        let messages = stmt.query_map(params![session_id, turn_number as i64], Message::from_row)
            .map_err(|e| RiverError::database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};

    #[test]
    fn test_insert_and_get_message() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "test-session".to_string(),
            role: MessageRole::User,
            content: Some("Hello, world!".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            created_at: 1234567890,
            metadata: None,
            turn_number: 0,
        };

        db.insert_message(&msg).unwrap();

        let messages = db.get_session_messages("test-session", 10).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, Some("Hello, world!".to_string()));
    }

    #[test]
    fn test_message_ordering() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        for i in 0..5 {
            let msg = Message {
                id: gen.next_id(SnowflakeType::Message),
                session_id: "test-session".to_string(),
                role: MessageRole::User,
                content: Some(format!("Message {}", i)),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                created_at: 1000 + i,
                metadata: None,
                turn_number: 0,
            };
            db.insert_message(&msg).unwrap();
        }

        let messages = db.get_session_messages("test-session", 10).unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, Some("Message 0".to_string()));
        assert_eq!(messages[4].content, Some("Message 4".to_string()));
    }

    #[test]
    fn test_insert_message_with_turn_number() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "test-session".to_string(),
            role: MessageRole::User,
            content: Some("Hello".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            turn_number: 1,
            created_at: 1000,
            metadata: None,
        };

        db.insert_message(&msg).unwrap();
        let messages = db.get_session_messages("test-session", 10).unwrap();
        assert_eq!(messages[0].turn_number, 1);
    }

    #[test]
    fn test_get_turn_messages() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        for (role, content) in [
            (MessageRole::User, "What is X?"),
            (MessageRole::Assistant, "X is Y."),
        ] {
            let msg = Message {
                id: gen.next_id(SnowflakeType::Message),
                session_id: "sess".to_string(),
                role,
                content: Some(content.to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                turn_number: 1,
                created_at: 1000,
                metadata: None,
            };
            db.insert_message(&msg).unwrap();
        }

        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "sess".to_string(),
            role: MessageRole::User,
            content: Some("Next question".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            turn_number: 2,
            created_at: 2000,
            metadata: None,
        };
        db.insert_message(&msg).unwrap();

        let turn_1 = db.get_turn_messages("sess", 1).unwrap();
        assert_eq!(turn_1.len(), 2);
        assert_eq!(turn_1[0].content, Some("What is X?".to_string()));
        assert_eq!(turn_1[1].content, Some("X is Y.".to_string()));

        let turn_2 = db.get_turn_messages("sess", 2).unwrap();
        assert_eq!(turn_2.len(), 1);
    }

    #[test]
    fn test_get_messages_above_turn() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 4, 30, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        for turn in 1..=3 {
            let msg = Message {
                id: gen.next_id(SnowflakeType::Message),
                session_id: "sess".into(),
                role: MessageRole::User,
                content: Some(format!("Turn {} message", turn)),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                created_at: turn as i64 * 100,
                metadata: None,
                turn_number: turn,
            };
            db.insert_message(&msg).unwrap();
        }

        let msgs = db.get_messages_above_turn("sess", 1).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].turn_number, 2);
        assert_eq!(msgs[1].turn_number, 3);

        let msgs = db.get_messages_above_turn("sess", 0).unwrap();
        assert_eq!(msgs.len(), 3);

        let msgs = db.get_messages_above_turn("sess", 3).unwrap();
        assert_eq!(msgs.len(), 0);
    }

    #[test]
    fn test_get_messages_for_turns() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 4, 30, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        for i in 0..2 {
            let msg = Message {
                id: gen.next_id(SnowflakeType::Message),
                session_id: "sess".into(),
                role: MessageRole::User,
                content: Some(format!("Turn 5 msg {}", i)),
                tool_calls: None, tool_call_id: None, name: None,
                created_at: 500 + i,
                metadata: None,
                turn_number: 5,
            };
            db.insert_message(&msg).unwrap();
        }
        for i in 0..3 {
            let msg = Message {
                id: gen.next_id(SnowflakeType::Message),
                session_id: "sess".into(),
                role: MessageRole::User,
                content: Some(format!("Turn 6 msg {}", i)),
                tool_calls: None, tool_call_id: None, name: None,
                created_at: 600 + i,
                metadata: None,
                turn_number: 6,
            };
            db.insert_message(&msg).unwrap();
        }

        let msgs = db.get_messages_for_turns("sess", &[5, 6]).unwrap();
        assert_eq!(msgs.len(), 5);

        let msgs = db.get_messages_for_turns("sess", &[6]).unwrap();
        assert_eq!(msgs.len(), 3);

        let msgs = db.get_messages_for_turns("sess", &[]).unwrap();
        assert_eq!(msgs.len(), 0);
    }

    #[test]
    fn test_get_distinct_turns_below() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 4, 30, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        for turn in 1..=5 {
            let msg = Message {
                id: gen.next_id(SnowflakeType::Message),
                session_id: "sess".into(),
                role: MessageRole::User,
                content: Some(format!("Turn {}", turn)),
                tool_calls: None, tool_call_id: None, name: None,
                created_at: turn as i64 * 100,
                metadata: None,
                turn_number: turn,
            };
            db.insert_message(&msg).unwrap();
        }

        let turns = db.get_distinct_turns_below("sess", 5, 3).unwrap();
        assert_eq!(turns, vec![4, 3, 2]);

        let turns = db.get_distinct_turns_below("sess", 2, 10).unwrap();
        assert_eq!(turns, vec![1]);
    }
}

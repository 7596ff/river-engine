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
        })
    }
}

impl Database {
    /// Insert a message
    pub fn insert_message(&self, msg: &Message) -> RiverResult<()> {
        self.conn().execute(
            "INSERT INTO messages (id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            ],
        ).map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
    }

    /// Get messages for a session, ordered by creation time
    pub fn get_session_messages(&self, session_id: &str, limit: usize) -> RiverResult<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata
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
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata
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
            };
            db.insert_message(&msg).unwrap();
        }

        let messages = db.get_session_messages("test-session", 10).unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, Some("Message 0".to_string()));
        assert_eq!(messages[4].content, Some("Message 4".to_string()));
    }
}

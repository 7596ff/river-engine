//! SnowflakeType - 8-bit type identifier for Snowflake IDs.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The type of entity a Snowflake ID represents.
///
/// Each type has a unique 8-bit identifier:
/// - Message: 0x01
/// - Embedding: 0x02
/// - Session: 0x03
/// - Subagent: 0x04
/// - ToolCall: 0x05
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SnowflakeType {
    /// A message in a conversation.
    Message = 0x01,
    /// An embedding vector.
    Embedding = 0x02,
    /// A conversation session.
    Session = 0x03,
    /// A subagent spawned by the main agent.
    Subagent = 0x04,
    /// A tool call invocation.
    ToolCall = 0x05,
}

impl SnowflakeType {
    /// Convert the type to its u8 representation.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Try to create a SnowflakeType from a u8 value.
    ///
    /// Returns None if the value doesn't correspond to a valid type.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(SnowflakeType::Message),
            0x02 => Some(SnowflakeType::Embedding),
            0x03 => Some(SnowflakeType::Session),
            0x04 => Some(SnowflakeType::Subagent),
            0x05 => Some(SnowflakeType::ToolCall),
            _ => None,
        }
    }

    /// Get all valid SnowflakeType variants.
    pub fn all() -> &'static [SnowflakeType] {
        &[
            SnowflakeType::Message,
            SnowflakeType::Embedding,
            SnowflakeType::Session,
            SnowflakeType::Subagent,
            SnowflakeType::ToolCall,
        ]
    }
}

impl fmt::Display for SnowflakeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnowflakeType::Message => write!(f, "Message"),
            SnowflakeType::Embedding => write!(f, "Embedding"),
            SnowflakeType::Session => write!(f, "Session"),
            SnowflakeType::Subagent => write!(f, "Subagent"),
            SnowflakeType::ToolCall => write!(f, "ToolCall"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snowflake_type_values() {
        assert_eq!(SnowflakeType::Message.as_u8(), 0x01);
        assert_eq!(SnowflakeType::Embedding.as_u8(), 0x02);
        assert_eq!(SnowflakeType::Session.as_u8(), 0x03);
        assert_eq!(SnowflakeType::Subagent.as_u8(), 0x04);
        assert_eq!(SnowflakeType::ToolCall.as_u8(), 0x05);
    }

    #[test]
    fn test_snowflake_type_from_u8_valid() {
        assert_eq!(SnowflakeType::from_u8(0x01), Some(SnowflakeType::Message));
        assert_eq!(SnowflakeType::from_u8(0x02), Some(SnowflakeType::Embedding));
        assert_eq!(SnowflakeType::from_u8(0x03), Some(SnowflakeType::Session));
        assert_eq!(SnowflakeType::from_u8(0x04), Some(SnowflakeType::Subagent));
        assert_eq!(SnowflakeType::from_u8(0x05), Some(SnowflakeType::ToolCall));
    }

    #[test]
    fn test_snowflake_type_from_u8_invalid() {
        assert_eq!(SnowflakeType::from_u8(0x00), None);
        assert_eq!(SnowflakeType::from_u8(0x06), None);
        assert_eq!(SnowflakeType::from_u8(0xFF), None);
    }

    #[test]
    fn test_snowflake_type_roundtrip() {
        for &t in SnowflakeType::all() {
            let value = t.as_u8();
            let recovered = SnowflakeType::from_u8(value).unwrap();
            assert_eq!(t, recovered);
        }
    }

    #[test]
    fn test_snowflake_type_display() {
        assert_eq!(format!("{}", SnowflakeType::Message), "Message");
        assert_eq!(format!("{}", SnowflakeType::Embedding), "Embedding");
        assert_eq!(format!("{}", SnowflakeType::Session), "Session");
        assert_eq!(format!("{}", SnowflakeType::Subagent), "Subagent");
        assert_eq!(format!("{}", SnowflakeType::ToolCall), "ToolCall");
    }

    #[test]
    fn test_snowflake_type_serde_roundtrip() {
        for &t in SnowflakeType::all() {
            let json = serde_json::to_string(&t).unwrap();
            let deserialized: SnowflakeType = serde_json::from_str(&json).unwrap();
            assert_eq!(t, deserialized);
        }
    }

    #[test]
    fn test_snowflake_type_all() {
        let all = SnowflakeType::all();
        assert_eq!(all.len(), 5);
        assert!(all.contains(&SnowflakeType::Message));
        assert!(all.contains(&SnowflakeType::Embedding));
        assert!(all.contains(&SnowflakeType::Session));
        assert!(all.contains(&SnowflakeType::Subagent));
        assert!(all.contains(&SnowflakeType::ToolCall));
    }
}

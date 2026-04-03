//! Snowflake type identifier enum.

use serde::{Deserialize, Serialize};

/// 8-bit type identifier for snowflake IDs.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnowflakeType {
    Message = 0x01,
    Embedding = 0x02,
    Session = 0x03,
    Subagent = 0x04,
    ToolCall = 0x05,
    Context = 0x06,
    Flash = 0x07,
    Move = 0x08,
    Moment = 0x09,
}

impl SnowflakeType {
    /// Parse from string (lowercase).
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "message" => Some(Self::Message),
            "embedding" => Some(Self::Embedding),
            "session" => Some(Self::Session),
            "subagent" => Some(Self::Subagent),
            "tool_call" => Some(Self::ToolCall),
            "context" => Some(Self::Context),
            "flash" => Some(Self::Flash),
            "move" => Some(Self::Move),
            "moment" => Some(Self::Moment),
            _ => None,
        }
    }

    /// Convert to lowercase string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Embedding => "embedding",
            Self::Session => "session",
            Self::Subagent => "subagent",
            Self::ToolCall => "tool_call",
            Self::Context => "context",
            Self::Flash => "flash",
            Self::Move => "move",
            Self::Moment => "moment",
        }
    }
}

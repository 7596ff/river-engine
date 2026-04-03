//! 128-bit Snowflake ID.

use serde::{Deserialize, Serialize};

use super::{AgentBirth, SnowflakeType};

/// 128-bit unique identifier.
///
/// Format:
/// - high (64 bits): timestamp in microseconds since agent birth
/// - low (64 bits): [birth:36][type:8][sequence:20]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Snowflake {
    pub(crate) high: u64,
    pub(crate) low: u64,
}

impl Snowflake {
    /// Create a new Snowflake from components.
    pub fn new(
        timestamp_micros: u64,
        birth: AgentBirth,
        snowflake_type: SnowflakeType,
        sequence: u32,
    ) -> Self {
        let low = (birth.as_u64() << 28) | ((snowflake_type as u64) << 20) | (sequence as u64 & 0xFFFFF);
        Self {
            high: timestamp_micros,
            low,
        }
    }

    /// Get the high 64 bits (timestamp in microseconds since birth).
    pub fn high(&self) -> u64 {
        self.high
    }

    /// Get the low 64 bits.
    pub fn low(&self) -> u64 {
        self.low
    }

    /// Extract the timestamp in microseconds since agent birth.
    pub fn timestamp_micros(&self) -> u64 {
        self.high
    }

    /// Extract the agent birth.
    pub fn birth(&self) -> AgentBirth {
        AgentBirth::from_u64(self.low >> 28)
    }

    /// Extract the snowflake type.
    pub fn snowflake_type(&self) -> Option<SnowflakeType> {
        let type_byte = ((self.low >> 20) & 0xFF) as u8;
        match type_byte {
            0x01 => Some(SnowflakeType::Message),
            0x02 => Some(SnowflakeType::Embedding),
            0x03 => Some(SnowflakeType::Session),
            0x04 => Some(SnowflakeType::Subagent),
            0x05 => Some(SnowflakeType::ToolCall),
            0x06 => Some(SnowflakeType::Context),
            0x07 => Some(SnowflakeType::Flash),
            0x08 => Some(SnowflakeType::Move),
            0x09 => Some(SnowflakeType::Moment),
            _ => None,
        }
    }

    /// Extract the sequence number.
    pub fn sequence(&self) -> u32 {
        (self.low & 0xFFFFF) as u32
    }

    /// Convert to a 128-bit array.
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&self.high.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.low.to_be_bytes());
        bytes
    }

    /// Create from a 128-bit array.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        let high = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
        let low = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
        Self { high, low }
    }
}

impl std::fmt::Display for Snowflake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}-{:016x}", self.high, self.low)
    }
}

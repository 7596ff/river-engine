//! Snowflake - 128-bit unique identifier.
//!
//! Layout:
//! - high (64 bits): timestamp (microseconds since agent birth)
//! - low (64 bits): [birth:36][type:8][sequence:20]

use super::{AgentBirth, SnowflakeType};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

/// A 128-bit Snowflake ID.
///
/// The ID is composed of:
/// - 64 bits: timestamp (microseconds since agent birth)
/// - 36 bits: agent birth (packed yyyymmddhhmmss)
/// - 8 bits: type identifier
/// - 20 bits: sequence number
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Snowflake {
    /// High 64 bits: timestamp in microseconds since agent birth.
    high: u64,
    /// Low 64 bits: [birth:36][type:8][sequence:20].
    low: u64,
}

impl Snowflake {
    /// Bit masks and shifts for the low 64 bits.
    const BIRTH_SHIFT: u32 = 28; // 8 + 20
    const BIRTH_MASK: u64 = 0xF_FFFF_FFFF; // 36 bits
    const TYPE_SHIFT: u32 = 20;
    const TYPE_MASK: u64 = 0xFF; // 8 bits
    const SEQUENCE_MASK: u64 = 0xF_FFFF; // 20 bits

    /// Create a new Snowflake ID.
    ///
    /// # Arguments
    /// * `timestamp_micros` - Microseconds since agent birth
    /// * `birth` - Agent birth timestamp
    /// * `snowflake_type` - Type of entity this ID represents
    /// * `sequence` - Sequence number (0-1048575, 20 bits)
    pub fn new(
        timestamp_micros: u64,
        birth: AgentBirth,
        snowflake_type: SnowflakeType,
        sequence: u32,
    ) -> Self {
        let low = ((birth.as_u64() & Self::BIRTH_MASK) << Self::BIRTH_SHIFT)
            | ((snowflake_type.as_u8() as u64 & Self::TYPE_MASK) << Self::TYPE_SHIFT)
            | (sequence as u64 & Self::SEQUENCE_MASK);

        Self {
            high: timestamp_micros,
            low,
        }
    }

    /// Create a Snowflake from raw high and low components.
    pub fn from_parts(high: u64, low: u64) -> Self {
        Self { high, low }
    }

    /// Get the timestamp in microseconds since agent birth.
    pub fn timestamp_micros(&self) -> u64 {
        self.high
    }

    /// Get the agent birth component.
    pub fn birth(&self) -> AgentBirth {
        AgentBirth::from_raw((self.low >> Self::BIRTH_SHIFT) & Self::BIRTH_MASK)
    }

    /// Get the snowflake type.
    ///
    /// Returns None if the stored type value is invalid.
    pub fn snowflake_type(&self) -> Option<SnowflakeType> {
        let type_value = ((self.low >> Self::TYPE_SHIFT) & Self::TYPE_MASK) as u8;
        SnowflakeType::from_u8(type_value)
    }

    /// Get the raw type value (useful when the type might be unknown).
    pub fn type_raw(&self) -> u8 {
        ((self.low >> Self::TYPE_SHIFT) & Self::TYPE_MASK) as u8
    }

    /// Get the sequence number.
    pub fn sequence(&self) -> u32 {
        (self.low & Self::SEQUENCE_MASK) as u32
    }

    /// Get the high 64 bits (timestamp).
    pub fn high(&self) -> u64 {
        self.high
    }

    /// Get the low 64 bits (birth + type + sequence).
    pub fn low(&self) -> u64 {
        self.low
    }

    /// Convert to a 16-byte big-endian representation.
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&self.high.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.low.to_be_bytes());
        bytes
    }

    /// Create from a 16-byte big-endian representation.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        let high = u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
        let low = u64::from_be_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]);
        Self { high, low }
    }
}

impl Ord for Snowflake {
    fn cmp(&self, other: &Self) -> Ordering {
        // Sort by timestamp first, then by low bits
        match self.high.cmp(&other.high) {
            Ordering::Equal => self.low.cmp(&other.low),
            ord => ord,
        }
    }
}

impl PartialOrd for Snowflake {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display as bare hex: 32 chars, zero-padded, sortable as strings
        write!(f, "{:016x}{:016x}", self.high, self.low)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_birth() -> AgentBirth {
        AgentBirth::new(2024, 3, 15, 14, 30, 45).unwrap()
    }

    #[test]
    fn test_snowflake_creation() {
        let birth = test_birth();
        let id = Snowflake::new(1000000, birth, SnowflakeType::Message, 42);

        assert_eq!(id.timestamp_micros(), 1000000);
        assert_eq!(id.birth(), birth);
        assert_eq!(id.snowflake_type(), Some(SnowflakeType::Message));
        assert_eq!(id.sequence(), 42);
    }

    #[test]
    fn test_snowflake_bytes_roundtrip() {
        let birth = test_birth();
        let id = Snowflake::new(9876543210, birth, SnowflakeType::Session, 12345);

        let bytes = id.to_bytes();
        let recovered = Snowflake::from_bytes(bytes);

        assert_eq!(id, recovered);
        assert_eq!(recovered.timestamp_micros(), 9876543210);
        assert_eq!(recovered.birth(), birth);
        assert_eq!(recovered.snowflake_type(), Some(SnowflakeType::Session));
        assert_eq!(recovered.sequence(), 12345);
    }

    #[test]
    fn test_snowflake_ordering_by_timestamp() {
        let birth = test_birth();

        let id1 = Snowflake::new(1000, birth, SnowflakeType::Message, 0);
        let id2 = Snowflake::new(2000, birth, SnowflakeType::Message, 0);
        let id3 = Snowflake::new(3000, birth, SnowflakeType::Message, 0);

        assert!(id1 < id2);
        assert!(id2 < id3);
        assert!(id1 < id3);

        let mut ids = vec![id3, id1, id2];
        ids.sort();
        assert_eq!(ids, vec![id1, id2, id3]);
    }

    #[test]
    fn test_snowflake_ordering_by_low_when_timestamp_equal() {
        let birth = test_birth();

        let id1 = Snowflake::new(1000, birth, SnowflakeType::Message, 1);
        let id2 = Snowflake::new(1000, birth, SnowflakeType::Message, 2);
        let id3 = Snowflake::new(1000, birth, SnowflakeType::Message, 3);

        assert!(id1 < id2);
        assert!(id2 < id3);
    }

    #[test]
    fn test_snowflake_max_sequence() {
        let birth = test_birth();
        let max_seq = 0xF_FFFF; // 20 bits max

        let id = Snowflake::new(1000, birth, SnowflakeType::Message, max_seq);
        assert_eq!(id.sequence(), max_seq);
    }

    #[test]
    fn test_snowflake_sequence_overflow_masked() {
        let birth = test_birth();
        // Pass a value larger than 20 bits
        let overflow_seq = 0x1F_FFFF; // 21 bits set

        let id = Snowflake::new(1000, birth, SnowflakeType::Message, overflow_seq);
        // Should be masked to 20 bits
        assert_eq!(id.sequence(), 0xF_FFFF);
    }

    #[test]
    fn test_snowflake_all_types() {
        let birth = test_birth();

        for &t in SnowflakeType::all() {
            let id = Snowflake::new(1000, birth, t, 0);
            assert_eq!(id.snowflake_type(), Some(t));
            assert_eq!(id.type_raw(), t.as_u8());
        }
    }

    #[test]
    fn test_snowflake_from_parts() {
        let birth = test_birth();
        let original = Snowflake::new(12345678, birth, SnowflakeType::Embedding, 999);

        let reconstructed = Snowflake::from_parts(original.high(), original.low());
        assert_eq!(original, reconstructed);
    }

    #[test]
    fn test_snowflake_display() {
        let birth = test_birth();
        let id = Snowflake::new(0x123456789ABCDEF0, birth, SnowflakeType::Message, 0);

        let display = format!("{}", id);
        assert!(!display.contains("-"));
        assert_eq!(display.len(), 32); // 16 + 16, bare hex
    }

    #[test]
    fn test_snowflake_serde_roundtrip() {
        let birth = test_birth();
        let id = Snowflake::new(999999, birth, SnowflakeType::ToolCall, 777);

        let json = serde_json::to_string(&id).unwrap();
        let deserialized: Snowflake = serde_json::from_str(&json).unwrap();

        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_snowflake_bytes_are_big_endian() {
        let birth = test_birth();
        let id = Snowflake::new(0x0102030405060708, birth, SnowflakeType::Message, 0);

        let bytes = id.to_bytes();
        // First 8 bytes should be the high value in big-endian
        assert_eq!(bytes[0], 0x01);
        assert_eq!(bytes[1], 0x02);
        assert_eq!(bytes[7], 0x08);
    }

    #[test]
    fn test_snowflake_equality() {
        let birth = test_birth();
        let id1 = Snowflake::new(1000, birth, SnowflakeType::Message, 42);
        let id2 = Snowflake::new(1000, birth, SnowflakeType::Message, 42);
        let id3 = Snowflake::new(1000, birth, SnowflakeType::Message, 43);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }
}

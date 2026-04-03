//! Thread-safe snowflake generator.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{AgentBirth, Snowflake, SnowflakeType};

/// Thread-safe generator for a single agent birth.
pub struct SnowflakeGenerator {
    birth: AgentBirth,
    birth_unix_micros: u64,
    last_timestamp: AtomicU64,
    sequence: AtomicU32,
}

impl SnowflakeGenerator {
    /// Create a new generator for the given birth.
    pub fn new(birth: AgentBirth) -> Self {
        let birth_unix_micros = (birth.to_unix_secs() as u64) * 1_000_000;
        Self {
            birth,
            birth_unix_micros,
            last_timestamp: AtomicU64::new(0),
            sequence: AtomicU32::new(0),
        }
    }

    /// Generate a new snowflake ID.
    pub fn next(&self, snowflake_type: SnowflakeType) -> Snowflake {
        let now_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_micros() as u64;

        let relative_micros = now_micros.saturating_sub(self.birth_unix_micros);

        // Update timestamp and sequence atomically
        let last = self.last_timestamp.load(Ordering::Acquire);
        let (timestamp, sequence) = if relative_micros > last {
            // New timestamp, reset sequence
            self.last_timestamp.store(relative_micros, Ordering::Release);
            self.sequence.store(0, Ordering::Release);
            (relative_micros, 0)
        } else {
            // Same timestamp, increment sequence
            let seq = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
            if seq >= 0xFFFFF {
                // Sequence overflow, wait for next microsecond
                std::thread::sleep(std::time::Duration::from_micros(1));
                return self.next(snowflake_type);
            }
            (last, seq)
        };

        Snowflake::new(timestamp, self.birth, snowflake_type, sequence)
    }

    /// Get the birth this generator is for.
    pub fn birth(&self) -> AgentBirth {
        self.birth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generates_unique_ids() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let id1 = gen.next(SnowflakeType::Message);
        let id2 = gen.next(SnowflakeType::Message);

        assert_ne!(id1, id2);
    }

    #[test]
    fn test_extracts_birth() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let id = gen.next(SnowflakeType::Message);
        let extracted = id.birth();

        assert_eq!(extracted.year(), 2026);
        assert_eq!(extracted.month(), 4);
        assert_eq!(extracted.day(), 1);
    }
}

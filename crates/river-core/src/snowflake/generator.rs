//! SnowflakeGenerator - Thread-safe generator for unique Snowflake IDs.

use super::{AgentBirth, Snowflake, SnowflakeType};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum sequence value (20 bits).
const MAX_SEQUENCE: u32 = 0xF_FFFF;

/// Internal state protected by mutex.
struct GeneratorState {
    last_timestamp: u64,
    sequence: u32,
}

/// Thread-safe generator for unique Snowflake IDs.
///
/// The generator maintains:
/// - The agent birth timestamp (used in all generated IDs)
/// - The birth time in microseconds since Unix epoch (for calculating relative timestamps)
/// - The last timestamp and sequence counter (protected by mutex)
pub struct SnowflakeGenerator {
    /// The agent birth timestamp.
    birth: AgentBirth,
    /// Birth time in microseconds since Unix epoch.
    birth_timestamp_micros: u64,
    /// Protected state for thread-safe access.
    state: Mutex<GeneratorState>,
}

impl SnowflakeGenerator {
    /// Create a new SnowflakeGenerator for the given agent birth.
    ///
    /// # Arguments
    /// * `birth` - The agent birth timestamp
    pub fn new(birth: AgentBirth) -> Self {
        let birth_timestamp_micros = Self::birth_to_micros(&birth);

        Self {
            birth,
            birth_timestamp_micros,
            state: Mutex::new(GeneratorState {
                last_timestamp: 0,
                sequence: 0,
            }),
        }
    }

    /// Convert an AgentBirth to microseconds since Unix epoch.
    fn birth_to_micros(birth: &AgentBirth) -> u64 {
        birth.to_epoch_micros()
    }

    /// Get the current timestamp in microseconds since agent birth.
    fn current_timestamp_micros(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before Unix epoch")
            .as_micros() as u64;

        // Saturating subtraction in case of clock issues
        now.saturating_sub(self.birth_timestamp_micros)
    }

    /// Generate the next unique Snowflake ID.
    ///
    /// This method is thread-safe and will generate monotonically increasing IDs
    /// even when called concurrently from multiple threads.
    ///
    /// # Arguments
    /// * `snowflake_type` - The type of entity this ID represents
    ///
    /// # Returns
    /// A new unique Snowflake ID
    pub fn next_id(&self, snowflake_type: SnowflakeType) -> Snowflake {
        let mut state = self.state.lock().unwrap();

        loop {
            let current_ts = self.current_timestamp_micros();

            if current_ts > state.last_timestamp {
                // New timestamp - reset sequence
                state.last_timestamp = current_ts;
                state.sequence = 0;
                return Snowflake::new(current_ts, self.birth, snowflake_type, 0);
            }

            // Same timestamp - increment sequence
            if state.sequence < MAX_SEQUENCE {
                state.sequence += 1;
                return Snowflake::new(state.last_timestamp, self.birth, snowflake_type, state.sequence);
            }

            // Sequence overflow - wait for next microsecond
            // Release lock briefly to allow other threads and reduce contention
            drop(state);
            std::hint::spin_loop();
            state = self.state.lock().unwrap();
        }
    }

    /// Get the agent birth used by this generator.
    pub fn birth(&self) -> AgentBirth {
        self.birth
    }

    /// Get the birth timestamp in microseconds since Unix epoch.
    pub fn birth_timestamp_micros(&self) -> u64 {
        self.birth_timestamp_micros
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn test_birth() -> AgentBirth {
        // Use a past date to ensure positive timestamps
        AgentBirth::new(2020, 1, 1, 0, 0, 0).unwrap()
    }

    #[test]
    fn test_generator_creation() {
        let birth = test_birth();
        let gen = SnowflakeGenerator::new(birth);

        assert_eq!(gen.birth(), birth);
        assert!(gen.birth_timestamp_micros() > 0);
    }

    #[test]
    fn test_generator_next_id() {
        let birth = test_birth();
        let gen = SnowflakeGenerator::new(birth);

        let id = gen.next_id(SnowflakeType::Message);

        assert_eq!(id.birth(), birth);
        assert_eq!(id.snowflake_type(), Some(SnowflakeType::Message));
        assert!(id.timestamp_micros() > 0);
    }

    #[test]
    fn test_generator_uniqueness() {
        let birth = test_birth();
        let gen = SnowflakeGenerator::new(birth);

        let mut ids = HashSet::new();
        for _ in 0..1000 {
            let id = gen.next_id(SnowflakeType::Message);
            assert!(ids.insert(id), "Duplicate ID generated");
        }

        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn test_generator_ordering_monotonic() {
        let birth = test_birth();
        let gen = SnowflakeGenerator::new(birth);

        let mut ids: Vec<Snowflake> = Vec::new();
        for _ in 0..1000 {
            ids.push(gen.next_id(SnowflakeType::Message));
        }

        // All IDs should be in strictly increasing order
        for i in 1..ids.len() {
            assert!(
                ids[i] > ids[i - 1],
                "IDs not monotonically increasing at index {}: {:?} vs {:?}",
                i,
                ids[i - 1],
                ids[i]
            );
        }
    }

    #[test]
    fn test_generator_different_types() {
        let birth = test_birth();
        let gen = SnowflakeGenerator::new(birth);

        let id1 = gen.next_id(SnowflakeType::Message);
        let id2 = gen.next_id(SnowflakeType::Embedding);
        let id3 = gen.next_id(SnowflakeType::Session);

        assert_eq!(id1.snowflake_type(), Some(SnowflakeType::Message));
        assert_eq!(id2.snowflake_type(), Some(SnowflakeType::Embedding));
        assert_eq!(id3.snowflake_type(), Some(SnowflakeType::Session));

        // All should still be unique and ordered
        assert!(id1 < id2);
        assert!(id2 < id3);
    }

    #[test]
    fn test_birth_to_micros() {
        // Test that birth_to_micros produces reasonable values
        let birth_2020 = AgentBirth::new(2020, 1, 1, 0, 0, 0).unwrap();
        let birth_2024 = AgentBirth::new(2024, 1, 1, 0, 0, 0).unwrap();

        let micros_2020 = SnowflakeGenerator::birth_to_micros(&birth_2020);
        let micros_2024 = SnowflakeGenerator::birth_to_micros(&birth_2024);

        // 4 years = approximately 4 * 365.25 * 24 * 60 * 60 * 1_000_000 microseconds
        let four_years_micros = 4 * 365 * 24 * 60 * 60 * 1_000_000_u64;

        let diff = micros_2024 - micros_2020;
        // Should be approximately 4 years (within a few days for leap years)
        assert!(
            diff > four_years_micros - 10 * 24 * 60 * 60 * 1_000_000,
            "Difference too small"
        );
        assert!(
            diff < four_years_micros + 10 * 24 * 60 * 60 * 1_000_000,
            "Difference too large"
        );
    }

    // is_leap_year test removed — logic moved to AgentBirth::to_epoch_micros

    #[test]
    fn test_generator_sequence_increments() {
        let birth = test_birth();
        let gen = SnowflakeGenerator::new(birth);

        // Generate IDs quickly to ensure some share the same timestamp
        let ids: Vec<Snowflake> = (0..100).map(|_| gen.next_id(SnowflakeType::Message)).collect();

        // Find IDs with the same timestamp and verify sequences are different
        let mut by_timestamp: std::collections::HashMap<u64, Vec<u32>> =
            std::collections::HashMap::new();
        for id in &ids {
            by_timestamp
                .entry(id.timestamp_micros())
                .or_default()
                .push(id.sequence());
        }

        // For any timestamp with multiple IDs, sequences should be unique
        for (_, seqs) in by_timestamp {
            let unique_seqs: HashSet<_> = seqs.iter().collect();
            assert_eq!(seqs.len(), unique_seqs.len(), "Duplicate sequences found");
        }
    }

    #[test]
    fn test_generator_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let birth = test_birth();
        let gen = Arc::new(SnowflakeGenerator::new(birth));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let gen = Arc::clone(&gen);
                thread::spawn(move || {
                    let mut ids = Vec::new();
                    for _ in 0..250 {
                        ids.push(gen.next_id(SnowflakeType::Message));
                    }
                    ids
                })
            })
            .collect();

        let mut all_ids = HashSet::new();
        for handle in handles {
            let ids = handle.join().unwrap();
            for id in ids {
                assert!(all_ids.insert(id), "Duplicate ID generated across threads");
            }
        }

        assert_eq!(all_ids.len(), 1000);
    }
}

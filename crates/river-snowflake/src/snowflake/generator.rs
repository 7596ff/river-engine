//! Thread-safe snowflake generator.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{AgentBirth, Snowflake, SnowflakeType};

// Pack timestamp (44 bits) and sequence (20 bits) into a single u64
const SEQ_BITS: u64 = 20;
const SEQ_MASK: u64 = (1 << SEQ_BITS) - 1; // 0xFFFFF
const TS_SHIFT: u64 = SEQ_BITS;

/// Combine timestamp and sequence into a packed state value.
#[inline]
fn pack_state(timestamp: u64, sequence: u64) -> u64 {
    (timestamp << TS_SHIFT) | (sequence & SEQ_MASK)
}

/// Extract timestamp from packed state.
#[inline]
fn unpack_timestamp(state: u64) -> u64 {
    state >> TS_SHIFT
}

/// Extract sequence from packed state.
#[inline]
fn unpack_sequence(state: u64) -> u64 {
    state & SEQ_MASK
}

/// Thread-safe generator for a single agent birth.
pub struct SnowflakeGenerator {
    birth: AgentBirth,
    birth_unix_micros: u64,
    /// Packed state: upper 44 bits = timestamp, lower 20 bits = sequence
    state: AtomicU64,
}

impl SnowflakeGenerator {
    /// Create a new generator for the given birth.
    pub fn new(birth: AgentBirth) -> Self {
        let birth_unix_micros = (birth.to_unix_secs() as u64) * 1_000_000;
        Self {
            birth,
            birth_unix_micros,
            state: AtomicU64::new(0),
        }
    }

    /// Generate a new snowflake ID.
    ///
    /// Returns `Err(SnowflakeError::SequenceOverflow)` if more than 0xFFFFF (1,048,575)
    /// IDs are generated in the same microsecond.
    pub fn next(&self, snowflake_type: SnowflakeType) -> Result<Snowflake, crate::SnowflakeError> {
        let now_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_micros() as u64;

        let relative_micros = now_micros.saturating_sub(self.birth_unix_micros);

        loop {
            let current_state = self.state.load(Ordering::Acquire);
            let last_ts = unpack_timestamp(current_state);
            let last_seq = unpack_sequence(current_state);

            let (new_ts, new_seq) = if relative_micros > last_ts {
                // New timestamp, reset sequence to 0
                (relative_micros, 0)
            } else {
                // Same or older timestamp, increment sequence
                let seq = last_seq + 1;
                if seq > SEQ_MASK {
                    // Sequence overflow
                    return Err(crate::SnowflakeError::SequenceOverflow);
                }
                (last_ts, seq)
            };

            let new_state = pack_state(new_ts, new_seq);

            // Try to atomically update the state
            match self.state.compare_exchange(
                current_state,
                new_state,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Successfully claimed this (timestamp, sequence) pair
                    return Ok(Snowflake::new(new_ts, self.birth, snowflake_type, new_seq as u32));
                }
                Err(_) => {
                    // Lost race, retry
                    continue;
                }
            }
        }
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

        let id1 = gen.next(SnowflakeType::Message).unwrap();
        let id2 = gen.next(SnowflakeType::Message).unwrap();

        assert_ne!(id1, id2);
    }

    #[test]
    fn test_extracts_birth() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let id = gen.next(SnowflakeType::Message).unwrap();
        let extracted = id.birth();

        assert_eq!(extracted.year(), 2026);
        assert_eq!(extracted.month(), 4);
        assert_eq!(extracted.day(), 1);
    }

    #[test]
    fn test_concurrent_generation() {
        use std::collections::HashSet;
        use std::sync::Arc;
        use std::thread;

        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let gen = Arc::new(SnowflakeGenerator::new(birth));
        let num_threads = 8;
        let ids_per_thread = 1000;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let gen = Arc::clone(&gen);
                thread::spawn(move || {
                    let mut ids = Vec::with_capacity(ids_per_thread);
                    for _ in 0..ids_per_thread {
                        if let Ok(id) = gen.next(SnowflakeType::Message) {
                            ids.push(id);
                        }
                    }
                    ids
                })
            })
            .collect();

        let mut all_ids = HashSet::new();
        for handle in handles {
            for id in handle.join().unwrap() {
                let key = (id.high(), id.low());
                assert!(
                    all_ids.insert(key),
                    "Duplicate ID detected: {:016x}-{:016x}",
                    id.high(),
                    id.low()
                );
            }
        }
    }
}

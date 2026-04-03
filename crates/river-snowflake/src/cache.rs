//! Generator cache for embedded generation.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::{AgentBirth, Snowflake, SnowflakeGenerator, SnowflakeType};

/// Cache of generators keyed by AgentBirth.
pub struct GeneratorCache {
    generators: RwLock<HashMap<u64, Arc<SnowflakeGenerator>>>,
}

impl GeneratorCache {
    /// Create empty cache.
    pub fn new() -> Self {
        Self {
            generators: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a generator for the given birth.
    fn get_or_create(&self, birth: AgentBirth) -> Arc<SnowflakeGenerator> {
        let key = birth.as_u64();

        // Try read lock first
        {
            let read = self.generators.read().unwrap();
            if let Some(gen) = read.get(&key) {
                return Arc::clone(gen);
            }
        }

        // Need to create - acquire write lock
        let mut write = self.generators.write().unwrap();
        // Double-check after acquiring write lock
        if let Some(gen) = write.get(&key) {
            return Arc::clone(gen);
        }

        let gen = Arc::new(SnowflakeGenerator::new(birth));
        write.insert(key, Arc::clone(&gen));
        gen
    }

    /// Generate single ID (creates generator for birth if needed).
    pub fn next_id(&self, birth: AgentBirth, snowflake_type: SnowflakeType) -> Snowflake {
        let gen = self.get_or_create(birth);
        gen.next(snowflake_type)
    }

    /// Generate multiple IDs.
    pub fn next_ids(
        &self,
        birth: AgentBirth,
        snowflake_type: SnowflakeType,
        count: usize,
    ) -> Vec<Snowflake> {
        let gen = self.get_or_create(birth);
        (0..count).map(|_| gen.next(snowflake_type)).collect()
    }

    /// Get the number of generators in the cache.
    pub fn len(&self) -> usize {
        self.generators.read().unwrap().len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for GeneratorCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_reuses_generators() {
        let cache = GeneratorCache::new();
        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();

        let _id1 = cache.next_id(birth, SnowflakeType::Message);
        let _id2 = cache.next_id(birth, SnowflakeType::Message);

        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_different_births() {
        let cache = GeneratorCache::new();
        let birth1 = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let birth2 = AgentBirth::new(2026, 4, 2, 12, 0, 0).unwrap();

        let _id1 = cache.next_id(birth1, SnowflakeType::Message);
        let _id2 = cache.next_id(birth2, SnowflakeType::Message);

        assert_eq!(cache.len(), 2);
    }
}

//! Redis connection wrapper with agent namespacing

use fred::prelude::*;
use river_core::{RiverError, RiverResult};

/// Redis configuration
#[derive(Debug, Clone)]
pub struct RedisConfig {
    pub url: String,
    pub agent_name: String,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6379".to_string(),
            agent_name: "default".to_string(),
        }
    }
}

/// Redis client with agent namespacing
#[derive(Clone)]
pub struct RedisClient {
    inner: fred::clients::RedisClient,
    agent_name: String,
}

impl RedisClient {
    /// Create new Redis client
    pub async fn new(config: RedisConfig) -> RiverResult<Self> {
        let redis_config = fred::types::RedisConfig::from_url(&config.url)
            .map_err(|e| RiverError::database(format!("Invalid Redis URL: {}", e)))?;

        let inner = fred::clients::RedisClient::new(redis_config, None, None, None);
        inner.connect();
        inner
            .wait_for_connect()
            .await
            .map_err(|e| RiverError::database(format!("Connection failed: {}", e)))?;

        Ok(Self {
            inner,
            agent_name: config.agent_name,
        })
    }

    /// Get namespaced key for a domain
    fn namespaced_key(&self, domain: &str, key: &str) -> String {
        format!("river:{}:{}:{}", self.agent_name, domain, key)
    }

    // Working memory domain
    pub async fn working_set(&self, key: &str, value: &str, ttl_minutes: u64) -> RiverResult<()> {
        let full_key = self.namespaced_key("working", key);
        self.inner
            .set::<(), _, _>(&full_key, value, Some(Expiration::EX(ttl_minutes as i64 * 60)), None, false)
            .await
            .map_err(|e| RiverError::database(format!("SET failed: {}", e)))
    }

    pub async fn working_get(&self, key: &str) -> RiverResult<Option<String>> {
        let full_key = self.namespaced_key("working", key);
        self.inner
            .get::<Option<String>, _>(&full_key)
            .await
            .map_err(|e| RiverError::database(format!("GET failed: {}", e)))
    }

    pub async fn working_delete(&self, key: &str) -> RiverResult<bool> {
        let full_key = self.namespaced_key("working", key);
        let deleted: i64 = self.inner
            .del(&full_key)
            .await
            .map_err(|e| RiverError::database(format!("DEL failed: {}", e)))?;
        Ok(deleted > 0)
    }

    // Medium-term domain
    pub async fn medium_set(&self, key: &str, value: &str, ttl_hours: u64) -> RiverResult<()> {
        let full_key = self.namespaced_key("medium", key);
        self.inner
            .set::<(), _, _>(&full_key, value, Some(Expiration::EX(ttl_hours as i64 * 3600)), None, false)
            .await
            .map_err(|e| RiverError::database(format!("SET failed: {}", e)))
    }

    pub async fn medium_get(&self, key: &str) -> RiverResult<Option<String>> {
        let full_key = self.namespaced_key("medium", key);
        self.inner
            .get::<Option<String>, _>(&full_key)
            .await
            .map_err(|e| RiverError::database(format!("GET failed: {}", e)))
    }

    // Coordination domain
    pub async fn acquire_lock(&self, key: &str, ttl_seconds: u64) -> RiverResult<bool> {
        let full_key = self.namespaced_key("coord", &format!("lock:{}", key));
        let result: Option<String> = self.inner
            .set(&full_key, "locked", Some(Expiration::EX(ttl_seconds as i64)), Some(SetOptions::NX), false)
            .await
            .map_err(|e| RiverError::database(format!("SET NX failed: {}", e)))?;
        Ok(result.is_some())
    }

    pub async fn release_lock(&self, key: &str) -> RiverResult<bool> {
        let full_key = self.namespaced_key("coord", &format!("lock:{}", key));
        let deleted: i64 = self.inner
            .del(&full_key)
            .await
            .map_err(|e| RiverError::database(format!("DEL failed: {}", e)))?;
        Ok(deleted > 0)
    }

    pub async fn counter_incr(&self, key: &str) -> RiverResult<i64> {
        let full_key = self.namespaced_key("coord", &format!("counter:{}", key));
        self.inner
            .incr(&full_key)
            .await
            .map_err(|e| RiverError::database(format!("INCR failed: {}", e)))
    }

    pub async fn counter_get(&self, key: &str) -> RiverResult<i64> {
        let full_key = self.namespaced_key("coord", &format!("counter:{}", key));
        let value: Option<i64> = self.inner
            .get(&full_key)
            .await
            .map_err(|e| RiverError::database(format!("GET failed: {}", e)))?;
        Ok(value.unwrap_or(0))
    }

    // Cache domain
    pub async fn cache_set(&self, key: &str, value: &str, ttl_seconds: Option<u64>) -> RiverResult<()> {
        let full_key = self.namespaced_key("cache", key);
        let expiration = ttl_seconds.map(|s| Expiration::EX(s as i64));
        self.inner
            .set::<(), _, _>(&full_key, value, expiration, None, false)
            .await
            .map_err(|e| RiverError::database(format!("SET failed: {}", e)))
    }

    pub async fn cache_get(&self, key: &str) -> RiverResult<Option<String>> {
        let full_key = self.namespaced_key("cache", key);
        self.inner
            .get::<Option<String>, _>(&full_key)
            .await
            .map_err(|e| RiverError::database(format!("GET failed: {}", e)))
    }

    /// Check if Redis is connected
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = RedisConfig::default();
        assert_eq!(config.url, "redis://127.0.0.1:6379");
        assert_eq!(config.agent_name, "default");
    }
}

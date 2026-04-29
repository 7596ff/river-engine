//! Cache tools

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::RedisClient;
use crate::tools::{Tool, ToolResult};

/// Cache set tool
pub struct CacheSetTool {
    redis: Arc<RedisClient>,
}

impl CacheSetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for CacheSetTool {
    fn name(&self) -> &str {
        "cache_set"
    }

    fn description(&self) -> &str {
        "Store computed value in cache"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Cache key" },
                "value": { "type": "string", "description": "Value to cache" },
                "ttl_seconds": { "type": "integer", "description": "TTL in seconds (optional, omit for no expiry)" }
            },
            "required": ["key", "value"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = args
            .get("value")
            .map(|v| {
                if v.is_string() {
                    v.as_str().unwrap().to_string()
                } else {
                    v.to_string()
                }
            })
            .ok_or_else(|| RiverError::tool("Missing required parameter: value".to_string()))?;

        let ttl = args.get("ttl_seconds").and_then(|v| v.as_u64());

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.cache_set(key, &value, ttl))
        })?;

        let ttl_msg = ttl
            .map(|s| format!(" with TTL {} seconds", s))
            .unwrap_or_default();

        Ok(ToolResult {
            output: format!("Cached '{}'{}", key, ttl_msg),
            output_file: None,
        })
    }
}

/// Cache get tool
pub struct CacheGetTool {
    redis: Arc<RedisClient>,
}

impl CacheGetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for CacheGetTool {
    fn name(&self) -> &str {
        "cache_get"
    }

    fn description(&self) -> &str {
        "Retrieve value from cache"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Cache key" }
            },
            "required": ["key"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.cache_get(key))
        })?;

        match value {
            Some(v) => Ok(ToolResult {
                output: v,
                output_file: None,
            }),
            None => Ok(ToolResult {
                output: format!("Cache miss: '{}'", key),
                output_file: None,
            }),
        }
    }
}

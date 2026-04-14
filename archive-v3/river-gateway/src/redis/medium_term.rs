//! Medium-term memory tools (hours TTL)

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::RedisClient;
use crate::tools::{Tool, ToolResult};

/// Medium-term memory set tool
pub struct MediumTermSetTool {
    redis: Arc<RedisClient>,
}

impl MediumTermSetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for MediumTermSetTool {
    fn name(&self) -> &str {
        "medium_term_set"
    }

    fn description(&self) -> &str {
        "Store value with TTL (hours) in medium-term memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to store" },
                "value": { "type": "string", "description": "Value to store (JSON or string)" },
                "ttl_hours": { "type": "integer", "description": "Time to live in hours", "default": 24 }
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

        let ttl = args.get("ttl_hours").and_then(|v| v.as_u64()).unwrap_or(24);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.medium_set(key, &value, ttl))
        })?;

        Ok(ToolResult {
            output: format!("Stored '{}' with TTL {} hours", key, ttl),
            output_file: None,
        })
    }
}

/// Medium-term memory get tool
pub struct MediumTermGetTool {
    redis: Arc<RedisClient>,
}

impl MediumTermGetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for MediumTermGetTool {
    fn name(&self) -> &str {
        "medium_term_get"
    }

    fn description(&self) -> &str {
        "Retrieve value from medium-term memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to retrieve" }
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
            tokio::runtime::Handle::current().block_on(self.redis.medium_get(key))
        })?;

        match value {
            Some(v) => Ok(ToolResult {
                output: v,
                output_file: None,
            }),
            None => Ok(ToolResult {
                output: format!("Key '{}' not found or expired", key),
                output_file: None,
            }),
        }
    }
}

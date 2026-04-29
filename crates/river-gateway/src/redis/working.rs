//! Working memory tools (short-term, minutes TTL)

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::RedisClient;
use crate::tools::{Tool, ToolResult};

/// Working memory set tool
pub struct WorkingMemorySetTool {
    redis: Arc<RedisClient>,
}

impl WorkingMemorySetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for WorkingMemorySetTool {
    fn name(&self) -> &str {
        "working_memory_set"
    }

    fn description(&self) -> &str {
        "Store value with TTL (minutes) in working memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to store" },
                "value": { "type": "string", "description": "Value to store (JSON or string)" },
                "ttl_minutes": { "type": "integer", "description": "Time to live in minutes", "default": 30 }
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

        let ttl = args.get("ttl_minutes").and_then(|v| v.as_u64()).unwrap_or(30);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.working_set(key, &value, ttl))
        })?;

        Ok(ToolResult {
            output: format!("Stored '{}' with TTL {} minutes", key, ttl),
            output_file: None,
        })
    }
}

/// Working memory get tool
pub struct WorkingMemoryGetTool {
    redis: Arc<RedisClient>,
}

impl WorkingMemoryGetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for WorkingMemoryGetTool {
    fn name(&self) -> &str {
        "working_memory_get"
    }

    fn description(&self) -> &str {
        "Retrieve value from working memory"
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
            tokio::runtime::Handle::current().block_on(self.redis.working_get(key))
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

/// Working memory delete tool
pub struct WorkingMemoryDeleteTool {
    redis: Arc<RedisClient>,
}

impl WorkingMemoryDeleteTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for WorkingMemoryDeleteTool {
    fn name(&self) -> &str {
        "working_memory_delete"
    }

    fn description(&self) -> &str {
        "Delete value from working memory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key to delete" }
            },
            "required": ["key"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let deleted = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.redis.working_delete(key))
        })?;

        if deleted {
            Ok(ToolResult {
                output: format!("Deleted '{}'", key),
                output_file: None,
            })
        } else {
            Ok(ToolResult {
                output: format!("Key '{}' not found", key),
                output_file: None,
            })
        }
    }
}

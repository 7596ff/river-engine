//! Coordination tools (locks, counters)

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::RedisClient;
use crate::tools::{Tool, ToolResult};

/// Resource lock tool
pub struct ResourceLockTool {
    redis: Arc<RedisClient>,
}

impl ResourceLockTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for ResourceLockTool {
    fn name(&self) -> &str {
        "resource_lock"
    }

    fn description(&self) -> &str {
        "Acquire or release a distributed lock"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Lock name" },
                "action": { "type": "string", "enum": ["acquire", "release"], "description": "Lock action" },
                "ttl_seconds": { "type": "integer", "description": "Lock TTL in seconds (for acquire)", "default": 60 }
            },
            "required": ["key", "action"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: key".to_string()))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: action".to_string()))?;

        match action {
            "acquire" => {
                let ttl = args.get("ttl_seconds").and_then(|v| v.as_u64()).unwrap_or(60);
                let acquired = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(self.redis.acquire_lock(key, ttl))
                })?;

                if acquired {
                    Ok(ToolResult {
                        output: format!("Acquired lock '{}' for {} seconds", key, ttl),
                        output_file: None,
                    })
                } else {
                    Ok(ToolResult {
                        output: format!("Lock '{}' is already held", key),
                        output_file: None,
                    })
                }
            }
            "release" => {
                let released = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(self.redis.release_lock(key))
                })?;

                if released {
                    Ok(ToolResult {
                        output: format!("Released lock '{}'", key),
                        output_file: None,
                    })
                } else {
                    Ok(ToolResult {
                        output: format!("Lock '{}' not held or already released", key),
                        output_file: None,
                    })
                }
            }
            _ => Err(RiverError::tool(format!("Invalid action: {}", action))),
        }
    }
}

/// Counter increment tool
pub struct CounterIncrementTool {
    redis: Arc<RedisClient>,
}

impl CounterIncrementTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for CounterIncrementTool {
    fn name(&self) -> &str {
        "counter_increment"
    }

    fn description(&self) -> &str {
        "Increment a counter and return new value"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Counter name" }
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
            tokio::runtime::Handle::current().block_on(self.redis.counter_incr(key))
        })?;

        Ok(ToolResult {
            output: value.to_string(),
            output_file: None,
        })
    }
}

/// Counter get tool
pub struct CounterGetTool {
    redis: Arc<RedisClient>,
}

impl CounterGetTool {
    pub fn new(redis: Arc<RedisClient>) -> Self {
        Self { redis }
    }
}

impl Tool for CounterGetTool {
    fn name(&self) -> &str {
        "counter_get"
    }

    fn description(&self) -> &str {
        "Get current counter value"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Counter name" }
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
            tokio::runtime::Handle::current().block_on(self.redis.counter_get(key))
        })?;

        Ok(ToolResult {
            output: value.to_string(),
            output_file: None,
        })
    }
}

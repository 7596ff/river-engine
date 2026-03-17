//! Communication tools for sending messages via adapters
//!
//! These tools allow the agent to send messages through configured communication adapters.

use crate::tools::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Adapter endpoint configuration
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// Adapter name (e.g., "discord", "slack")
    pub name: String,
    /// Outbound webhook URL (for sending messages)
    pub outbound_url: String,
    /// Read URL (for fetching channel history), optional
    pub read_url: Option<String>,
}

/// Registry of configured adapters
#[derive(Debug, Clone, Default)]
pub struct AdapterRegistry {
    adapters: HashMap<String, AdapterConfig>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, config: AdapterConfig) {
        self.adapters.insert(config.name.clone(), config);
    }

    pub fn get(&self, name: &str) -> Option<&AdapterConfig> {
        self.adapters.get(name)
    }

    pub fn list(&self) -> Vec<&AdapterConfig> {
        self.adapters.values().collect()
    }

    pub fn names(&self) -> Vec<&str> {
        self.adapters.keys().map(|s| s.as_str()).collect()
    }
}

/// Send message via communication adapter
pub struct SendMessageTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
}

impl SendMessageTool {
    pub fn new(registry: Arc<RwLock<AdapterRegistry>>) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
        }
    }
}

impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send message via communication adapter"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "adapter": {
                    "type": "string",
                    "description": "Adapter name (e.g., 'discord')"
                },
                "channel": {
                    "type": "string",
                    "description": "Channel to send to"
                },
                "content": {
                    "type": "string",
                    "description": "Message content to send"
                },
                "reply_to": {
                    "type": "string",
                    "description": "Optional message ID to reply to"
                }
            },
            "required": ["adapter", "channel", "content"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let adapter = args["adapter"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'adapter' parameter"))?;
        let channel = args["channel"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'channel' parameter"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'content' parameter"))?;
        let reply_to = args["reply_to"].as_str();

        // Block on async operation
        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let adapter = adapter.to_string();
        let channel = channel.to_string();
        let content = content.to_string();
        let reply_to = reply_to.map(|s| s.to_string());

        // Use tokio runtime handle to block on async
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;
                let config = registry
                    .get(&adapter)
                    .ok_or_else(|| RiverError::tool(format!("Unknown adapter: {}", adapter)))?;

                let payload = serde_json::json!({
                    "channel": channel,
                    "content": content,
                    "reply_to": reply_to,
                });

                let response = http_client
                    .post(&config.outbound_url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| RiverError::tool(format!("Failed to send message: {}", e)))?;

                if response.status().is_success() {
                    Ok(ToolResult::success(format!(
                        "Message sent to {} via {}",
                        channel, adapter
                    )))
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    Err(RiverError::tool(format!(
                        "Adapter returned error {}: {}",
                        status, body
                    )))
                }
            })
        });

        result
    }
}

/// List available communication adapters
pub struct ListAdaptersTool {
    registry: Arc<RwLock<AdapterRegistry>>,
}

impl ListAdaptersTool {
    pub fn new(registry: Arc<RwLock<AdapterRegistry>>) -> Self {
        Self { registry }
    }
}

impl Tool for ListAdaptersTool {
    fn name(&self) -> &str {
        "list_adapters"
    }

    fn description(&self) -> &str {
        "List available communication adapters"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn execute(&self, _args: Value) -> Result<ToolResult, RiverError> {
        let registry = self.registry.clone();

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;
                let adapters: Vec<_> = registry
                    .list()
                    .iter()
                    .map(|a| serde_json::json!({
                        "name": a.name,
                        "outbound_url": a.outbound_url
                    }))
                    .collect();

                Ok(ToolResult::success(serde_json::to_string_pretty(&serde_json::json!({
                    "adapters": adapters,
                    "count": adapters.len()
                })).unwrap()))
            })
        });

        result
    }
}

/// Get current context status
pub struct ContextStatusTool {
    context_limit: u64,
    context_used: Arc<std::sync::atomic::AtomicU64>,
}

impl ContextStatusTool {
    pub fn new(context_limit: u64, context_used: Arc<std::sync::atomic::AtomicU64>) -> Self {
        Self {
            context_limit,
            context_used,
        }
    }
}

impl Tool for ContextStatusTool {
    fn name(&self) -> &str {
        "context_status"
    }

    fn description(&self) -> &str {
        "Get current context window usage"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn execute(&self, _args: Value) -> Result<ToolResult, RiverError> {
        let used = self.context_used.load(std::sync::atomic::Ordering::Relaxed);
        let limit = self.context_limit;
        let percent = if limit > 0 {
            (used as f64 / limit as f64) * 100.0
        } else {
            0.0
        };
        let remaining = limit.saturating_sub(used);

        let output = serde_json::json!({
            "used": used,
            "limit": limit,
            "remaining": remaining,
            "percent": format!("{:.1}%", percent),
            "near_limit": percent >= 90.0
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&output).unwrap()))
    }
}

/// Read messages from a channel via adapter
pub struct ReadChannelTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
}

impl ReadChannelTool {
    pub fn new(registry: Arc<RwLock<AdapterRegistry>>) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
        }
    }
}

impl Tool for ReadChannelTool {
    fn name(&self) -> &str {
        "read_channel"
    }

    fn description(&self) -> &str {
        "Read messages from a channel via communication adapter"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "adapter": {
                    "type": "string",
                    "description": "Adapter name (e.g., 'discord')"
                },
                "channel": {
                    "type": "string",
                    "description": "Channel to read from"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of messages to fetch (default: 20)",
                    "default": 20
                }
            },
            "required": ["adapter", "channel"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let adapter = args["adapter"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'adapter' parameter"))?;
        let channel = args["channel"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'channel' parameter"))?;
        let limit = args["limit"].as_u64().unwrap_or(20);

        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let adapter = adapter.to_string();
        let channel = channel.to_string();

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;
                let config = registry
                    .get(&adapter)
                    .ok_or_else(|| RiverError::tool(format!("Unknown adapter: {}", adapter)))?;

                let read_url = config.read_url.as_ref()
                    .ok_or_else(|| RiverError::tool(format!(
                        "Adapter '{}' does not support reading channel history",
                        adapter
                    )))?;

                let url = format!("{}?channel={}&limit={}", read_url, channel, limit);

                let response = http_client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| RiverError::tool(format!("Failed to read channel: {}", e)))?;

                if response.status().is_success() {
                    let body = response.text().await
                        .map_err(|e| RiverError::tool(format!("Failed to read response: {}", e)))?;

                    // Try to parse as JSON and format nicely
                    if let Ok(json) = serde_json::from_str::<Value>(&body) {
                        Ok(ToolResult::success(serde_json::to_string_pretty(&json).unwrap()))
                    } else {
                        Ok(ToolResult::success(body))
                    }
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    Err(RiverError::tool(format!(
                        "Adapter returned error {}: {}",
                        status, body
                    )))
                }
            })
        });

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_registry() {
        let mut registry = AdapterRegistry::new();

        registry.register(AdapterConfig {
            name: "discord".to_string(),
            outbound_url: "http://localhost:8080/outbound".to_string(),
            read_url: Some("http://localhost:8080/read".to_string()),
        });

        assert!(registry.get("discord").is_some());
        assert!(registry.get("slack").is_none());
        assert_eq!(registry.names().len(), 1);
    }

    #[test]
    fn test_read_channel_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let tool = ReadChannelTool::new(registry);

        assert_eq!(tool.name(), "read_channel");
        let params = tool.parameters();
        assert!(params["properties"]["adapter"].is_object());
        assert!(params["properties"]["channel"].is_object());
        assert!(params["properties"]["limit"].is_object());
    }

    #[test]
    fn test_list_adapters_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let tool = ListAdaptersTool::new(registry);

        assert_eq!(tool.name(), "list_adapters");
        assert_eq!(tool.description(), "List available communication adapters");
    }

    #[test]
    fn test_send_message_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let tool = SendMessageTool::new(registry);

        assert_eq!(tool.name(), "send_message");
        let params = tool.parameters();
        assert!(params["properties"]["adapter"].is_object());
        assert!(params["properties"]["channel"].is_object());
        assert!(params["properties"]["content"].is_object());
    }

    #[test]
    fn test_context_status_tool() {
        use std::sync::atomic::AtomicU64;

        let context_used = Arc::new(AtomicU64::new(5000));
        let tool = ContextStatusTool::new(10000, context_used);

        assert_eq!(tool.name(), "context_status");
        assert_eq!(tool.description(), "Get current context window usage");
    }
}

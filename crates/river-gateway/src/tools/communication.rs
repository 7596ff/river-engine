//! Communication tools for sending messages via adapters
//!
//! These tools allow the agent to send messages through configured communication adapters.

use super::adapters::{send_to_adapter, AdapterRegistry};
use super::registry::{Tool, ToolResult};
use river_core::{RiverError, SnowflakeGenerator};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Send message via communication adapter
pub struct SendMessageTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    workspace: PathBuf,
    snowflake_gen: Arc<SnowflakeGenerator>,
}

impl SendMessageTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        workspace: PathBuf,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            workspace,
            snowflake_gen,
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
        let channel_id = args["channel"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'channel' parameter"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'content' parameter"))?;
        let reply_to = args["reply_to"].as_str();

        info!(
            adapter = %adapter,
            channel_id = %channel_id,
            content_len = content.len(),
            "SendMessageTool: Sending message"
        );

        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let channels_dir = self.workspace.join("channels");
        let snowflake_gen = self.snowflake_gen.clone();
        let adapter = adapter.to_string();
        let channel_id = channel_id.to_string();
        let content = content.to_string();
        let reply_to = reply_to.map(|s| s.to_string());

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;

                send_to_adapter(
                    &http_client,
                    &registry,
                    &adapter,
                    &channel_id,
                    &content,
                    reply_to.as_deref(),
                    &channels_dir,
                    &snowflake_gen,
                )
                .await
            })
        })
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
        info!(
            args = %serde_json::to_string(&args).unwrap_or_default(),
            "ReadChannelTool::execute called"
        );

        let adapter = args["adapter"].as_str().ok_or_else(|| {
            error!("ReadChannelTool: Missing 'adapter' parameter");
            RiverError::tool("Missing 'adapter' parameter")
        })?;
        let channel = args["channel"].as_str().ok_or_else(|| {
            error!("ReadChannelTool: Missing 'channel' parameter");
            RiverError::tool("Missing 'channel' parameter")
        })?;
        let limit = args["limit"].as_u64().unwrap_or(20);

        info!(
            adapter = %adapter,
            channel = %channel,
            limit = limit,
            "ReadChannelTool: Reading channel"
        );

        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let adapter = adapter.to_string();
        let channel = channel.to_string();

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;
                let config = registry.get(&adapter).ok_or_else(|| {
                    error!(
                        adapter = %adapter,
                        available = ?registry.names(),
                        "ReadChannelTool: Unknown adapter"
                    );
                    RiverError::tool(format!("Unknown adapter: {}", adapter))
                })?;

                let read_url = config.read_url.as_ref().ok_or_else(|| {
                    warn!(
                        adapter = %adapter,
                        "ReadChannelTool: Adapter does not support reading"
                    );
                    RiverError::tool(format!(
                        "Adapter '{}' does not support reading channel history",
                        adapter
                    ))
                })?;

                let url = format!("{}?channel={}&limit={}", read_url, channel, limit);
                debug!(url = %url, "ReadChannelTool: Sending HTTP request");

                let response = http_client.get(&url).send().await.map_err(|e| {
                    error!(error = %e, url = %url, "ReadChannelTool: HTTP request failed");
                    RiverError::tool(format!("Failed to read channel: {}", e))
                })?;

                let status = response.status();
                debug!(status = %status, "ReadChannelTool: Received HTTP response");

                if status.is_success() {
                    let body = response.text().await.map_err(|e| {
                        error!(error = %e, "ReadChannelTool: Failed to read response body");
                        RiverError::tool(format!("Failed to read response: {}", e))
                    })?;

                    info!(
                        adapter = %adapter,
                        channel = %channel,
                        body_len = body.len(),
                        "ReadChannelTool: Channel read successfully"
                    );

                    // Try to parse as JSON and format nicely
                    if let Ok(json) = serde_json::from_str::<Value>(&body) {
                        Ok(ToolResult::success(
                            serde_json::to_string_pretty(&json).unwrap(),
                        ))
                    } else {
                        Ok(ToolResult::success(body))
                    }
                } else {
                    let body = response.text().await.unwrap_or_default();
                    error!(
                        status = %status,
                        body = %body,
                        adapter = %adapter,
                        channel = %channel,
                        "ReadChannelTool: Adapter returned error"
                    );
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
    use super::super::adapters::AdapterConfig;
    use super::*;
    use river_core::AgentBirth;

    fn test_snowflake_gen() -> Arc<SnowflakeGenerator> {
        let birth = AgentBirth::new(2026, 4, 29, 12, 0, 0).unwrap();
        Arc::new(SnowflakeGenerator::new(birth))
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

    #[tokio::test]
    async fn test_send_message_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let tool = SendMessageTool::new(registry, PathBuf::from("."), test_snowflake_gen());

        assert_eq!(tool.name(), "send_message");
        let params = tool.parameters();
        assert!(params["properties"]["adapter"].is_object());
        assert!(params["properties"]["channel"].is_object());
        assert!(params["properties"]["content"].is_object());
    }
}

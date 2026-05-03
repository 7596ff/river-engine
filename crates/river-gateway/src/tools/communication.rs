//! Communication tools for sending messages via adapters
//!
//! These tools allow the agent to send messages through configured communication adapters.

use super::registry::{Tool, ToolResult};
use super::adapters::{AdapterRegistry, send_to_adapter};
use river_core::{RiverError, SnowflakeGenerator};
use river_adapter::Feature;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
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

        let adapter = args["adapter"]
            .as_str()
            .ok_or_else(|| {
                error!("ReadChannelTool: Missing 'adapter' parameter");
                RiverError::tool("Missing 'adapter' parameter")
            })?;
        let channel = args["channel"]
            .as_str()
            .ok_or_else(|| {
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
                let config = registry
                    .get(&adapter)
                    .ok_or_else(|| {
                        error!(
                            adapter = %adapter,
                            available = ?registry.names(),
                            "ReadChannelTool: Unknown adapter"
                        );
                        RiverError::tool(format!("Unknown adapter: {}", adapter))
                    })?;

                let read_url = config.read_url.as_ref()
                    .ok_or_else(|| {
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

                let response = http_client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| {
                        error!(error = %e, url = %url, "ReadChannelTool: HTTP request failed");
                        RiverError::tool(format!("Failed to read channel: {}", e))
                    })?;

                let status = response.status();
                debug!(status = %status, "ReadChannelTool: Received HTTP response");

                if status.is_success() {
                    let body = response.text().await
                        .map_err(|e| {
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
                        Ok(ToolResult::success(serde_json::to_string_pretty(&json).unwrap()))
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

/// Switch the agent's current channel
pub struct SwitchChannelTool {
    workspace: PathBuf,
    channel_context_tx: mpsc::Sender<crate::agent::ChannelContext>,
}

impl SwitchChannelTool {
    pub fn new(
        workspace: PathBuf,
        channel_context_tx: mpsc::Sender<crate::agent::ChannelContext>,
    ) -> Self {
        Self {
            workspace,
            channel_context_tx,
        }
    }
}

impl Tool for SwitchChannelTool {
    fn name(&self) -> &str {
        "switch_channel"
    }

    fn description(&self) -> &str {
        "Switch to a different channel for subsequent speak commands"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to conversation file (e.g., 'conversations/discord/myserver/general.txt')"
                }
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'path' parameter"))?;

        let path = self.workspace.join(path_str);

        info!(path = %path.display(), "Switching channel");

        // Read and parse conversation file
        let content = std::fs::read_to_string(&path).map_err(|e| {
            error!(path = %path.display(), error = %e, "Failed to read conversation file");
            RiverError::tool(format!("Conversation file not found: {}", path_str))
        })?;

        let conversation = crate::conversations::Conversation::from_str(&content).map_err(|e| {
            error!(path = %path.display(), error = %e.0, "Failed to parse conversation");
            RiverError::tool(format!("Failed to parse conversation: {}", e.0))
        })?;

        let meta = conversation.meta.ok_or_else(|| {
            error!(path = %path.display(), "Conversation file missing frontmatter");
            RiverError::tool("Conversation file missing routing metadata")
        })?;

        // Create channel context
        let context = crate::agent::ChannelContext::from_conversation(
            PathBuf::from(path_str),
            &meta,
        );

        let channel_name = context.display_name().to_string();

        // Send to agent task
        let tx = self.channel_context_tx.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                tx.send(context).await.map_err(|e| {
                    RiverError::tool(format!("Failed to update channel context: {}", e))
                })
            })
        })?;

        info!(channel = %channel_name, "Switched to channel");

        Ok(ToolResult::success(format!("Switched to channel: {}", channel_name)))
    }
}

/// Send message to the current channel
pub struct SpeakTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    workspace: PathBuf,
    snowflake_gen: Arc<SnowflakeGenerator>,
    channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
}

impl SpeakTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        workspace: PathBuf,
        snowflake_gen: Arc<SnowflakeGenerator>,
        channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            workspace,
            snowflake_gen,
            channel_context,
        }
    }
}

impl Tool for SpeakTool {
    fn name(&self) -> &str {
        "speak"
    }

    fn description(&self) -> &str {
        "Send a message to the current channel"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Message content to send"
                },
                "reply_to": {
                    "type": "string",
                    "description": "Optional message ID to reply to"
                }
            },
            "required": ["content"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let content = args["content"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'content' parameter"))?;
        let reply_to = args["reply_to"].as_str();

        info!(
            content_len = content.len(),
            reply_to = ?reply_to,
            "SpeakTool: Sending message"
        );

        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let channels_dir = self.workspace.join("channels");
        let snowflake_gen = self.snowflake_gen.clone();
        let channel_context = self.channel_context.clone();
        let content = content.to_string();
        let reply_to = reply_to.map(|s| s.to_string());

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Get channel context
                let ctx_guard = channel_context.read().await;
                let ctx = ctx_guard.as_ref().ok_or_else(|| {
                    error!("SpeakTool: No channel selected");
                    RiverError::tool("No channel selected. Use switch_channel first.")
                })?;

                let adapter = ctx.adapter.clone();
                let channel_id = ctx.channel_id.clone();

                drop(ctx_guard); // Release lock before async call

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

/// Send typing indicator to the current channel
pub struct TypingTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
}

impl TypingTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            channel_context,
        }
    }
}

impl Tool for TypingTool {
    fn name(&self) -> &str {
        "typing"
    }

    fn description(&self) -> &str {
        "Send a typing indicator to the current channel"
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
        let http_client = self.http_client.clone();
        let channel_context = self.channel_context.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Get channel context
                let ctx_guard = channel_context.read().await;
                let ctx = ctx_guard.as_ref().ok_or_else(|| {
                    error!("TypingTool: No channel selected");
                    RiverError::tool("No channel selected. Use switch_channel first.")
                })?;

                let adapter = ctx.adapter.clone();
                let channel_id = ctx.channel_id.clone();

                drop(ctx_guard); // Release lock before async call

                let registry = registry.read().await;

                // Check if adapter supports typing
                if !registry.supports(&adapter, Feature::TypingIndicator) {
                    debug!(adapter = %adapter, "Adapter doesn't support typing indicators");
                    return Ok(ToolResult::success("Typing indicator sent"));
                }

                let config = registry.get(&adapter).ok_or_else(|| {
                    RiverError::tool(format!("Adapter '{}' not registered", adapter))
                })?;

                // Build typing URL (same base as outbound, but /typing endpoint)
                let typing_url = config.outbound_url
                    .trim_end_matches("/send")
                    .to_string() + "/typing";

                let payload = serde_json::json!({
                    "channel": channel_id,
                });

                info!(
                    url = %typing_url,
                    adapter = %adapter,
                    channel_id = %channel_id,
                    "Sending typing indicator"
                );

                let response = http_client
                    .post(&typing_url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| {
                        error!(error = %e, "Typing indicator request failed");
                        RiverError::tool(format!("Failed to send typing indicator: {}", e))
                    })?;

                if response.status().is_success() {
                    Ok(ToolResult::success("Typing indicator sent"))
                } else {
                    // Silent failure - just log and return success
                    warn!(status = %response.status(), "Typing indicator returned non-success");
                    Ok(ToolResult::success("Typing indicator sent"))
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::adapters::AdapterConfig;
    use river_core::AgentBirth;
    use std::collections::HashSet;
    use tokio::sync::mpsc;

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
        let tool = SendMessageTool::new(
            registry,
            PathBuf::from("."),
            test_snowflake_gen(),
        );

        assert_eq!(tool.name(), "send_message");
        let params = tool.parameters();
        assert!(params["properties"]["adapter"].is_object());
        assert!(params["properties"]["channel"].is_object());
        assert!(params["properties"]["content"].is_object());
    }

    #[test]
    fn test_switch_channel_tool_schema() {
        let (tx, _rx) = mpsc::channel(1);
        let tool = SwitchChannelTool::new(PathBuf::from("/workspace"), tx);

        assert_eq!(tool.name(), "switch_channel");
        let params = tool.parameters();
        assert!(params["properties"]["path"].is_object());
        assert_eq!(params["required"], serde_json::json!(["path"]));
    }

    #[test]
    fn test_switch_channel_file_not_found() {
        let (tx, _rx) = mpsc::channel(1);
        let tool = SwitchChannelTool::new(PathBuf::from("/nonexistent"), tx);

        let result = tool.execute(serde_json::json!({
            "path": "conversations/missing.txt"
        }));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_switch_channel_missing_frontmatter() {
        let temp = tempfile::TempDir::new().unwrap();
        let conv_path = temp.path().join("convo.txt");
        std::fs::write(&conv_path, "[ ] 2026-03-28 10:00:00 msg1 <alice:111> hello\n").unwrap();

        let (tx, _rx) = mpsc::channel(1);
        let tool = SwitchChannelTool::new(temp.path().to_path_buf(), tx);

        let result = tool.execute(serde_json::json!({
            "path": "convo.txt"
        }));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("missing routing metadata"));
    }

    #[tokio::test]
    async fn test_speak_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let channel_context = Arc::new(RwLock::new(None));

        let tool = SpeakTool::new(
            registry,
            PathBuf::from("/workspace"),
            test_snowflake_gen(),
            channel_context,
        );

        assert_eq!(tool.name(), "speak");
        let params = tool.parameters();
        assert!(params["properties"]["content"].is_object());
        assert!(params["properties"]["reply_to"].is_object());
        assert_eq!(params["required"], serde_json::json!(["content"]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_speak_without_channel_selected() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let channel_context = Arc::new(RwLock::new(None)); // No channel set

        let tool = SpeakTool::new(
            registry,
            PathBuf::from("/workspace"),
            test_snowflake_gen(),
            channel_context,
        );

        let result = tool.execute(serde_json::json!({
            "content": "Hello!"
        }));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No channel selected"));
    }

    #[test]
    fn test_typing_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let channel_context = Arc::new(RwLock::new(None));

        let tool = TypingTool::new(registry, channel_context);

        assert_eq!(tool.name(), "typing");
        assert_eq!(tool.description(), "Send a typing indicator to the current channel");
        let params = tool.parameters();
        assert_eq!(params["properties"], serde_json::json!({}));
        assert_eq!(params["required"], serde_json::json!([]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_typing_without_channel_selected() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let channel_context = Arc::new(RwLock::new(None)); // No channel set

        let tool = TypingTool::new(registry, channel_context);

        let result = tool.execute(serde_json::json!({}));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No channel selected"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_typing_unsupported_adapter_silent_success() {
        let mut registry = AdapterRegistry::new();
        registry.register(AdapterConfig {
            name: "test".to_string(),
            outbound_url: "http://localhost:9999/send".to_string(),
            read_url: None,
            features: HashSet::new(), // No TypingIndicator feature
        });

        let registry = Arc::new(RwLock::new(registry));
        let channel_context = Arc::new(RwLock::new(Some(crate::agent::ChannelContext {
            path: PathBuf::from("test.txt"),
            adapter: "test".to_string(),
            channel_id: "123".to_string(),
            channel_name: Some("test".to_string()),
            guild_id: None,
        })));

        let tool = TypingTool::new(registry, channel_context);

        let result = tool.execute(serde_json::json!({}));

        assert!(result.is_ok());
    }
}

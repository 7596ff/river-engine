//! Communication tools for sending messages via adapters
//!
//! These tools allow the agent to send messages through configured communication adapters.

use crate::conversations::{Author, WriteOp};
use river_tools::{Tool, ToolResult};
use river_core::RiverError;
use river_adapter::Feature;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// Adapter endpoint configuration
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// Adapter name (e.g., "discord", "slack")
    pub name: String,
    /// Outbound webhook URL (for sending messages)
    pub outbound_url: String,
    /// Read URL (for fetching channel history), optional
    pub read_url: Option<String>,
    /// Supported features
    pub features: HashSet<Feature>,
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

    pub fn supports(&self, name: &str, feature: Feature) -> bool {
        self.adapters
            .get(name)
            .map(|c| c.features.contains(&feature))
            .unwrap_or(false)
    }
}

/// Send a message through an adapter (shared by speak and send_message)
async fn send_to_adapter(
    http_client: &reqwest::Client,
    registry: &AdapterRegistry,
    adapter: &str,
    channel_id: &str,
    content: &str,
    reply_to: Option<&str>,
    writer_tx: &mpsc::Sender<WriteOp>,
    conversation_path: &std::path::Path,
    agent_author: Author,
) -> Result<ToolResult, RiverError> {
    let config = registry
        .get(adapter)
        .ok_or_else(|| {
            error!(
                adapter = %adapter,
                available = ?registry.names(),
                "Unknown adapter"
            );
            RiverError::tool(format!("Adapter '{}' not registered", adapter))
        })?;

    let payload = serde_json::json!({
        "channel": channel_id,
        "content": content,
        "reply_to": reply_to,
    });

    info!(
        url = %config.outbound_url,
        adapter = %adapter,
        channel_id = %channel_id,
        content_len = content.len(),
        "Sending message to adapter"
    );

    let response = http_client
        .post(&config.outbound_url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            error!(error = %e, url = %config.outbound_url, "HTTP request failed");
            RiverError::tool(format!("Failed to send message: {}", e))
        })?;

    let status = response.status();

    if status.is_success() {
        let body = response.text().await.unwrap_or_default();

        info!(adapter = %adapter, channel_id = %channel_id, "Message sent successfully");

        // Extract message_id from adapter response
        let message_id = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message_id")?.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("out-{}", chrono::Utc::now().timestamp_millis()));

        // Record outgoing message
        let msg = crate::conversations::Message::outgoing(&message_id, agent_author, content);

        if let Err(e) = writer_tx
            .send(WriteOp::Message {
                path: conversation_path.to_path_buf(),
                msg,
            })
            .await
        {
            warn!("Failed to record outgoing message: {}", e);
        }

        Ok(ToolResult::success(format!(
            "Message sent to {} via {}",
            channel_id, adapter
        )))
    } else {
        let body = response.text().await.unwrap_or_default();
        error!(
            status = %status,
            body = %body,
            adapter = %adapter,
            channel_id = %channel_id,
            "Adapter returned error"
        );

        // Record failed message
        let msg = crate::conversations::Message::failed(
            agent_author,
            &format!("Adapter returned error {}", status),
            content,
        );

        if let Err(e) = writer_tx
            .send(WriteOp::Message {
                path: conversation_path.to_path_buf(),
                msg,
            })
            .await
        {
            warn!("Failed to record failed message: {}", e);
        }

        Err(RiverError::tool(format!(
            "Adapter returned error {}: {}",
            status, body
        )))
    }
}

/// Send message via communication adapter
pub struct SendMessageTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    workspace: PathBuf,
    agent_name: String,
    agent_id: String,
    writer_tx: mpsc::Sender<WriteOp>,
}

impl SendMessageTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        workspace: PathBuf,
        agent_name: String,
        agent_id: String,
        writer_tx: mpsc::Sender<WriteOp>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            workspace,
            agent_name,
            agent_id,
            writer_tx,
        }
    }

    fn agent_author(&self) -> Author {
        Author {
            name: self.agent_name.clone(),
            id: self.agent_id.clone(),
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
        let writer_tx = self.writer_tx.clone();
        let workspace = self.workspace.clone();
        let agent_author = self.agent_author();
        let adapter = adapter.to_string();
        let channel_id = channel_id.to_string();
        let content = content.to_string();
        let reply_to = reply_to.map(|s| s.to_string());

        // Build conversation path from adapter/channel
        let conversation_path = workspace.join(format!("conversations/{}/{}.txt", adapter, channel_id));

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
                    &writer_tx,
                    &conversation_path,
                    agent_author,
                )
                .await
            })
        })
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
        info!("ListAdaptersTool::execute called");

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

                info!(
                    adapter_count = adapters.len(),
                    adapter_names = ?registry.names(),
                    "ListAdaptersTool: Returning adapter list"
                );

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
    agent_name: String,
    agent_id: String,
    writer_tx: mpsc::Sender<WriteOp>,
    channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
}

impl SpeakTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        workspace: PathBuf,
        agent_name: String,
        agent_id: String,
        writer_tx: mpsc::Sender<WriteOp>,
        channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            workspace,
            agent_name,
            agent_id,
            writer_tx,
            channel_context,
        }
    }

    fn agent_author(&self) -> Author {
        Author {
            name: self.agent_name.clone(),
            id: self.agent_id.clone(),
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
        let writer_tx = self.writer_tx.clone();
        let workspace = self.workspace.clone();
        let agent_author = self.agent_author();
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
                let conversation_path = workspace.join(&ctx.path);

                drop(ctx_guard); // Release lock before async call

                let registry = registry.read().await;

                send_to_adapter(
                    &http_client,
                    &registry,
                    &adapter,
                    &channel_id,
                    &content,
                    reply_to.as_deref(),
                    &writer_tx,
                    &conversation_path,
                    agent_author,
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

    #[test]
    fn test_adapter_registry() {
        let mut registry = AdapterRegistry::new();

        registry.register(AdapterConfig {
            name: "discord".to_string(),
            outbound_url: "http://localhost:8080/outbound".to_string(),
            read_url: Some("http://localhost:8080/read".to_string()),
            features: HashSet::new(),
        });

        assert!(registry.get("discord").is_some());
        assert!(registry.get("slack").is_none());
        assert_eq!(registry.names().len(), 1);
    }

    #[test]
    fn test_adapter_registry_supports() {
        let mut registry = AdapterRegistry::new();

        let mut features = HashSet::new();
        features.insert(Feature::TypingIndicator);

        registry.register(AdapterConfig {
            name: "discord".to_string(),
            outbound_url: "http://localhost:8080/send".to_string(),
            read_url: None,
            features,
        });

        assert!(registry.supports("discord", Feature::TypingIndicator));
        assert!(!registry.supports("discord", Feature::Reactions));
        assert!(!registry.supports("nonexistent", Feature::TypingIndicator));
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

    #[tokio::test]
    async fn test_send_message_tool_schema() {
        use std::path::PathBuf;
        use tokio::sync::mpsc;

        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let (tx, _rx) = mpsc::channel(1);
        let tool = SendMessageTool::new(
            registry,
            PathBuf::from("."),
            "test_agent".to_string(),
            "agent_123".to_string(),
            tx,
        );

        assert_eq!(tool.name(), "send_message");
        let params = tool.parameters();
        assert!(params["properties"]["adapter"].is_object());
        assert!(params["properties"]["channel"].is_object());
        assert!(params["properties"]["content"].is_object());
    }

    #[test]
    fn test_send_message_tool_agent_author() {
        use std::path::PathBuf;
        use tokio::sync::mpsc;

        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let (tx, _rx) = mpsc::channel(1);
        let tool = SendMessageTool::new(
            registry,
            PathBuf::from("/workspace"),
            "river".to_string(),
            "river_001".to_string(),
            tx,
        );

        let author = tool.agent_author();
        assert_eq!(author.name, "river");
        assert_eq!(author.id, "river_001");
    }

    #[test]
    fn test_context_status_tool() {
        use std::sync::atomic::AtomicU64;

        let context_used = Arc::new(AtomicU64::new(5000));
        let tool = ContextStatusTool::new(10000, context_used);

        assert_eq!(tool.name(), "context_status");
        assert_eq!(tool.description(), "Get current context window usage");
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
        let (tx, _rx) = mpsc::channel(1);
        let channel_context = Arc::new(RwLock::new(None));

        let tool = SpeakTool::new(
            registry,
            PathBuf::from("/workspace"),
            "agent".to_string(),
            "agent_001".to_string(),
            tx,
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
        let (tx, _rx) = mpsc::channel(1);
        let channel_context = Arc::new(RwLock::new(None)); // No channel set

        let tool = SpeakTool::new(
            registry,
            PathBuf::from("/workspace"),
            "agent".to_string(),
            "agent_001".to_string(),
            tx,
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

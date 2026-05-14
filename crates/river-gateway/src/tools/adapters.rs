//! Adapter infrastructure — registry, config, shared send logic

use super::registry::{Tool, ToolResult};
use river_adapter::Feature;
use river_core::{RiverError, SnowflakeGenerator, SnowflakeType};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

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
pub async fn send_to_adapter(
    http_client: &reqwest::Client,
    registry: &AdapterRegistry,
    adapter: &str,
    channel_id: &str,
    content: &str,
    reply_to: Option<&str>,
    channels_dir: &Path,
    snowflake_gen: &SnowflakeGenerator,
) -> Result<ToolResult, RiverError> {
    let config = registry.get(adapter).ok_or_else(|| {
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
        let adapter_msg_id = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message_id")?.as_str().map(|s| s.to_string()));

        // Log agent message to channel JSONL
        let snowflake = snowflake_gen.next_id(SnowflakeType::Message);
        let log = crate::channels::ChannelLog::open(channels_dir, adapter, channel_id);
        let agent_entry = crate::channels::MessageEntry::agent(
            snowflake,
            content.to_string(),
            adapter.to_string(),
            adapter_msg_id,
        );
        if let Err(e) = log.append_entry(&agent_entry).await {
            warn!(error = %e, "Failed to log agent message to channel");
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

        Err(RiverError::tool(format!(
            "Adapter returned error {}: {}",
            status, body
        )))
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
                    .map(|a| {
                        serde_json::json!({
                            "name": a.name,
                            "outbound_url": a.outbound_url
                        })
                    })
                    .collect();

                info!(
                    adapter_count = adapters.len(),
                    adapter_names = ?registry.names(),
                    "ListAdaptersTool: Returning adapter list"
                );

                Ok(ToolResult::success(
                    serde_json::to_string_pretty(&serde_json::json!({
                        "adapters": adapters,
                        "count": adapters.len()
                    }))
                    .unwrap(),
                ))
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
    fn test_list_adapters_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let tool = ListAdaptersTool::new(registry);

        assert_eq!(tool.name(), "list_adapters");
        assert_eq!(tool.description(), "List available communication adapters");
    }
}

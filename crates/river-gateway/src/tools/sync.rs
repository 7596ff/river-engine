//! sync_conversation tool for fetching and merging message history

use super::registry::{Tool, ToolResult};
use super::AdapterRegistry;
use river_core::{RiverError, SnowflakeGenerator, SnowflakeType};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use urlencoding::encode;

pub struct SyncConversationTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    workspace: PathBuf,
    snowflake_gen: Arc<SnowflakeGenerator>,
}

impl SyncConversationTool {
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

impl Tool for SyncConversationTool {
    fn name(&self) -> &str {
        "sync_conversation"
    }

    fn description(&self) -> &str {
        "Fetch message history from adapter and write to channel log"
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
                    "description": "Channel ID to sync"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max messages to fetch (default: 50)"
                },
                "before": {
                    "type": "string",
                    "description": "Fetch messages before this ID (pagination)"
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
        let limit = args["limit"].as_u64().unwrap_or(50);
        let before = args["before"].as_str().map(String::from);

        info!(adapter = %adapter, channel = %channel, limit = limit, "Syncing conversation");

        // Clone for async block
        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let workspace = self.workspace.clone();
        let snowflake_gen = self.snowflake_gen.clone();
        let adapter = adapter.to_string();
        let channel = channel.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;
                let config = registry
                    .get(&adapter)
                    .ok_or_else(|| RiverError::tool(format!("Unknown adapter: {}", adapter)))?;

                let read_url = config
                    .read_url
                    .as_ref()
                    .ok_or_else(|| RiverError::tool("Adapter doesn't support reading"))?;

                // Build URL with params
                let mut url = format!("{}?channel={}&limit={}", read_url, encode(&channel), limit);
                if let Some(ref before_id) = before {
                    url.push_str(&format!("&before={}", encode(before_id)));
                }

                debug!(url = %url, "Fetching messages from adapter");

                // Fetch messages
                let response = http_client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| RiverError::tool(format!("HTTP error: {}", e)))?;

                if !response.status().is_success() {
                    let body = response.text().await.unwrap_or_default();
                    return Err(RiverError::tool(format!("Adapter error: {}", body)));
                }

                let messages: Vec<FetchedMessage> = response
                    .json()
                    .await
                    .map_err(|e| RiverError::tool(format!("Parse error: {}", e)))?;

                // Write messages to channel log
                let channels_dir = workspace.join("channels");
                let log = crate::channels::ChannelLog::open(&channels_dir, &adapter, &channel);

                let mut processed_count = 0;
                for fetched in &messages {
                    let snowflake = snowflake_gen.next_id(SnowflakeType::Message);
                    let entry = if fetched.is_bot {
                        crate::channels::MessageEntry::agent(
                            snowflake,
                            fetched.content.clone(),
                            adapter.clone(),
                            Some(fetched.id.clone()),
                        )
                    } else {
                        crate::channels::MessageEntry::incoming(
                            snowflake,
                            fetched.author_name.clone(),
                            fetched.author_id.clone(),
                            fetched.content.clone(),
                            adapter.clone(),
                            Some(fetched.id.clone()),
                        )
                    };

                    if let Err(e) = log.append_entry(&entry).await {
                        warn!(error = %e, "Failed to write synced message to channel log");
                        return Err(RiverError::tool("Failed to write to channel log"));
                    }

                    processed_count += 1;
                }

                info!(
                    fetched = messages.len(),
                    processed = processed_count,
                    "Sync complete"
                );

                Ok(ToolResult::success(
                    serde_json::json!({
                        "fetched": messages.len(),
                        "processed": processed_count,
                    })
                    .to_string(),
                ))
            })
        })
    }
}

#[derive(Debug, Deserialize)]
struct FetchedMessage {
    id: String,
    author_id: String,
    author_name: String,
    content: String,
    #[allow(dead_code)]
    timestamp: i64,
    is_bot: bool,
    #[allow(dead_code)]
    reactions: Vec<FetchedReaction>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FetchedReaction {
    emoji: String,
    count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::default()));
        let birth = river_core::AgentBirth::new(2026, 4, 29, 12, 0, 0).unwrap();
        let sg = Arc::new(SnowflakeGenerator::new(birth));
        let tool = SyncConversationTool::new(registry, PathBuf::from("."), sg);

        assert_eq!(tool.name(), "sync_conversation");
        let params = tool.parameters();
        assert!(params["properties"]["adapter"].is_object());
        assert!(params["properties"]["channel"].is_object());
        assert!(params["properties"]["limit"].is_object());
        assert!(params["properties"]["before"].is_object());
    }
}

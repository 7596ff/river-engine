//! sync_conversation tool for fetching and merging message history

use crate::conversations::path::build_discord_path;
use crate::conversations::{Author, Message, MessageDirection, WriteOp};
use river_tools::{Tool, ToolResult};
use super::AdapterRegistry;
use river_core::RiverError;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};
use urlencoding::encode;

pub struct SyncConversationTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    workspace: PathBuf,
    writer_tx: mpsc::Sender<WriteOp>,
}

impl SyncConversationTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        workspace: PathBuf,
        writer_tx: mpsc::Sender<WriteOp>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            workspace,
            writer_tx,
        }
    }
}

impl Tool for SyncConversationTool {
    fn name(&self) -> &str {
        "sync_conversation"
    }

    fn description(&self) -> &str {
        "Fetch message history from adapter and merge into conversation file"
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
        let adapter = args["adapter"].as_str()
            .ok_or_else(|| RiverError::tool("Missing 'adapter' parameter"))?;
        let channel = args["channel"].as_str()
            .ok_or_else(|| RiverError::tool("Missing 'channel' parameter"))?;
        let limit = args["limit"].as_u64().unwrap_or(50);
        let before = args["before"].as_str().map(String::from);

        info!(adapter = %adapter, channel = %channel, limit = limit, "Syncing conversation");

        // Clone for async block
        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let workspace = self.workspace.clone();
        let writer_tx = self.writer_tx.clone();
        let adapter = adapter.to_string();
        let channel = channel.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let registry = registry.read().await;
                let config = registry.get(&adapter)
                    .ok_or_else(|| RiverError::tool(format!("Unknown adapter: {}", adapter)))?;

                let read_url = config.read_url.as_ref()
                    .ok_or_else(|| RiverError::tool("Adapter doesn't support reading"))?;

                // Build URL with params
                let mut url = format!("{}?channel={}&limit={}", read_url, encode(&channel), limit);
                if let Some(ref before_id) = before {
                    url.push_str(&format!("&before={}", encode(before_id)));
                }

                debug!(url = %url, "Fetching messages from adapter");

                // Fetch messages
                let response = http_client.get(&url).send().await
                    .map_err(|e| RiverError::tool(format!("HTTP error: {}", e)))?;

                if !response.status().is_success() {
                    let body = response.text().await.unwrap_or_default();
                    return Err(RiverError::tool(format!("Adapter error: {}", body)));
                }

                let messages: Vec<FetchedMessage> = response.json().await
                    .map_err(|e| RiverError::tool(format!("Parse error: {}", e)))?;

                // Build conversation path using the path helper
                let path = build_discord_path(
                    &workspace,
                    None,  // guild_id not available in this flow
                    None,  // guild_name
                    &channel,
                    &channel,  // use channel ID as name for now
                );

                let mut processed_count = 0;
                for fetched in &messages {
                    let msg = Message {
                        direction: if fetched.is_bot {
                            MessageDirection::Outgoing
                        } else {
                            MessageDirection::Read // Assume read from history
                        },
                        timestamp: chrono::DateTime::from_timestamp(fetched.timestamp, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| {
                                warn!(
                                    message_id = %fetched.id,
                                    timestamp = fetched.timestamp,
                                    "Invalid timestamp, using current time"
                                );
                                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
                            }),
                        id: fetched.id.clone(),
                        author: Author { name: fetched.author_name.clone(), id: fetched.author_id.clone() },
                        content: fetched.content.clone(),
                        reactions: vec![],
                    };

                    if let Err(e) = writer_tx.send(WriteOp::Message { path: path.clone(), msg }).await {
                        warn!("Failed to send message to writer: {}", e);
                        return Err(RiverError::tool("Writer channel closed"));
                    }

                    // Send reaction counts
                    for r in &fetched.reactions {
                        if let Err(e) = writer_tx.send(WriteOp::ReactionCount {
                            path: path.clone(),
                            message_id: fetched.id.clone(),
                            emoji: r.emoji.clone(),
                            count: r.count,
                        }).await {
                            warn!("Failed to send reaction to writer: {}", e);
                            return Err(RiverError::tool("Writer channel closed"));
                        }
                    }

                    processed_count += 1;
                }

                info!(fetched = messages.len(), processed = processed_count, "Sync complete");

                Ok(ToolResult::success(serde_json::json!({
                    "fetched": messages.len(),
                    "processed": processed_count,
                }).to_string()))
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
    timestamp: i64,
    is_bot: bool,
    reactions: Vec<FetchedReaction>,
}

#[derive(Debug, Deserialize)]
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
        let (tx, _rx) = mpsc::channel(1);
        let tool = SyncConversationTool::new(registry, PathBuf::from("."), tx);

        assert_eq!(tool.name(), "sync_conversation");
        let params = tool.parameters();
        assert!(params["properties"]["adapter"].is_object());
        assert!(params["properties"]["channel"].is_object());
        assert!(params["properties"]["limit"].is_object());
        assert!(params["properties"]["before"].is_object());
    }
}

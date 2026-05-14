//! HTTP client for river-gateway communication

use reqwest::Client;
use river_adapter::IncomingEvent;
use serde::Deserialize;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

/// Build metadata JSON for Discord events
pub fn discord_metadata(
    guild_id: Option<String>,
    thread_id: Option<String>,
    reply_to: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "guild_id": guild_id,
        "thread_id": thread_id,
        "reply_to": reply_to,
    })
}

/// Response from gateway /incoming endpoint
#[derive(Debug, Deserialize)]
pub struct IncomingResponse {
    pub status: String,
    #[serde(default)]
    pub channel: Option<String>,
}

/// HTTP client for river-gateway
pub struct GatewayClient {
    client: Client,
    base_url: String,
}

impl GatewayClient {
    /// Create a new gateway client with a pre-configured HTTP client (includes auth headers)
    pub fn new(client: Client, base_url: String) -> Self {
        Self { client, base_url }
    }

    /// Send an incoming event to the gateway with retry and exponential backoff
    pub async fn send_incoming(
        &self,
        event: IncomingEvent,
    ) -> Result<IncomingResponse, GatewayError> {
        const MAX_RETRIES: u32 = 3;
        const INITIAL_BACKOFF_SECS: u64 = 1;

        let url = format!("{}/incoming", self.base_url);
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let backoff = Duration::from_secs(INITIAL_BACKOFF_SECS << (attempt - 1));
                sleep(backoff).await;
            }

            match self.client.post(&url).json(&event).send().await {
                Ok(response) => {
                    if !response.status().is_success() {
                        return Err(GatewayError::Response(format!(
                            "Gateway returned status {}",
                            response.status()
                        )));
                    }

                    return response
                        .json()
                        .await
                        .map_err(|e| GatewayError::Parse(e.to_string()));
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    if attempt + 1 < MAX_RETRIES {
                        warn!("Gateway request failed, retrying");
                    }
                }
            }
        }

        Err(GatewayError::Request(
            last_error.unwrap_or_else(|| "Unknown error".to_string()),
        ))
    }

    /// Check if gateway is reachable
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/health", self.base_url);
        self.client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

/// Gateway communication errors
#[derive(Debug)]
pub enum GatewayError {
    Request(String),
    Response(String),
    Parse(String),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatewayError::Request(e) => write!(f, "request failed: {}", e),
            GatewayError::Response(e) => write!(f, "bad response: {}", e),
            GatewayError::Parse(e) => write!(f, "parse error: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_adapter::{Author, EventType};

    #[test]
    fn test_incoming_event_serialization() {
        let event = IncomingEvent {
            adapter: "discord".into(),
            event_type: EventType::MessageCreate,
            channel: "123456".to_string(),
            channel_name: Some("general".to_string()),
            author: Author {
                id: "user123".to_string(),
                name: "TestUser".to_string(),
                is_bot: false,
            },
            content: "Hello world".to_string(),
            message_id: "msg789".to_string(),
            timestamp: chrono::Utc::now(),
            metadata: serde_json::json!({
                "guild_id": "guild1",
            }),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"adapter\":\"discord\""));
        assert!(json.contains("MessageCreate"));
    }

    #[test]
    fn test_gateway_client_creation() {
        let client = GatewayClient::new(Client::new(), "http://localhost:3000".to_string());
        assert_eq!(client.base_url, "http://localhost:3000");
    }

    #[test]
    fn test_discord_metadata() {
        let metadata = discord_metadata(
            Some("guild123".to_string()),
            Some("thread456".to_string()),
            None,
        );
        assert_eq!(metadata["guild_id"], "guild123");
        assert_eq!(metadata["thread_id"], "thread456");
        assert!(metadata["reply_to"].is_null());
    }
}

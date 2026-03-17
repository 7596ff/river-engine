//! HTTP client for river-gateway communication

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

/// Message author info
#[derive(Debug, Serialize)]
pub struct Author {
    pub id: String,
    pub name: String,
}

/// Metadata for incoming events
#[derive(Debug, Serialize, Default)]
pub struct EventMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

/// Incoming event sent to gateway
#[derive(Debug, Serialize)]
pub struct IncomingEvent {
    pub adapter: &'static str,
    pub event_type: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    pub message_id: String,
    pub metadata: EventMetadata,
}

/// Response from gateway /incoming endpoint
#[derive(Debug, Deserialize)]
pub struct IncomingResponse {
    pub status: String,
    pub channel: String,
}

/// HTTP client for river-gateway
pub struct GatewayClient {
    client: Client,
    base_url: String,
}

impl GatewayClient {
    /// Create a new gateway client
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, base_url }
    }

    /// Send an incoming event to the gateway with retry and exponential backoff
    pub async fn send_incoming(&self, event: IncomingEvent) -> Result<IncomingResponse, GatewayError> {
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

        Err(GatewayError::Request(last_error.unwrap_or_else(|| "Unknown error".to_string())))
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

    #[test]
    fn test_incoming_event_serialization() {
        let event = IncomingEvent {
            adapter: "discord",
            event_type: "message".to_string(),
            channel: "123456".to_string(),
            author: Author {
                id: "user123".to_string(),
                name: "TestUser".to_string(),
            },
            content: "Hello world".to_string(),
            message_id: "msg789".to_string(),
            metadata: EventMetadata {
                guild_id: Some("guild1".to_string()),
                thread_id: None,
                reply_to: None,
            },
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"adapter\":\"discord\""));
        assert!(json.contains("\"event_type\":\"message\""));
        assert!(json.contains("\"guild_id\":\"guild1\""));
        // thread_id should be skipped (None)
        assert!(!json.contains("thread_id"));
    }

    #[test]
    fn test_gateway_client_creation() {
        let client = GatewayClient::new("http://localhost:3000".to_string());
        assert_eq!(client.base_url, "http://localhost:3000");
    }
}

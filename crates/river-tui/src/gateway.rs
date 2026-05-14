//! HTTP client for river-gateway communication

use reqwest::Client;
use river_adapter::{AdapterInfo, RegisterRequest};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

/// Message sent to gateway /incoming endpoint.
/// Matches the gateway's IncomingMessage struct (routes.rs:102).
#[derive(Debug, Serialize)]
pub struct IncomingMessage {
    pub adapter: String,
    pub event_type: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// Author for incoming messages.
/// Matches the gateway's Author struct (routes.rs:126) — id and name only.
#[derive(Debug, Serialize)]
pub struct Author {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct IncomingResponse {
    pub status: String,
}

/// HTTP client for river-gateway
pub struct GatewayClient {
    client: Client,
    base_url: String,
    auth_token: Option<String>,
}

impl GatewayClient {
    pub fn new(base_url: String, auth_token: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url,
            auth_token,
        }
    }

    /// Send a user message to the gateway
    pub async fn send_incoming(&self, msg: IncomingMessage) -> Result<IncomingResponse, String> {
        let url = format!("{}/incoming", self.base_url);
        let mut req = self.client.post(&url).json(&msg);

        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response = req
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("gateway returned status {}", response.status()));
        }

        response
            .json()
            .await
            .map_err(|e| format!("parse error: {}", e))
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

    /// Register this adapter with the gateway
    pub async fn register(&self, listen_port: u16) -> Result<(), String> {
        let url = format!("{}/adapters/register", self.base_url);
        let info = AdapterInfo {
            name: "tui".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            url: format!("http://127.0.0.1:{}", listen_port),
            features: HashSet::new(),
            metadata: serde_json::json!({}),
        };

        let mut req = self
            .client
            .post(&url)
            .json(&RegisterRequest { adapter: info });

        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response: river_adapter::RegisterResponse = req
            .send()
            .await
            .map_err(|e| format!("registration request failed: {}", e))?
            .json()
            .await
            .map_err(|e| format!("failed to parse registration response: {}", e))?;

        if response.accepted {
            Ok(())
        } else {
            Err(response
                .error
                .unwrap_or_else(|| "registration rejected".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message_serialization() {
        let msg = IncomingMessage {
            adapter: "tui".into(),
            event_type: "MessageCreate".into(),
            channel: "terminal".into(),
            author: Author {
                id: "local-user".into(),
                name: "cassie".into(),
            },
            content: "hello".into(),
            message_id: Some("msg-1".into()),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"adapter\":\"tui\""));
        assert!(json.contains("\"name\":\"cassie\""));
        assert!(!json.contains("is_bot"));
    }

    #[test]
    fn test_incoming_message_no_message_id() {
        let msg = IncomingMessage {
            adapter: "tui".into(),
            event_type: "MessageCreate".into(),
            channel: "terminal".into(),
            author: Author {
                id: "local-user".into(),
                name: "cassie".into(),
            },
            content: "hello".into(),
            message_id: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("message_id"));
    }

    #[test]
    fn test_gateway_client_creation() {
        let client = GatewayClient::new("http://localhost:3000".into(), None);
        assert_eq!(client.base_url, "http://localhost:3000");
    }
}

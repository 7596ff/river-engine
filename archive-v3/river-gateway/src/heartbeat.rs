//! Heartbeat client for orchestrator communication

use river_core::RiverResult;
use serde::Serialize;
use std::time::Duration;

/// Heartbeat request payload
#[derive(Serialize)]
pub struct HeartbeatRequest {
    pub agent: String,
    pub gateway_url: String,
}

/// Heartbeat client for sending heartbeats to orchestrator
#[derive(Clone)]
pub struct HeartbeatClient {
    client: reqwest::Client,
    orchestrator_url: String,
    agent_name: String,
    gateway_url: String,
}

impl HeartbeatClient {
    /// Create new heartbeat client
    pub fn new(orchestrator_url: String, agent_name: String, gateway_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            orchestrator_url,
            agent_name,
            gateway_url,
        }
    }

    /// Send heartbeat to orchestrator
    pub async fn send_heartbeat(&self) -> RiverResult<()> {
        let url = format!("{}/heartbeat", self.orchestrator_url);
        let req = HeartbeatRequest {
            agent: self.agent_name.clone(),
            gateway_url: self.gateway_url.clone(),
        };

        match self.client.post(&url).json(&req).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    tracing::debug!("Heartbeat sent successfully");
                    Ok(())
                } else {
                    tracing::warn!("Heartbeat failed with status: {}", response.status());
                    Ok(()) // Don't error, graceful degradation
                }
            }
            Err(e) => {
                tracing::warn!("Failed to send heartbeat: {}", e);
                Ok(()) // Don't error, graceful degradation
            }
        }
    }

    /// Start heartbeat loop (runs forever, call in background task)
    pub async fn run_loop(&self, interval_seconds: u64) {
        let interval = Duration::from_secs(interval_seconds);
        loop {
            let _ = self.send_heartbeat().await;
            tokio::time::sleep(interval).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_request_serialize() {
        let req = HeartbeatRequest {
            agent: "test".to_string(),
            gateway_url: "http://localhost:3000".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"agent\":\"test\""));
    }

    #[test]
    fn test_heartbeat_client_creation() {
        let client = HeartbeatClient::new(
            "http://localhost:5000".to_string(),
            "test-agent".to_string(),
            "http://localhost:3000".to_string(),
        );
        assert_eq!(client.orchestrator_url, "http://localhost:5000");
        assert_eq!(client.agent_name, "test-agent");
    }
}

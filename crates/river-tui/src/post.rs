//! Bystander endpoint HTTP client

use reqwest::Client;
use std::time::Duration;

pub struct BystanderClient {
    client: Client,
    url: String,
    auth_token: Option<String>,
}

impl BystanderClient {
    pub fn new(url: String, auth_token: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            client,
            url,
            auth_token,
        }
    }

    /// Post a message to the bystander endpoint.
    pub async fn post(&self, content: &str) -> Result<(), String> {
        let body = serde_json::json!({ "content": content });
        let mut req = self.client.post(&self.url).json(&body);
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("gateway returned {}", resp.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_construction() {
        let client = BystanderClient::new(
            "http://localhost:3000/home/iris/message".into(),
            Some("test-token".into()),
        );
        assert_eq!(client.url, "http://localhost:3000/home/iris/message");
    }
}

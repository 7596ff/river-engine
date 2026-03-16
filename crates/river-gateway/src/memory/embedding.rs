//! Embedding server client (llama-server --embedding compatible)

use river_core::{RiverError, RiverResult};
use serde::{Deserialize, Serialize};

/// Configuration for embedding server
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub url: String,
    pub model: String,
    pub dimensions: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8081".to_string(),
            model: "nomic-embed-text-v1.5".to_string(),
            dimensions: 768,
        }
    }
}

/// Client for embedding server
#[derive(Clone)]
pub struct EmbeddingClient {
    client: reqwest::Client,
    config: EmbeddingConfig,
}

#[derive(Serialize)]
struct EmbeddingRequest {
    input: String,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl EmbeddingClient {
    /// Create new embedding client
    pub fn new(config: EmbeddingConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    /// Get embedding for text
    pub async fn embed(&self, text: &str) -> RiverResult<Vec<f32>> {
        let url = format!("{}/v1/embeddings", self.config.url);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "input": text,
                "model": self.config.model
            }))
            .send()
            .await
            .map_err(|e| RiverError::Model(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RiverError::Model(format!(
                "Embedding server error {}: {}",
                status, body
            )));
        }

        let resp: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| RiverError::Model(format!("Invalid response: {}", e)))?;

        resp.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| RiverError::Model("Empty embedding response".to_string()))
    }

    /// Get embeddings for multiple texts
    pub async fn embed_batch(&self, texts: &[String]) -> RiverResult<Vec<Vec<f32>>> {
        let url = format!("{}/v1/embeddings", self.config.url);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "input": texts,
                "model": self.config.model
            }))
            .send()
            .await
            .map_err(|e| RiverError::Model(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RiverError::Model(format!(
                "Embedding server error {}: {}",
                status, body
            )));
        }

        let resp: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| RiverError::Model(format!("Invalid response: {}", e)))?;

        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    /// Get expected embedding dimensions
    pub fn dimensions(&self) -> usize {
        self.config.dimensions
    }

    /// Check if embedding server is reachable
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/health", self.config.url);
        self.client.get(&url).send().await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.url, "http://localhost:8081");
        assert_eq!(config.dimensions, 768);
    }

    #[test]
    fn test_client_creation() {
        let client = EmbeddingClient::new(EmbeddingConfig::default());
        assert_eq!(client.dimensions(), 768);
    }
}

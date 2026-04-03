//! Embedding client for external model.

use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::config::EmbedModelConfig;

#[derive(Debug)]
pub enum EmbedError {
    Http(reqwest::Error),
    Api(String),
    InvalidResponse,
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP error: {}", e),
            Self::Api(s) => write!(f, "API error: {}", s),
            Self::InvalidResponse => write!(f, "invalid response format"),
        }
    }
}

impl std::error::Error for EmbedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for EmbedError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

/// Client for calling external embedding service.
pub struct EmbedClient {
    client: reqwest::Client,
    config: EmbedModelConfig,
}


// Ollama request/response
#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct OllamaResponse {
    embedding: Vec<f32>,
}

// OpenAI-compatible response (request uses Ollama format which is compatible)
#[derive(Deserialize)]
struct OpenAiResponse {
    data: Vec<OpenAiEmbedding>,
}

#[derive(Deserialize)]
struct OpenAiEmbedding {
    embedding: Vec<f32>,
}

impl EmbedClient {
    /// Create a new client with the given configuration.
    pub fn new(config: EmbedModelConfig) -> Self {
        let client = reqwest::Client::new();
        Self { client, config }
    }

    /// Embed multiple texts concurrently.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let futures: Vec<_> = texts.iter().map(|t| self.embed_one(t)).collect();
        let results = join_all(futures).await;

        results.into_iter().collect()
    }

    /// Embed a single text.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        // Try Ollama format first
        let ollama_req = OllamaRequest {
            model: &self.config.name,
            prompt: text,
        };

        let mut request = self.client.post(&self.config.endpoint).json(&ollama_req);

        if !self.config.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.config.api_key));
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(EmbedError::Api(format!("{}: {}", status, body)));
        }

        let body = response.text().await?;

        // Try Ollama format
        if let Ok(ollama) = serde_json::from_str::<OllamaResponse>(&body) {
            return Ok(ollama.embedding);
        }

        // Try OpenAI format
        if let Ok(openai) = serde_json::from_str::<OpenAiResponse>(&body) {
            if let Some(first) = openai.data.into_iter().next() {
                return Ok(first.embedding);
            }
        }

        Err(EmbedError::InvalidResponse)
    }
}

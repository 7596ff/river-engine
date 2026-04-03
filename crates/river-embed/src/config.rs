//! Configuration types.

use serde::{Deserialize, Serialize};

/// Configuration built from CLI args.
pub struct EmbedConfig {
    pub orchestrator_endpoint: String,
    pub name: String,
    pub port: u16,
}

/// Registration request sent to orchestrator.
#[derive(Serialize)]
pub struct RegistrationRequest {
    pub endpoint: String,
    pub embed: EmbedServiceInfo,
}

#[derive(Serialize)]
pub struct EmbedServiceInfo {
    pub name: String,
}

/// Registration response from orchestrator.
#[derive(Deserialize)]
pub struct RegistrationResponse {
    pub accepted: bool,
    pub model: Option<EmbedModelConfig>,
}

/// Model configuration received from orchestrator.
#[derive(Clone, Debug, Deserialize)]
pub struct EmbedModelConfig {
    /// Embedding model API endpoint.
    pub endpoint: String,
    /// Model name (e.g., "nomic-embed-text").
    pub name: String,
    /// API key for embedding service.
    pub api_key: String,
    /// Vector dimensions (e.g., 768).
    pub dimensions: usize,
}

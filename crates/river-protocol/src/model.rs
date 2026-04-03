//! Model configuration types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Model configuration from orchestrator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ModelConfig {
    /// LLM API endpoint URL.
    pub endpoint: String,
    /// Model name/identifier.
    pub name: String,
    /// API key for authentication.
    pub api_key: String,
    /// Maximum context window size in tokens.
    pub context_limit: usize,
}

//! Model configuration types.

use serde::{Deserialize, Serialize};
use std::fmt;
use utoipa::ToSchema;

/// Model configuration from orchestrator.
#[derive(Clone, PartialEq, Serialize, Deserialize, ToSchema)]
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

impl fmt::Debug for ModelConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelConfig")
            .field("endpoint", &self.endpoint)
            .field("name", &self.name)
            .field("api_key", &"[REDACTED]")
            .field("context_limit", &self.context_limit)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_redacts_api_key() {
        let config = ModelConfig {
            endpoint: "https://api.example.com".to_string(),
            name: "gpt-4".to_string(),
            api_key: "sk-secret-key-12345".to_string(),
            context_limit: 128000,
        };
        let debug_output = format!("{:?}", config);
        assert!(
            !debug_output.contains("sk-secret-key-12345"),
            "Debug output should not contain actual API key: {}",
            debug_output
        );
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug output should contain [REDACTED]: {}",
            debug_output
        );
    }
}

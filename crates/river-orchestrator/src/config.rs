//! Configuration types

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Orchestrator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,

    /// Seconds before agent marked unhealthy
    #[serde(default = "default_health_threshold")]
    pub health_threshold_seconds: u64,

    /// Path to models config file (optional)
    pub models_config: Option<PathBuf>,
}

fn default_port() -> u16 {
    5000
}

fn default_health_threshold() -> u64 {
    120
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            health_threshold_seconds: default_health_threshold(),
            models_config: None,
        }
    }
}

/// Model configuration entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub provider: String,
}

/// Models configuration file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsFile {
    pub models: Vec<ModelConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_config_defaults() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.port, 5000);
        assert_eq!(config.health_threshold_seconds, 120);
    }

    #[test]
    fn test_models_file_deserialize() {
        let json = r#"{"models": [{"name": "qwen3-32b", "provider": "local"}]}"#;
        let file: ModelsFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.models.len(), 1);
        assert_eq!(file.models[0].name, "qwen3-32b");
    }
}

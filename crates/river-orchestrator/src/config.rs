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

    /// Path to models config file (optional, legacy)
    pub models_config: Option<PathBuf>,

    /// Directories to scan for GGUF models
    #[serde(default)]
    pub model_dirs: Vec<PathBuf>,

    /// Path to external models config file
    pub external_models_config: Option<PathBuf>,

    /// Idle timeout in seconds before unloading models
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_seconds: u64,

    /// Path to llama-server binary
    #[serde(default = "default_llama_server_path")]
    pub llama_server_path: PathBuf,

    /// Port range for llama-server instances
    #[serde(default = "default_port_range_start")]
    pub port_range_start: u16,

    #[serde(default = "default_port_range_end")]
    pub port_range_end: u16,

    /// Reserved VRAM in MB
    #[serde(default = "default_reserve_vram_mb")]
    pub reserve_vram_mb: u64,

    /// Reserved RAM in MB
    #[serde(default = "default_reserve_ram_mb")]
    pub reserve_ram_mb: u64,
}

fn default_port() -> u16 {
    5000
}

fn default_health_threshold() -> u64 {
    120
}

fn default_idle_timeout() -> u64 {
    900 // 15 minutes
}

fn default_llama_server_path() -> PathBuf {
    PathBuf::from("llama-server")
}

fn default_port_range_start() -> u16 {
    8080
}

fn default_port_range_end() -> u16 {
    8180
}

fn default_reserve_vram_mb() -> u64 {
    500
}

fn default_reserve_ram_mb() -> u64 {
    2000
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            health_threshold_seconds: default_health_threshold(),
            models_config: None,
            model_dirs: Vec::new(),
            external_models_config: None,
            idle_timeout_seconds: default_idle_timeout(),
            llama_server_path: default_llama_server_path(),
            port_range_start: default_port_range_start(),
            port_range_end: default_port_range_end(),
            reserve_vram_mb: default_reserve_vram_mb(),
            reserve_ram_mb: default_reserve_ram_mb(),
        }
    }
}

/// Model configuration entry (legacy)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub provider: String,
}

/// Models configuration file format (legacy)
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
        assert_eq!(config.idle_timeout_seconds, 900);
        assert_eq!(config.port_range_start, 8080);
        assert_eq!(config.port_range_end, 8180);
    }

    #[test]
    fn test_models_file_deserialize() {
        let json = r#"{"models": [{"name": "qwen3-32b", "provider": "local"}]}"#;
        let file: ModelsFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.models.len(), 1);
        assert_eq!(file.models[0].name, "qwen3-32b");
    }
}

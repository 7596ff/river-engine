//! JSON config file types
//!
//! Deserialized from the --config file after env var expansion.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level config file
#[derive(Debug, Deserialize)]
pub struct RiverConfig {
    /// Orchestrator HTTP port
    #[serde(default = "default_port")]
    pub port: u16,

    /// Named model backends
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,

    /// Named agents (each becomes a gateway process)
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,

    /// Global resource management
    #[serde(default)]
    pub resources: ResourcesConfig,
}

/// A model backend
#[derive(Debug, Deserialize)]
pub struct ModelConfig {
    /// Provider type: "anthropic", "openai", "ollama", "gguf", etc.
    pub provider: String,

    /// API endpoint URL (for external models)
    #[serde(default)]
    pub endpoint: Option<String>,

    /// Model name at the endpoint
    #[serde(default)]
    pub name: Option<String>,

    /// Path to file containing API key
    #[serde(default)]
    pub api_key_file: Option<PathBuf>,

    /// Context window size in tokens
    #[serde(default)]
    pub context_limit: Option<u64>,

    /// Path to GGUF file (for provider: "gguf")
    #[serde(default)]
    pub path: Option<PathBuf>,

    /// Embedding dimensions (presence marks this as an embedding model)
    #[serde(default)]
    pub dimensions: Option<u32>,
}

/// Agent (gateway) configuration
#[derive(Debug, Deserialize)]
pub struct AgentConfig {
    /// Path to agent's workspace directory
    pub workspace: PathBuf,

    /// Path to agent's data directory (contains river.db)
    pub data_dir: PathBuf,

    /// Gateway HTTP port
    pub port: u16,

    /// Key into models map for primary model
    pub model: String,

    /// Key into models map for spectator/bystander model
    #[serde(default)]
    pub spectator_model: Option<String>,

    /// Key into models map for embeddings
    #[serde(default)]
    pub embedding_model: Option<String>,

    /// Context window configuration
    #[serde(default)]
    pub context: ContextConfig,

    /// Redis connection URL
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Path to file containing auth token for gateway API
    #[serde(default)]
    pub auth_token_file: Option<PathBuf>,

    /// Logging configuration
    #[serde(default)]
    pub log: LogConfig,

    /// Adapter processes to spawn
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
}

/// Context window shape parameters
#[derive(Debug, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "default_context_limit")]
    pub limit: u64,

    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f64,

    #[serde(default = "default_fill_target")]
    pub fill_target: f64,

    #[serde(default = "default_min_messages")]
    pub min_messages: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            limit: default_context_limit(),
            compaction_threshold: default_compaction_threshold(),
            fill_target: default_fill_target(),
            min_messages: default_min_messages(),
        }
    }
}

/// Logging configuration
#[derive(Debug, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,

    #[serde(default)]
    pub dir: Option<PathBuf>,

    #[serde(default)]
    pub file: Option<PathBuf>,

    #[serde(default)]
    pub json_stdout: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            dir: None,
            file: None,
            json_stdout: false,
        }
    }
}

/// Adapter process configuration
#[derive(Debug, Deserialize)]
pub struct AdapterConfig {
    /// Adapter type (e.g., "discord")
    #[serde(rename = "type")]
    pub adapter_type: String,

    /// Path to adapter binary (default: river-{type})
    #[serde(default)]
    pub bin: Option<PathBuf>,

    /// HTTP port for adapter's outbound server
    pub port: u16,

    /// Path to file containing token (for discord)
    #[serde(default)]
    pub token_file: Option<PathBuf>,

    /// Environment variable name for token (for discord, e.g. DISCORD_TOKEN)
    #[serde(default)]
    pub token_env: Option<String>,

    /// Guild/server ID (for discord)
    #[serde(default)]
    pub guild_id: Option<String>,

    /// Channel IDs (for discord)
    #[serde(default)]
    pub channels: Vec<u64>,
}

/// Global resource management config
#[derive(Debug, Deserialize)]
pub struct ResourcesConfig {
    #[serde(default = "default_reserve_vram_mb")]
    pub reserve_vram_mb: u64,

    #[serde(default = "default_reserve_ram_mb")]
    pub reserve_ram_mb: u64,

    #[serde(default = "default_llama_server_path")]
    pub llama_server_path: PathBuf,

    #[serde(default = "default_port_range")]
    pub port_range: String,
}

impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            reserve_vram_mb: default_reserve_vram_mb(),
            reserve_ram_mb: default_reserve_ram_mb(),
            llama_server_path: default_llama_server_path(),
            port_range: default_port_range(),
        }
    }
}

// Defaults
fn default_port() -> u16 {
    5000
}
fn default_context_limit() -> u64 {
    128_000
}
fn default_compaction_threshold() -> f64 {
    0.80
}
fn default_fill_target() -> f64 {
    0.40
}
fn default_min_messages() -> u32 {
    20
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_reserve_vram_mb() -> u64 {
    500
}
fn default_reserve_ram_mb() -> u64 {
    2000
}
fn default_llama_server_path() -> PathBuf {
    PathBuf::from("llama-server")
}
fn default_port_range() -> String {
    "8080-8180".to_string()
}

impl ModelConfig {
    /// Returns true if this is an embedding model
    pub fn is_embedding(&self) -> bool {
        self.dimensions.is_some()
    }

    /// Returns true if this is a local GGUF model
    pub fn is_gguf(&self) -> bool {
        self.provider == "gguf"
    }
}

impl AdapterConfig {
    /// Get the binary path, defaulting to river-{type}
    pub fn bin_path(&self) -> PathBuf {
        self.bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("river-{}", self.adapter_type)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_config() {
        let json = r#"{
            "port": 5000,
            "models": {
                "claude": {
                    "provider": "anthropic",
                    "endpoint": "https://api.anthropic.com/v1",
                    "name": "claude-sonnet-4-20250514",
                    "api_key_file": "/run/secrets/key",
                    "context_limit": 200000
                },
                "local": {
                    "provider": "gguf",
                    "path": "/models/test.gguf",
                    "context_limit": 32000
                }
            },
            "agents": {
                "iris": {
                    "workspace": "/home/test/stream",
                    "data_dir": "/var/lib/river/iris",
                    "port": 3000,
                    "model": "claude",
                    "adapters": []
                }
            }
        }"#;

        let config: RiverConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 5000);
        assert_eq!(config.models.len(), 2);
        assert!(config.models["local"].is_gguf());
        assert!(!config.models["claude"].is_gguf());
        assert_eq!(config.agents["iris"].port, 3000);
    }

    #[test]
    fn test_defaults_applied() {
        let json = r#"{
            "agents": {
                "test": {
                    "workspace": "/tmp/ws",
                    "data_dir": "/tmp/data",
                    "port": 3000,
                    "model": "m"
                }
            }
        }"#;

        let config: RiverConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 5000);
        assert_eq!(config.agents["test"].context.limit, 128_000);
        assert_eq!(config.agents["test"].context.compaction_threshold, 0.80);
        assert_eq!(config.agents["test"].log.level, "info");
        assert_eq!(config.resources.reserve_vram_mb, 500);
    }

    #[test]
    fn test_adapter_bin_path_default() {
        let adapter = AdapterConfig {
            adapter_type: "discord".to_string(),
            bin: None,
            port: 8081,
            token_file: None,
            token_env: None,
            guild_id: None,
            channels: vec![],
        };
        assert_eq!(adapter.bin_path(), PathBuf::from("river-discord"));
    }

    #[test]
    fn test_adapter_bin_path_custom() {
        let adapter = AdapterConfig {
            adapter_type: "discord".to_string(),
            bin: Some(PathBuf::from("/usr/local/bin/my-discord")),
            port: 8081,
            token_file: None,
            token_env: None,
            guild_id: None,
            channels: vec![],
        };
        assert_eq!(
            adapter.bin_path(),
            PathBuf::from("/usr/local/bin/my-discord")
        );
    }

    #[test]
    fn test_model_is_embedding() {
        let embed = ModelConfig {
            provider: "ollama".to_string(),
            endpoint: Some("http://localhost:11434/v1".to_string()),
            name: Some("nomic".to_string()),
            api_key_file: None,
            context_limit: None,
            path: None,
            dimensions: Some(768),
        };
        assert!(embed.is_embedding());
        assert!(!embed.is_gguf());
    }

    #[test]
    fn test_parse_example_config() {
        // Set required env vars for expansion
        unsafe { std::env::set_var("DISCORD_GUILD_ID", "999888777") };

        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/river.example.json"
        ))
        .unwrap();
        let expanded = crate::env::expand_vars(&raw).unwrap();
        let config: RiverConfig = serde_json::from_str(&expanded).unwrap();

        assert_eq!(config.port, 5000);
        assert_eq!(config.models.len(), 2);
        assert!(config.models.contains_key("claude-sonnet"));
        assert!(config.models["nomic-embed"].is_embedding());
        assert_eq!(config.agents.len(), 1);
        assert!(config.agents.contains_key("iris"));

        let iris = &config.agents["iris"];
        assert_eq!(iris.port, 3000);
        assert_eq!(iris.context.limit, 200000);
        assert_eq!(iris.context.compaction_threshold, 0.80);
        assert_eq!(iris.adapters.len(), 1);
        assert_eq!(iris.adapters[0].adapter_type, "discord");
        assert_eq!(iris.adapters[0].guild_id.as_deref(), Some("999888777"));

        // Validate
        let errors = crate::validate::validate(&config);
        assert!(errors.is_empty(), "Validation errors: {:?}", errors);

        unsafe { std::env::remove_var("DISCORD_GUILD_ID") };
    }

    #[test]
    fn test_parse_discord_adapter() {
        let json = r#"{
            "type": "discord",
            "port": 8081,
            "token_file": "/run/secrets/discord",
            "guild_id": "123456",
            "channels": [111, 222, 333]
        }"#;

        let adapter: AdapterConfig = serde_json::from_str(json).unwrap();
        assert_eq!(adapter.adapter_type, "discord");
        assert_eq!(adapter.channels, vec![111, 222, 333]);
        assert_eq!(adapter.guild_id.unwrap(), "123456");
    }
}

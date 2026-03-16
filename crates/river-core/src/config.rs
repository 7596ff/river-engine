//! Configuration types for River Engine
//!
//! This module defines the configuration structures used to configure
//! River Engine agents, orchestrators, and related components.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default heartbeat interval in minutes.
fn default_heartbeat_minutes() -> u32 {
    45
}

/// Default TTL for auto-embedded documents in days.
fn default_ttl_days() -> u32 {
    14
}

/// Configuration for a River Engine agent.
///
/// This struct contains all the settings needed to initialize and run
/// a River Engine agent, including model settings, paths, and networking.
///
/// # Examples
///
/// ```
/// use river_core::AgentConfig;
/// use std::path::PathBuf;
///
/// let config = AgentConfig {
///     name: "my-agent".to_string(),
///     workspace: PathBuf::from("/workspace"),
///     data_dir: PathBuf::from("/data"),
///     primary_model: "claude-3-opus".to_string(),
///     context_limit: 200_000,
///     gateway_port: 8080,
///     auth_token_file: None,
///     heartbeat: Default::default(),
///     embedding: Default::default(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique name for this agent instance
    pub name: String,

    /// Root directory for agent workspace operations
    pub workspace: PathBuf,

    /// Directory for storing agent data (database, embeddings, etc.)
    pub data_dir: PathBuf,

    /// The primary model identifier to use for inference
    pub primary_model: String,

    /// Maximum context window size in tokens
    pub context_limit: u64,

    /// Port number for the gateway server
    pub gateway_port: u16,

    /// Optional path to file containing authentication token
    pub auth_token_file: Option<PathBuf>,

    /// Heartbeat configuration for session keepalive
    #[serde(default)]
    pub heartbeat: HeartbeatConfig,

    /// Embedding configuration for document indexing
    #[serde(default)]
    pub embedding: EmbeddingConfig,
}

/// Configuration for agent heartbeat/keepalive behavior.
///
/// The heartbeat mechanism ensures sessions remain active and
/// allows the orchestrator to detect unresponsive agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// Default interval between heartbeats in minutes
    #[serde(default = "default_heartbeat_minutes")]
    pub default_minutes: u32,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            default_minutes: default_heartbeat_minutes(),
        }
    }
}

/// Configuration for document embedding and indexing.
///
/// Controls how and when documents are automatically embedded
/// for semantic search capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Number of days before auto-embedded documents expire
    #[serde(default = "default_ttl_days")]
    pub auto_embed_ttl_days: u32,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            auto_embed_ttl_days: default_ttl_days(),
        }
    }
}

/// Configuration for the River Engine orchestrator.
///
/// The orchestrator coordinates multiple agents and manages
/// model resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Port number for the orchestrator server
    pub port: u16,

    /// Directory containing model files and configurations
    pub models_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_config_default() {
        let config = HeartbeatConfig::default();
        assert_eq!(config.default_minutes, 45);
    }

    #[test]
    fn test_embedding_config_default() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.auto_embed_ttl_days, 14);
    }

    #[test]
    fn test_agent_config_serialize() {
        let config = AgentConfig {
            name: "test-agent".to_string(),
            workspace: PathBuf::from("/workspace"),
            data_dir: PathBuf::from("/data"),
            primary_model: "claude-3-opus".to_string(),
            context_limit: 200_000,
            gateway_port: 8080,
            auth_token_file: None,
            heartbeat: HeartbeatConfig::default(),
            embedding: EmbeddingConfig::default(),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"name\":\"test-agent\""));
        assert!(json.contains("\"context_limit\":200000"));
    }

    #[test]
    fn test_agent_config_deserialize_with_defaults() {
        let json = r#"{
            "name": "test-agent",
            "workspace": "/workspace",
            "data_dir": "/data",
            "primary_model": "claude-3-opus",
            "context_limit": 200000,
            "gateway_port": 8080,
            "auth_token_file": null
        }"#;

        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "test-agent");
        assert_eq!(config.workspace, PathBuf::from("/workspace"));
        assert_eq!(config.data_dir, PathBuf::from("/data"));
        assert_eq!(config.primary_model, "claude-3-opus");
        assert_eq!(config.context_limit, 200_000);
        assert_eq!(config.gateway_port, 8080);
        assert!(config.auth_token_file.is_none());
        // These should use defaults
        assert_eq!(config.heartbeat.default_minutes, 45);
        assert_eq!(config.embedding.auto_embed_ttl_days, 14);
    }

    #[test]
    fn test_agent_config_deserialize_with_custom_values() {
        let json = r#"{
            "name": "custom-agent",
            "workspace": "/custom/workspace",
            "data_dir": "/custom/data",
            "primary_model": "gpt-4",
            "context_limit": 128000,
            "gateway_port": 9090,
            "auth_token_file": "/path/to/token",
            "heartbeat": {
                "default_minutes": 30
            },
            "embedding": {
                "auto_embed_ttl_days": 7
            }
        }"#;

        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "custom-agent");
        assert_eq!(config.heartbeat.default_minutes, 30);
        assert_eq!(config.embedding.auto_embed_ttl_days, 7);
        assert_eq!(
            config.auth_token_file,
            Some(PathBuf::from("/path/to/token"))
        );
    }

    #[test]
    fn test_agent_config_roundtrip() {
        let config = AgentConfig {
            name: "roundtrip-agent".to_string(),
            workspace: PathBuf::from("/workspace"),
            data_dir: PathBuf::from("/data"),
            primary_model: "model".to_string(),
            context_limit: 100_000,
            gateway_port: 8000,
            auth_token_file: Some(PathBuf::from("/token")),
            heartbeat: HeartbeatConfig { default_minutes: 60 },
            embedding: EmbeddingConfig {
                auto_embed_ttl_days: 30,
            },
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AgentConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.name, deserialized.name);
        assert_eq!(config.workspace, deserialized.workspace);
        assert_eq!(config.data_dir, deserialized.data_dir);
        assert_eq!(config.primary_model, deserialized.primary_model);
        assert_eq!(config.context_limit, deserialized.context_limit);
        assert_eq!(config.gateway_port, deserialized.gateway_port);
        assert_eq!(config.auth_token_file, deserialized.auth_token_file);
        assert_eq!(
            config.heartbeat.default_minutes,
            deserialized.heartbeat.default_minutes
        );
        assert_eq!(
            config.embedding.auto_embed_ttl_days,
            deserialized.embedding.auto_embed_ttl_days
        );
    }

    #[test]
    fn test_orchestrator_config_serialize() {
        let config = OrchestratorConfig {
            port: 9000,
            models_dir: PathBuf::from("/models"),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"port\":9000"));
        assert!(json.contains("/models"));
    }

    #[test]
    fn test_orchestrator_config_deserialize() {
        let json = r#"{
            "port": 9000,
            "models_dir": "/models"
        }"#;

        let config: OrchestratorConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 9000);
        assert_eq!(config.models_dir, PathBuf::from("/models"));
    }

    #[test]
    fn test_orchestrator_config_roundtrip() {
        let config = OrchestratorConfig {
            port: 8888,
            models_dir: PathBuf::from("/path/to/models"),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: OrchestratorConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.port, deserialized.port);
        assert_eq!(config.models_dir, deserialized.models_dir);
    }

    #[test]
    fn test_heartbeat_config_partial_deserialize() {
        // Test that missing fields use defaults
        let json = r#"{}"#;
        let config: HeartbeatConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.default_minutes, 45);
    }

    #[test]
    fn test_embedding_config_partial_deserialize() {
        // Test that missing fields use defaults
        let json = r#"{}"#;
        let config: EmbeddingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.auto_embed_ttl_days, 14);
    }
}

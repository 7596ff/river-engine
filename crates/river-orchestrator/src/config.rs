//! Configuration loading with env var substitution.

use river_adapter::{Ground, Side};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Root configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub models: HashMap<String, ModelConfig>,
    pub embed: Option<EmbedConfig>,
    pub dyads: HashMap<String, DyadConfig>,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    4000
}

/// Model configuration for LLMs or embedding models.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,
    pub context_limit: Option<usize>,
    pub dimensions: Option<usize>,
}

/// Embed service configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbedConfig {
    pub model: String,
}

/// Dyad configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DyadConfig {
    pub workspace: PathBuf,
    pub left_model: String,
    pub right_model: String,
    pub initial_actor: Side,
    pub ground: Ground,
    pub adapters: Vec<AdapterConfig>,
}

/// Adapter configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AdapterConfig {
    #[serde(rename = "type")]
    pub adapter_type: String,
    pub binary: String,
    pub side: river_adapter::Side,
    #[serde(default)]
    pub config: Value,
}

/// Configuration error.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Json(serde_json::Error),
    MissingEnvVar(String),
    UnknownModel { reference: String, context: String },
    MissingContextLimit { model: String },
    MissingDimensions { model: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "IO error: {}", e),
            ConfigError::Json(e) => write!(f, "JSON error: {}", e),
            ConfigError::MissingEnvVar(var) => write!(f, "Missing environment variable: {}", var),
            ConfigError::UnknownModel { reference, context } => {
                write!(f, "Unknown model '{}' referenced in {}", reference, context)
            }
            ConfigError::MissingContextLimit { model } => {
                write!(f, "Model '{}' is used as LLM but missing context_limit", model)
            }
            ConfigError::MissingDimensions { model } => {
                write!(f, "Model '{}' is used for embeddings but missing dimensions", model)
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(e: serde_json::Error) -> Self {
        ConfigError::Json(e)
    }
}

/// Load configuration from file with env var substitution.
pub fn load_config(path: &std::path::Path) -> Result<Config, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let resolved = substitute_env_vars(&content)?;
    let config: Config = serde_json::from_str(&resolved)?;
    validate_config(&config)?;
    Ok(config)
}

/// Substitute $VAR_NAME patterns with environment variable values.
fn substitute_env_vars(content: &str) -> Result<String, ConfigError> {
    let re = regex::Regex::new(r"\$([A-Z_][A-Z0-9_]*)").unwrap();
    let mut result = content.to_string();
    let mut missing_vars = Vec::new();

    for cap in re.captures_iter(content) {
        let var_name = &cap[1];
        match std::env::var(var_name) {
            Ok(value) => {
                let pattern = format!("${}", var_name);
                result = result.replace(&pattern, &value);
            }
            Err(_) => {
                missing_vars.push(var_name.to_string());
            }
        }
    }

    if !missing_vars.is_empty() {
        return Err(ConfigError::MissingEnvVar(missing_vars.join(", ")));
    }

    Ok(result)
}

/// Validate configuration consistency.
fn validate_config(config: &Config) -> Result<(), ConfigError> {
    // Check embed model reference
    if let Some(ref embed) = config.embed {
        let model = config.models.get(&embed.model).ok_or_else(|| ConfigError::UnknownModel {
            reference: embed.model.clone(),
            context: "embed.model".into(),
        })?;
        if model.dimensions.is_none() {
            return Err(ConfigError::MissingDimensions {
                model: embed.model.clone(),
            });
        }
    }

    // Check dyad model references
    for (dyad_name, dyad) in &config.dyads {
        let left_model = config.models.get(&dyad.left_model).ok_or_else(|| ConfigError::UnknownModel {
            reference: dyad.left_model.clone(),
            context: format!("dyads.{}.left_model", dyad_name),
        })?;
        if left_model.context_limit.is_none() {
            return Err(ConfigError::MissingContextLimit {
                model: dyad.left_model.clone(),
            });
        }

        let right_model = config.models.get(&dyad.right_model).ok_or_else(|| ConfigError::UnknownModel {
            reference: dyad.right_model.clone(),
            context: format!("dyads.{}.right_model", dyad_name),
        })?;
        if right_model.context_limit.is_none() {
            return Err(ConfigError::MissingContextLimit {
                model: dyad.right_model.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_substitution() {
        std::env::set_var("TEST_API_KEY", "secret123");
        let content = r#"{"key": "$TEST_API_KEY"}"#;
        let result = substitute_env_vars(content).unwrap();
        assert_eq!(result, r#"{"key": "secret123"}"#);
        std::env::remove_var("TEST_API_KEY");
    }

    #[test]
    fn test_missing_env_var() {
        let content = r#"{"key": "$NONEXISTENT_VAR_12345"}"#;
        let result = substitute_env_vars(content);
        assert!(matches!(result, Err(ConfigError::MissingEnvVar(_))));
    }
}

//! Model config resolution and switching.

use crate::config::{Config, ModelConfig};
use river_adapter::Side;
use serde::{Deserialize, Serialize};

/// Model switch request from worker.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelSwitchRequest {
    pub dyad: String,
    pub side: Side,
    pub model: String,
}

/// Model switch response to worker.
#[derive(Debug, Clone, Serialize)]
pub struct ModelSwitchResponse {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,
    pub context_limit: usize,
}

/// Model switch error response.
#[derive(Debug, Clone, Serialize)]
pub struct ModelSwitchError {
    pub error: String,
}

/// Resolve a model by name from config.
pub fn resolve_model<'a>(config: &'a Config, model_name: &str) -> Option<&'a ModelConfig> {
    config.models.get(model_name)
}

/// Get the model config for a worker.
pub fn get_worker_model<'a>(config: &'a Config, dyad: &str, side: &Side) -> Option<&'a ModelConfig> {
    let dyad_config = config.dyads.get(dyad)?;
    let model_name = match side {
        Side::Left => &dyad_config.left_model,
        Side::Right => &dyad_config.right_model,
    };
    config.models.get(model_name)
}

/// Get the model name for a worker.
pub fn get_worker_model_name(config: &Config, dyad: &str, side: &Side) -> Option<String> {
    let dyad_config = config.dyads.get(dyad)?;
    Some(match side {
        Side::Left => dyad_config.left_model.clone(),
        Side::Right => dyad_config.right_model.clone(),
    })
}

/// Get the embed model config.
pub fn get_embed_model(config: &Config) -> Option<&ModelConfig> {
    let embed_config = config.embed.as_ref()?;
    config.models.get(&embed_config.model)
}

/// Convert ModelConfig to ModelSwitchResponse.
impl From<&ModelConfig> for ModelSwitchResponse {
    fn from(model: &ModelConfig) -> Self {
        Self {
            endpoint: model.endpoint.clone(),
            name: model.name.clone(),
            api_key: model.api_key.clone(),
            context_limit: model.context_limit.unwrap_or(8192),
        }
    }
}

//! Model config resolution and switching.

use crate::config::ModelDefinition;
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

/// Convert ModelDefinition to ModelSwitchResponse.
impl From<&ModelDefinition> for ModelSwitchResponse {
    fn from(model: &ModelDefinition) -> Self {
        Self {
            endpoint: model.endpoint.clone(),
            name: model.name.clone(),
            api_key: model.api_key.clone(),
            context_limit: model.context_limit.unwrap_or(8192),
        }
    }
}

//! Model management tools for orchestrator integration
//!
//! These tools allow the agent to request, release, and switch models
//! through the orchestrator service.

use crate::tools::{Tool, ToolResult};
use river_core::RiverError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Model management client configuration
#[derive(Debug, Clone)]
pub struct ModelManagerConfig {
    /// Orchestrator URL
    pub orchestrator_url: String,
    /// Request timeout
    pub timeout: Duration,
}

impl Default for ModelManagerConfig {
    fn default() -> Self {
        Self {
            orchestrator_url: "http://localhost:5000".to_string(),
            timeout: Duration::from_secs(120),
        }
    }
}

/// Shared state for model management
#[derive(Debug)]
pub struct ModelManagerState {
    /// Currently active model endpoint
    pub active_model_endpoint: Option<String>,
    /// Currently active model name
    pub active_model_name: Option<String>,
}

impl Default for ModelManagerState {
    fn default() -> Self {
        Self {
            active_model_endpoint: None,
            active_model_name: None,
        }
    }
}

/// Response from orchestrator model request
#[derive(Debug, Deserialize)]
struct ModelRequestResponse {
    status: String,
    model: String,
    endpoint: Option<String>,
    device: Option<String>,
    warning: Option<String>,
    error: Option<String>,
}

/// Response from orchestrator model release
#[derive(Debug, Deserialize)]
struct ModelReleaseResponse {
    acknowledged: bool,
}

/// Request a model from the orchestrator
pub struct RequestModelTool {
    config: ModelManagerConfig,
    state: Arc<RwLock<ModelManagerState>>,
    http_client: reqwest::Client,
}

impl RequestModelTool {
    pub fn new(config: ModelManagerConfig, state: Arc<RwLock<ModelManagerState>>) -> Self {
        Self {
            http_client: reqwest::Client::builder()
                .timeout(config.timeout)
                .build()
                .unwrap_or_default(),
            config,
            state,
        }
    }
}

impl Tool for RequestModelTool {
    fn name(&self) -> &str {
        "request_model"
    }

    fn description(&self) -> &str {
        "Request a model from the orchestrator"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "description": "Model ID to request"
                },
                "priority": {
                    "type": "string",
                    "enum": ["interactive", "scheduled", "background"],
                    "description": "Request priority (default: interactive)",
                    "default": "interactive"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "How long to wait for model to load (default: 120)",
                    "default": 120
                }
            },
            "required": ["model"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let model = args["model"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'model' parameter"))?;
        let priority = args["priority"].as_str().unwrap_or("interactive");
        let timeout_seconds = args["timeout_seconds"].as_u64().unwrap_or(120) as u32;

        let url = format!("{}/model/request", self.config.orchestrator_url);
        let state = self.state.clone();
        let http_client = self.http_client.clone();
        let model = model.to_string();
        let priority = priority.to_string();

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let payload = serde_json::json!({
                    "model": model,
                    "priority": priority,
                    "timeout_seconds": timeout_seconds
                });

                let response = http_client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| RiverError::tool(format!("Failed to contact orchestrator: {}", e)))?;

                let resp: ModelRequestResponse = response
                    .json()
                    .await
                    .map_err(|e| RiverError::tool(format!("Invalid response from orchestrator: {}", e)))?;

                match resp.status.as_str() {
                    "ready" => {
                        let endpoint = resp.endpoint.clone().unwrap_or_default();

                        // Update active model state
                        {
                            let mut state = state.write().await;
                            state.active_model_endpoint = resp.endpoint.clone();
                            state.active_model_name = Some(model.clone());
                        }

                        let mut output = format!(
                            "Model '{}' ready at {}\nDevice: {}",
                            model,
                            endpoint,
                            resp.device.unwrap_or_else(|| "unknown".to_string())
                        );

                        if let Some(warning) = resp.warning {
                            output.push_str(&format!("\nWarning: {}", warning));
                        }

                        Ok(ToolResult::success(output))
                    }
                    "loading" => {
                        Ok(ToolResult::success(format!(
                            "Model '{}' is loading. Try again in a moment.",
                            model
                        )))
                    }
                    "error" => {
                        Err(RiverError::tool(format!(
                            "Failed to load model: {}",
                            resp.error.unwrap_or_else(|| "unknown error".to_string())
                        )))
                    }
                    _ => {
                        Err(RiverError::tool(format!(
                            "Unknown response status: {}",
                            resp.status
                        )))
                    }
                }
            })
        });

        result
    }
}

/// Release a model back to the orchestrator
pub struct ReleaseModelTool {
    config: ModelManagerConfig,
    state: Arc<RwLock<ModelManagerState>>,
    http_client: reqwest::Client,
}

impl ReleaseModelTool {
    pub fn new(config: ModelManagerConfig, state: Arc<RwLock<ModelManagerState>>) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            config,
            state,
        }
    }
}

impl Tool for ReleaseModelTool {
    fn name(&self) -> &str {
        "release_model"
    }

    fn description(&self) -> &str {
        "Release a model back to the orchestrator for potential eviction"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "description": "Model ID to release"
                }
            },
            "required": ["model"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let model = args["model"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'model' parameter"))?;

        let url = format!("{}/model/release", self.config.orchestrator_url);
        let state = self.state.clone();
        let http_client = self.http_client.clone();
        let model = model.to_string();

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let payload = serde_json::json!({
                    "model": model
                });

                let response = http_client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| RiverError::tool(format!("Failed to contact orchestrator: {}", e)))?;

                let resp: ModelReleaseResponse = response
                    .json()
                    .await
                    .map_err(|e| RiverError::tool(format!("Invalid response from orchestrator: {}", e)))?;

                // Clear active model if it was the released one
                {
                    let mut state = state.write().await;
                    if state.active_model_name.as_ref() == Some(&model) {
                        state.active_model_endpoint = None;
                        state.active_model_name = None;
                    }
                }

                if resp.acknowledged {
                    Ok(ToolResult::success(format!(
                        "Model '{}' marked as releasable. Orchestrator may evict it when resources are needed.",
                        model
                    )))
                } else {
                    Ok(ToolResult::success(format!(
                        "Model '{}' not found or already released.",
                        model
                    )))
                }
            })
        });

        result
    }
}

/// Switch the active model for the session
pub struct SwitchModelTool {
    state: Arc<RwLock<ModelManagerState>>,
}

impl SwitchModelTool {
    pub fn new(state: Arc<RwLock<ModelManagerState>>) -> Self {
        Self { state }
    }
}

impl Tool for SwitchModelTool {
    fn name(&self) -> &str {
        "switch_model"
    }

    fn description(&self) -> &str {
        "Switch the active model for this session (use after request_model)"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "description": "Model name to switch to"
                },
                "endpoint": {
                    "type": "string",
                    "description": "Model endpoint URL"
                }
            },
            "required": ["model", "endpoint"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let model = args["model"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'model' parameter"))?;
        let endpoint = args["endpoint"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'endpoint' parameter"))?;

        // Use try_write to avoid blocking - this is a quick state update
        let state = self.state.clone();
        let model = model.to_string();
        let endpoint = endpoint.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut state = state.write().await;
                let previous = state.active_model_name.clone();

                state.active_model_name = Some(model.clone());
                state.active_model_endpoint = Some(endpoint.clone());

                let output = if let Some(prev) = previous {
                    format!("Switched from '{}' to '{}' at {}", prev, model, endpoint)
                } else {
                    format!("Set active model to '{}' at {}", model, endpoint)
                };

                Ok(ToolResult::success(output))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_model_tool_schema() {
        let config = ModelManagerConfig::default();
        let state = Arc::new(RwLock::new(ModelManagerState::default()));
        let tool = RequestModelTool::new(config, state);

        assert_eq!(tool.name(), "request_model");
        let params = tool.parameters();
        assert!(params["properties"]["model"].is_object());
        assert!(params["properties"]["priority"].is_object());
    }

    #[test]
    fn test_release_model_tool_schema() {
        let config = ModelManagerConfig::default();
        let state = Arc::new(RwLock::new(ModelManagerState::default()));
        let tool = ReleaseModelTool::new(config, state);

        assert_eq!(tool.name(), "release_model");
        let params = tool.parameters();
        assert!(params["properties"]["model"].is_object());
    }

    #[test]
    fn test_switch_model_tool_schema() {
        let state = Arc::new(RwLock::new(ModelManagerState::default()));
        let tool = SwitchModelTool::new(state);

        assert_eq!(tool.name(), "switch_model");
        let params = tool.parameters();
        assert!(params["properties"]["model"].is_object());
        assert!(params["properties"]["endpoint"].is_object());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_switch_model_updates_state() {
        let state = Arc::new(RwLock::new(ModelManagerState::default()));
        let tool = SwitchModelTool::new(state.clone());

        let result = tool.execute(serde_json::json!({
            "model": "test-model",
            "endpoint": "http://localhost:8080"
        }));

        assert!(result.is_ok());

        let state = state.read().await;
        assert_eq!(state.active_model_name, Some("test-model".to_string()));
        assert_eq!(state.active_model_endpoint, Some("http://localhost:8080".to_string()));
    }
}

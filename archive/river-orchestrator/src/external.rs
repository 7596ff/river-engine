//! External model configuration (LiteLLM)

use serde::{Deserialize, Serialize};

/// An external model accessible via LiteLLM or similar proxy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalModel {
    pub id: String,
    pub provider: String,
    pub litellm_model: String,
    pub api_base: String,
}

impl ExternalModel {
    /// Get the chat completions endpoint
    pub fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.api_base)
    }
}

/// External models configuration file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalModelsFile {
    pub external_models: Vec<ExternalModel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_external_model_endpoint() {
        let model = ExternalModel {
            id: "claude-sonnet".to_string(),
            provider: "litellm".to_string(),
            litellm_model: "claude-sonnet-4-20250514".to_string(),
            api_base: "http://localhost:4000".to_string(),
        };
        assert_eq!(model.endpoint(), "http://localhost:4000/v1/chat/completions");
    }

    #[test]
    fn test_external_models_deserialize() {
        let json = r#"{
            "external_models": [
                {
                    "id": "claude-sonnet",
                    "provider": "litellm",
                    "litellm_model": "claude-sonnet-4-20250514",
                    "api_base": "http://localhost:4000"
                }
            ]
        }"#;
        let file: ExternalModelsFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.external_models.len(), 1);
        assert_eq!(file.external_models[0].id, "claude-sonnet");
    }
}

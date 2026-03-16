//! Static model registry

use serde::Serialize;

/// Model provider type
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModelProvider {
    Local,
    LiteLLM,
}

impl From<&str> for ModelProvider {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "local" => ModelProvider::Local,
            "litellm" => ModelProvider::LiteLLM,
            _ => ModelProvider::Local, // default
        }
    }
}

/// Model availability status
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModelStatus {
    Available,
    Unavailable,
}

/// Information about a configured model
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub provider: ModelProvider,
    pub status: ModelStatus,
}

impl ModelInfo {
    pub fn new(name: String, provider: ModelProvider) -> Self {
        Self {
            name,
            provider,
            status: ModelStatus::Available,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_creation() {
        let model = ModelInfo::new("qwen3-32b".to_string(), ModelProvider::Local);
        assert_eq!(model.name, "qwen3-32b");
        assert!(matches!(model.provider, ModelProvider::Local));
        assert!(matches!(model.status, ModelStatus::Available));
    }

    #[test]
    fn test_model_provider_from_str() {
        assert!(matches!(ModelProvider::from("local"), ModelProvider::Local));
        assert!(matches!(ModelProvider::from("litellm"), ModelProvider::LiteLLM));
        assert!(matches!(ModelProvider::from("LOCAL"), ModelProvider::Local));
        assert!(matches!(ModelProvider::from("unknown"), ModelProvider::Local));
    }

    #[test]
    fn test_model_info_serialize() {
        let model = ModelInfo::new("test".to_string(), ModelProvider::LiteLLM);
        let json = serde_json::to_string(&model).unwrap();
        assert!(json.contains("\"provider\":\"litellm\""));
        assert!(json.contains("\"status\":\"available\""));
    }
}

//! Model client for OpenAI-compatible API

use crate::r#loop::context::{ChatMessage, ToolCallRequest, FunctionCall};
use crate::tools::ToolSchema;
use river_core::RiverError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Client for calling OpenAI-compatible model servers
pub struct ModelClient {
    url: String,
    model: String,
    http: reqwest::Client,
}

impl ModelClient {
    pub fn new(url: String, model: String, timeout: Duration) -> Result<Self, RiverError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| RiverError::config(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { url, model, http })
    }

    /// Call the model with the current context
    pub async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> Result<ModelResponse, RiverError> {
        let request = ChatCompletionRequest {
            model: &self.model,
            messages,
            tools: if tools.is_empty() { None } else { Some(tools) },
        };

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.url))
            .json(&request)
            .send()
            .await
            .map_err(|e| RiverError::model(format!("HTTP error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RiverError::model(format!(
                "API error {}: {}",
                status, body
            )));
        }

        let completion: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| RiverError::model(format!("JSON parse error: {}", e)))?;

        ModelResponse::from_completion(completion)
    }

    /// Get the model URL
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Get the model name
    pub fn model(&self) -> &str {
        &self.model
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolSchema]>,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallRaw>>,
}

#[derive(Deserialize)]
struct ToolCallRaw {
    id: String,
    r#type: Option<String>,
    function: FunctionCallRaw,
}

#[derive(Deserialize)]
struct FunctionCallRaw {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Parsed model response
#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub usage: Usage,
}

impl ModelResponse {
    fn from_completion(resp: ChatCompletionResponse) -> Result<Self, RiverError> {
        let choice = resp.choices.into_iter().next().ok_or_else(|| {
            RiverError::model("API response contained no choices".to_string())
        })?;

        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCallRequest {
                id: tc.id,
                r#type: tc.r#type.unwrap_or_else(|| "function".to_string()),
                function: FunctionCall {
                    name: tc.function.name,
                    arguments: tc.function.arguments,
                },
            })
            .collect();

        Ok(Self {
            content: choice.message.content,
            tool_calls,
            usage: resp.usage,
        })
    }

    /// Check if response has tool calls
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_response_has_tool_calls() {
        let resp = ModelResponse {
            content: Some("Hello".to_string()),
            tool_calls: vec![],
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        };
        assert!(!resp.has_tool_calls());

        let resp_with_tools = ModelResponse {
            content: None,
            tool_calls: vec![ToolCallRequest {
                id: "call_1".to_string(),
                r#type: "function".to_string(),
                function: FunctionCall {
                    name: "read".to_string(),
                    arguments: "{}".to_string(),
                },
            }],
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
        };
        assert!(resp_with_tools.has_tool_calls());
    }

    #[test]
    fn test_model_client_creation() {
        let client = ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        )
        .expect("test client creation should succeed");
        assert_eq!(client.url(), "http://localhost:8080");
        assert_eq!(client.model(), "test-model");
    }

    #[test]
    fn test_chat_completion_request_serialization() {
        let messages = vec![ChatMessage::user("Hello")];
        let request = ChatCompletionRequest {
            model: "test",
            messages: &messages,
            tools: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"model\":\"test\""));
        assert!(json.contains("\"messages\""));
        // tools should be omitted when None
        assert!(!json.contains("\"tools\""));
    }
}

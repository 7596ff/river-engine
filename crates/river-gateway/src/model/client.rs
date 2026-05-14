//! Model client supporting OpenAI-compatible and Anthropic APIs

use super::types::{ChatMessage, FunctionCall, ToolCallRequest};
use crate::tools::ToolSchema;
use river_core::RiverError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// API provider type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// OpenAI-compatible API (llama-server, OpenRouter, etc.)
    OpenAI,
    /// Native Anthropic Messages API
    Anthropic,
}

/// Client for calling model APIs
pub struct ModelClient {
    url: String,
    model: String,
    api_key: Option<String>,
    provider: Provider,
    http: reqwest::Client,
}

impl ModelClient {
    pub fn new(url: String, model: String, timeout: Duration) -> Result<Self, RiverError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| RiverError::config(format!("Failed to create HTTP client: {}", e)))?;

        // Detect provider based on URL
        let provider = if url.contains("api.anthropic.com") {
            Provider::Anthropic
        } else {
            Provider::OpenAI
        };

        // Check for API key in environment based on provider
        let api_key = match provider {
            Provider::Anthropic => std::env::var("ANTHROPIC_API_KEY").ok(),
            Provider::OpenAI => std::env::var("OPENROUTER_API_KEY").ok(),
        };

        if api_key.is_some() {
            tracing::info!(provider = ?provider, "API key found in environment");
        } else {
            let env_var = match provider {
                Provider::Anthropic => "ANTHROPIC_API_KEY",
                Provider::OpenAI => "OPENROUTER_API_KEY",
            };
            tracing::warn!("No API key found in {} environment variable(s)", env_var);
        }

        tracing::info!(provider = ?provider, url = %url, model = %model, "ModelClient initialized");

        Ok(Self {
            url,
            model,
            api_key,
            provider,
            http,
        })
    }

    /// Get the provider type
    pub fn provider(&self) -> Provider {
        self.provider
    }

    /// Call the model with the current context
    pub async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> Result<ModelResponse, RiverError> {
        tracing::info!(
            model = %self.model,
            url = %self.url,
            provider = ?self.provider,
            message_count = messages.len(),
            tool_count = tools.len(),
            "Calling model"
        );

        // Log last few messages for context
        for (i, msg) in messages.iter().rev().take(3).enumerate() {
            tracing::debug!(
                index = messages.len() - 1 - i,
                role = %msg.role,
                content_preview = %msg.content.as_deref().unwrap_or("").chars().take(100).collect::<String>(),
                has_tool_calls = msg.tool_calls.is_some(),
                "Message in context"
            );
        }

        match self.provider {
            Provider::OpenAI => self.complete_openai(messages, tools).await,
            Provider::Anthropic => self.complete_anthropic(messages, tools).await,
        }
    }

    /// OpenAI-compatible API completion
    async fn complete_openai(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> Result<ModelResponse, RiverError> {
        let openai_tools: Vec<OpenAITool> = tools.iter().map(OpenAITool::from_schema).collect();
        let request = ChatCompletionRequest {
            model: &self.model,
            messages,
            tools: if tools.is_empty() {
                None
            } else {
                Some(openai_tools)
            },
        };

        tracing::debug!(
            request_json = %serde_json::to_string(&request).unwrap_or_default().chars().take(1000).collect::<String>(),
            "Sending OpenAI request"
        );

        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.url))
            .json(&request);

        if let Some(ref api_key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = req.send().await.map_err(|e| {
            tracing::error!(error = %e, "HTTP request to model failed");
            RiverError::model(format!("HTTP error: {}", e))
        })?;

        let status = response.status();
        tracing::debug!(status = %status, "Received response from model");

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!(status = %status, body = %body, "Model API returned error");
            return Err(RiverError::model_api(status.as_u16(), body));
        }

        let body = response.text().await.map_err(|e| {
            tracing::error!(error = %e, "Failed to read response body");
            RiverError::model(format!("Failed to read response: {}", e))
        })?;

        tracing::debug!(
            body_preview = %body.chars().take(500).collect::<String>(),
            body_len = body.len(),
            "Response body received"
        );

        let completion: ChatCompletionResponse = serde_json::from_str(&body).map_err(|e| {
            tracing::error!(error = %e, body = %body, "Failed to parse model response JSON");
            RiverError::model(format!("JSON parse error: {}", e))
        })?;

        let result = ModelResponse::from_openai_completion(completion)?;
        self.log_response(&result);
        Ok(result)
    }

    /// Anthropic Messages API completion with ephemeral caching
    async fn complete_anthropic(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> Result<ModelResponse, RiverError> {
        // Extract system message and convert remaining messages
        let (system, anthropic_messages) = self.convert_to_anthropic_messages(messages);

        // Convert tools to Anthropic format
        let anthropic_tools: Vec<AnthropicTool> =
            tools.iter().map(AnthropicTool::from_schema).collect();

        let request = AnthropicRequest {
            model: &self.model,
            max_tokens: 8192,
            system,
            messages: anthropic_messages,
            tools: if tools.is_empty() {
                None
            } else {
                Some(anthropic_tools)
            },
        };

        tracing::debug!(
            request_json = %serde_json::to_string(&request).unwrap_or_default().chars().take(1000).collect::<String>(),
            "Sending Anthropic request"
        );

        let mut req = self
            .http
            .post(format!("{}/messages", self.url))
            .header("content-type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .json(&request);

        if let Some(ref api_key) = self.api_key {
            req = req.header("x-api-key", api_key);
        }

        let response = req.send().await.map_err(|e| {
            tracing::error!(error = %e, "HTTP request to Anthropic failed");
            RiverError::model(format!("HTTP error: {}", e))
        })?;

        let status = response.status();
        tracing::debug!(status = %status, "Received response from Anthropic");

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!(status = %status, body = %body, "Anthropic API returned error");
            return Err(RiverError::model_api(status.as_u16(), body));
        }

        let body = response.text().await.map_err(|e| {
            tracing::error!(error = %e, "Failed to read response body");
            RiverError::model(format!("Failed to read response: {}", e))
        })?;

        tracing::debug!(
            body_preview = %body.chars().take(500).collect::<String>(),
            body_len = body.len(),
            "Response body received"
        );

        let completion: AnthropicResponse = serde_json::from_str(&body).map_err(|e| {
            tracing::error!(error = %e, body = %body, "Failed to parse Anthropic response JSON");
            RiverError::model(format!("JSON parse error: {}", e))
        })?;

        let result = ModelResponse::from_anthropic_response(completion)?;
        self.log_response(&result);
        Ok(result)
    }

    /// Convert internal messages to Anthropic format with cache_control
    /// Adds cache_control to first system prompt and last non-tool message (max 2 breakpoints)
    fn convert_to_anthropic_messages(
        &self,
        messages: &[ChatMessage],
    ) -> (Option<Vec<AnthropicContentBlock>>, Vec<AnthropicMessage>) {
        let mut system_content: Option<Vec<AnthropicContentBlock>> = None;
        let mut anthropic_messages: Vec<AnthropicMessage> = Vec::new();

        // Find the last non-tool message index for caching
        let last_cacheable_idx = messages
            .iter()
            .rposition(|m| m.role != "tool" && m.role != "system");

        for (i, msg) in messages.iter().enumerate() {
            let should_cache = Some(i) == last_cacheable_idx;

            if msg.role == "system" {
                // System messages go into the system field - cache the first one only
                if let Some(content) = &msg.content {
                    let is_first_system = system_content.is_none();
                    let block = AnthropicContentBlock::Text {
                        text: content.clone(),
                        cache_control: if is_first_system {
                            Some(CacheControl {
                                r#type: "ephemeral",
                            })
                        } else {
                            None
                        },
                    };
                    system_content.get_or_insert_with(Vec::new).push(block);
                }
            } else if msg.role == "assistant" {
                let mut content = Vec::new();

                if let Some(text) = &msg.content {
                    if !text.is_empty() {
                        content.push(AnthropicContentBlock::Text {
                            text: text.clone(),
                            cache_control: if should_cache {
                                Some(CacheControl {
                                    r#type: "ephemeral",
                                })
                            } else {
                                None
                            },
                        });
                    }
                }

                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        let input: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                        content.push(AnthropicContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            input,
                        });
                    }
                }

                if !content.is_empty() {
                    anthropic_messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                }
            } else if msg.role == "tool" {
                let result_content = msg.content.clone().unwrap_or_default();
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();

                anthropic_messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: vec![AnthropicContentBlock::ToolResult {
                        tool_use_id,
                        content: result_content,
                    }],
                });
            } else {
                // User messages
                if let Some(text) = &msg.content {
                    anthropic_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: vec![AnthropicContentBlock::Text {
                            text: text.clone(),
                            cache_control: if should_cache {
                                Some(CacheControl {
                                    r#type: "ephemeral",
                                })
                            } else {
                                None
                            },
                        }],
                    });
                }
            }
        }

        (system_content, anthropic_messages)
    }

    fn log_response(&self, result: &ModelResponse) {
        // Log cache stats for Anthropic
        if self.provider == Provider::Anthropic {
            let cache_hit_pct = if result.usage.prompt_tokens > 0 {
                (result.usage.cache_read_tokens as f64 / result.usage.prompt_tokens as f64) * 100.0
            } else {
                0.0
            };
            tracing::info!(
                usage_prompt = result.usage.prompt_tokens,
                usage_completion = result.usage.completion_tokens,
                cache_creation = result.usage.cache_creation_tokens,
                cache_read = result.usage.cache_read_tokens,
                cache_hit_pct = format!("{:.1}%", cache_hit_pct),
                "Anthropic response (cache stats)"
            );
        }

        tracing::info!(
            usage_prompt = result.usage.prompt_tokens,
            usage_completion = result.usage.completion_tokens,
            usage_total = result.usage.total_tokens,
            has_content = result.content.is_some(),
            tool_call_count = result.tool_calls.len(),
            "Model response parsed"
        );

        if let Some(ref content) = result.content {
            tracing::debug!(
                content_preview = %content.chars().take(200).collect::<String>(),
                "Model content"
            );
        }

        for tc in &result.tool_calls {
            tracing::info!(
                tool_call_id = %tc.id,
                tool_name = %tc.function.name,
                args_preview = %tc.function.arguments.chars().take(200).collect::<String>(),
                "Model requested tool call"
            );
        }
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

// ============================================================================
// OpenAI-compatible API types
// ============================================================================

#[derive(Serialize)]
struct OpenAITool<'a> {
    r#type: &'static str,
    function: &'a ToolSchema,
}

impl<'a> OpenAITool<'a> {
    fn from_schema(schema: &'a ToolSchema) -> Self {
        Self {
            r#type: "function",
            function: schema,
        }
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool<'a>>>,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    usage: OpenAIUsage,
}

#[derive(Deserialize)]
struct Choice {
    message: OpenAIAssistantMessage,
}

#[derive(Deserialize)]
struct OpenAIAssistantMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAIToolCall>>,
}

#[derive(Deserialize)]
struct OpenAIToolCall {
    id: String,
    #[serde(default)]
    r#type: Option<String>,
    function: OpenAIFunctionCall,
}

#[derive(Deserialize)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

// ============================================================================
// Anthropic Messages API types
// ============================================================================

#[derive(Serialize)]
struct CacheControl {
    r#type: &'static str,
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<AnthropicContentBlock>>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool<'a>>>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Serialize)]
struct AnthropicTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Value,
}

impl<'a> AnthropicTool<'a> {
    fn from_schema(schema: &'a ToolSchema) -> Self {
        Self {
            name: &schema.name,
            description: &schema.description,
            input_schema: &schema.parameters,
        }
    }
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicResponseContent>,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicResponseContent {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

/// Unified usage statistics
#[derive(Debug, Clone)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// Anthropic: tokens used to create cache
    pub cache_creation_tokens: u32,
    /// Anthropic: tokens read from cache
    pub cache_read_tokens: u32,
}

impl Usage {
    fn from_openai(usage: OpenAIUsage) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: if usage.total_tokens > 0 {
                usage.total_tokens
            } else {
                usage.prompt_tokens + usage.completion_tokens
            },
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        }
    }

    fn from_anthropic(usage: AnthropicUsage) -> Self {
        Self {
            prompt_tokens: usage.input_tokens,
            completion_tokens: usage.output_tokens,
            total_tokens: usage.input_tokens + usage.output_tokens,
            cache_creation_tokens: usage.cache_creation_input_tokens,
            cache_read_tokens: usage.cache_read_input_tokens,
        }
    }
}

/// Parsed model response
#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub usage: Usage,
}

impl ModelResponse {
    fn from_openai_completion(resp: ChatCompletionResponse) -> Result<Self, RiverError> {
        let choice =
            resp.choices.into_iter().next().ok_or_else(|| {
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
            usage: Usage::from_openai(resp.usage),
        })
    }

    fn from_anthropic_response(resp: AnthropicResponse) -> Result<Self, RiverError> {
        let mut content: Option<String> = None;
        let mut tool_calls = Vec::new();

        for block in resp.content {
            match block {
                AnthropicResponseContent::Text { text } => {
                    content = Some(text);
                }
                AnthropicResponseContent::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCallRequest {
                        id,
                        r#type: "function".to_string(),
                        function: FunctionCall {
                            name,
                            arguments: serde_json::to_string(&input).unwrap_or_default(),
                        },
                    });
                }
            }
        }

        Ok(Self {
            content,
            tool_calls,
            usage: Usage::from_anthropic(resp.usage),
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

    fn test_usage() -> Usage {
        Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        }
    }

    #[test]
    fn test_model_response_has_tool_calls() {
        let resp = ModelResponse {
            content: Some("Hello".to_string()),
            tool_calls: vec![],
            usage: test_usage(),
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
            usage: test_usage(),
        };
        assert!(resp_with_tools.has_tool_calls());
    }

    #[test]
    fn test_model_client_creation_openai() {
        let client = ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        )
        .expect("test client creation should succeed");
        assert_eq!(client.url(), "http://localhost:8080");
        assert_eq!(client.model(), "test-model");
        assert_eq!(client.provider(), Provider::OpenAI);
    }

    #[test]
    fn test_model_client_creation_anthropic() {
        let client = ModelClient::new(
            "https://api.anthropic.com".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            Duration::from_secs(30),
        )
        .expect("test client creation should succeed");
        assert_eq!(client.url(), "https://api.anthropic.com");
        assert_eq!(client.provider(), Provider::Anthropic);
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
        assert!(!json.contains("\"tools\""));
    }

    #[test]
    fn test_anthropic_content_block_serialization() {
        let block = AnthropicContentBlock::Text {
            text: "Hello".to_string(),
            cache_control: Some(CacheControl {
                r#type: "ephemeral",
            }),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Hello\""));
        assert!(json.contains("\"cache_control\""));
        assert!(json.contains("\"type\":\"ephemeral\""));
    }

    #[test]
    fn test_anthropic_tool_use_serialization() {
        let block = AnthropicContentBlock::ToolUse {
            id: "tool_123".to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"path": "/tmp/test"}),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"tool_use\""));
        assert!(json.contains("\"id\":\"tool_123\""));
        assert!(json.contains("\"name\":\"read\""));
    }

    #[test]
    fn test_anthropic_response_parsing() {
        let json = r#"{
            "content": [
                {"type": "text", "text": "Hello!"},
                {"type": "tool_use", "id": "tool_1", "name": "read", "input": {"path": "/tmp"}}
            ],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 80,
                "cache_read_input_tokens": 20
            }
        }"#;

        let resp: AnthropicResponse = serde_json::from_str(json).unwrap();
        let model_resp = ModelResponse::from_anthropic_response(resp).unwrap();

        assert_eq!(model_resp.content, Some("Hello!".to_string()));
        assert_eq!(model_resp.tool_calls.len(), 1);
        assert_eq!(model_resp.tool_calls[0].function.name, "read");
        assert_eq!(model_resp.usage.prompt_tokens, 100);
        assert_eq!(model_resp.usage.completion_tokens, 50);
        assert_eq!(model_resp.usage.cache_creation_tokens, 80);
        assert_eq!(model_resp.usage.cache_read_tokens, 20);
    }

    #[test]
    fn test_usage_from_anthropic() {
        let anthropic_usage = AnthropicUsage {
            input_tokens: 1000,
            output_tokens: 200,
            cache_creation_input_tokens: 500,
            cache_read_input_tokens: 300,
        };
        let usage = Usage::from_anthropic(anthropic_usage);

        assert_eq!(usage.prompt_tokens, 1000);
        assert_eq!(usage.completion_tokens, 200);
        assert_eq!(usage.total_tokens, 1200);
        assert_eq!(usage.cache_creation_tokens, 500);
        assert_eq!(usage.cache_read_tokens, 300);
    }
}

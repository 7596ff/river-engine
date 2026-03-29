//! Claude (Anthropic) LLM provider.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::LlmProvider;
use crate::types::{ContentBlock, LlmResponse, Message, MessageContent, Role, ToolDef, Usage};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Claude provider using the Anthropic API.
pub struct ClaudeProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: Option<String>,
}

impl ClaudeProvider {
    /// Create a new Claude provider.
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
            base_url,
        }
    }

    fn api_url(&self) -> String {
        self.base_url
            .clone()
            .unwrap_or_else(|| ANTHROPIC_API_URL.to_string())
    }
}

#[async_trait]
impl LlmProvider for ClaudeProvider {
    async fn complete(&self, messages: &[Message], tools: &[ToolDef]) -> Result<LlmResponse> {
        // Separate system message from conversation
        let (system, conversation): (Vec<_>, Vec<_>) = messages
            .iter()
            .partition(|m| matches!(m.role, Role::System));

        let system_text = system
            .iter()
            .filter_map(|m| match &m.content {
                MessageContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        // Convert messages to API format
        let mut api_messages: Vec<ApiMessage> = conversation
            .iter()
            .map(|m| ApiMessage::from_message(m))
            .collect();

        // Ensure first message is from user (API requirement)
        if api_messages.first().map(|m| m.role.as_str()) != Some("user") {
            return Err(anyhow!("First message must be from user"));
        }

        // Ensure messages alternate properly (merge consecutive same-role messages)
        api_messages = merge_consecutive_messages(api_messages);

        // Build request
        let mut request = ApiRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            system: if system_text.is_empty() {
                None
            } else {
                Some(system_text)
            },
            messages: api_messages,
            tools: None,
        };

        // Add tools if provided
        if !tools.is_empty() {
            request.tools = Some(
                tools
                    .iter()
                    .map(|t| ApiTool {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        input_schema: t.input_schema.clone(),
                    })
                    .collect(),
            );
        }

        tracing::debug!(model = %self.model, messages = request.messages.len(), "Calling Claude API");
        tracing::debug!("Request: {}", serde_json::to_string_pretty(&request).unwrap_or_default());

        let response = self
            .client
            .post(&self.api_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Claude API")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Claude API error ({}): {}",
                status,
                error_text
            ));
        }

        let api_response: ApiResponse = response
            .json()
            .await
            .context("Failed to parse Claude API response")?;

        tracing::debug!(
            stop_reason = ?api_response.stop_reason,
            input_tokens = api_response.usage.input_tokens,
            output_tokens = api_response.usage.output_tokens,
            "Claude API response"
        );

        // Convert to our types
        let content = api_response
            .content
            .into_iter()
            .filter_map(|block| match block {
                ApiContentBlock::Text { text } => Some(ContentBlock::Text { text }),
                ApiContentBlock::ToolUse { id, name, input } => {
                    Some(ContentBlock::ToolUse { id, name, input })
                }
                // ToolResult shouldn't appear in responses, only in requests
                ApiContentBlock::ToolResult { .. } => None,
            })
            .collect();

        Ok(LlmResponse {
            content,
            stop_reason: api_response.stop_reason,
            usage: Some(Usage {
                input_tokens: api_response.usage.input_tokens,
                output_tokens: api_response.usage.output_tokens,
            }),
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

/// Merge consecutive messages with the same role.
/// Claude API requires alternating user/assistant messages.
fn merge_consecutive_messages(messages: Vec<ApiMessage>) -> Vec<ApiMessage> {
    let mut result: Vec<ApiMessage> = Vec::new();

    for msg in messages {
        if let Some(last) = result.last_mut() {
            if last.role == msg.role {
                // Merge content
                match (&mut last.content, msg.content) {
                    (ApiMessageContent::Text(ref mut t1), ApiMessageContent::Text(t2)) => {
                        t1.push_str("\n\n");
                        t1.push_str(&t2);
                    }
                    (ApiMessageContent::Blocks(ref mut b1), ApiMessageContent::Blocks(b2)) => {
                        b1.extend(b2);
                    }
                    (ApiMessageContent::Text(t), ApiMessageContent::Blocks(b)) => {
                        let mut blocks = vec![ApiContentBlock::Text { text: t.clone() }];
                        blocks.extend(b);
                        last.content = ApiMessageContent::Blocks(blocks);
                    }
                    (ApiMessageContent::Blocks(ref mut b), ApiMessageContent::Text(t)) => {
                        b.push(ApiContentBlock::Text { text: t });
                    }
                }
                continue;
            }
        }
        result.push(msg);
    }

    result
}

// API types for serialization

#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: ApiMessageContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ApiMessageContent {
    Text(String),
    Blocks(Vec<ApiContentBlock>),
}

impl ApiMessage {
    fn from_message(msg: &Message) -> Self {
        let role = match msg.role {
            Role::System => "user", // System handled separately
            Role::User => "user",
            Role::Assistant => "assistant",
        };

        let content = match &msg.content {
            MessageContent::Text(t) => ApiMessageContent::Text(t.clone()),
            MessageContent::Blocks(blocks) => {
                let api_blocks: Vec<ApiContentBlock> = blocks
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => ApiContentBlock::Text { text: text.clone() },
                        ContentBlock::ToolUse { id, name, input } => ApiContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        },
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => ApiContentBlock::ToolResult {
                            tool_use_id: tool_use_id.clone(),
                            content: content.clone(),
                            is_error: *is_error,
                        },
                    })
                    .collect();
                ApiMessageContent::Blocks(api_blocks)
            }
        };

        Self {
            role: role.to_string(),
            content,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiContentBlock {
    Text {
        text: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

#[derive(Debug, Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ApiContentBlock>,
    stop_reason: Option<String>,
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
}

//! LLM client for OpenAI-compatible endpoints.

use crate::config::ModelConfig;
use river_context::OpenAIMessage;
use serde::{Deserialize, Serialize};

/// LLM client.
pub struct LlmClient {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: String,
}

/// Chat completion request.
#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [OpenAIMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolDef]>,
}

/// Tool definition for the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Chat completion response.
#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallResponse>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallResponse {
    id: String,
    #[serde(rename = "type")]
    #[allow(dead_code)] // Always "function" in OpenAI API, but part of the schema
    call_type: String,
    function: FunctionCallResponse,
}

#[derive(Debug, Deserialize)]
struct FunctionCallResponse {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    total_tokens: usize,
}

/// LLM response.
#[derive(Debug)]
pub struct LlmResponse {
    pub content: LlmContent,
    pub usage: LlmUsage,
}

/// Response content.
#[derive(Debug)]
pub enum LlmContent {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// Tool call from LLM.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Token usage.
#[derive(Debug)]
pub struct LlmUsage {
    pub total_tokens: usize,
}

/// LLM error.
#[derive(Debug)]
pub enum LlmError {
    Request(reqwest::Error),
    NoChoices,
    ApiError { status: u16, message: String },
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Request(e) => write!(f, "Request error: {}", e),
            LlmError::NoChoices => write!(f, "No choices in response"),
            LlmError::ApiError { status, message } => {
                write!(f, "API error ({}): {}", status, message)
            }
        }
    }
}

impl std::error::Error for LlmError {}

impl LlmClient {
    /// Create a new LLM client.
    pub fn new(config: &ModelConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: config.endpoint.clone(),
            model: config.name.clone(),
            api_key: config.api_key.clone(),
        }
    }

    /// Update model configuration.
    #[allow(dead_code)] // For future dynamic model switching
    pub fn update_config(&mut self, config: &ModelConfig) {
        self.endpoint = config.endpoint.clone();
        self.model = config.name.clone();
        self.api_key = config.api_key.clone();
    }

    /// Send chat completion request.
    pub async fn chat(
        &self,
        messages: &[OpenAIMessage],
        tools: Option<&[ToolDef]>,
    ) -> Result<LlmResponse, LlmError> {
        let url = format!("{}/chat/completions", self.endpoint.trim_end_matches('/'));

        let request = ChatRequest {
            model: &self.model,
            messages,
            tools,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(LlmError::Request)?;

        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::ApiError {
                status: status.as_u16(),
                message,
            });
        }

        let body: ChatResponse = response.json().await.map_err(LlmError::Request)?;

        let choice = body.choices.first().ok_or(LlmError::NoChoices)?;

        let content = if let Some(tool_calls) = &choice.message.tool_calls {
            LlmContent::ToolCalls(
                tool_calls
                    .iter()
                    .map(|tc| ToolCall {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments: tc.function.arguments.clone(),
                    })
                    .collect(),
            )
        } else {
            LlmContent::Text(choice.message.content.clone().unwrap_or_default())
        };

        Ok(LlmResponse {
            content,
            usage: LlmUsage {
                total_tokens: body.usage.total_tokens,
            },
        })
    }
}

/// Get tool definitions for all worker tools.
pub fn get_tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "read".into(),
                description: "Read file contents from workspace".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative path from workspace root" },
                        "start_line": { "type": "integer", "description": "First line to read (1-indexed)" },
                        "end_line": { "type": "integer", "description": "Last line to read (inclusive)" }
                    },
                    "required": ["path"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "write".into(),
                description: "Write file to workspace".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative path from workspace root" },
                        "content": { "type": "string", "description": "Content to write" },
                        "mode": { "type": "string", "enum": ["overwrite", "append", "insert"], "description": "Write mode (default: overwrite)" },
                        "at_line": { "type": "integer", "description": "Line number for insert mode" }
                    },
                    "required": ["path", "content"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "delete".into(),
                description: "Delete file from workspace".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative path from workspace root" }
                    },
                    "required": ["path"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "bash".into(),
                description: "Execute shell command".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to execute" },
                        "timeout_seconds": { "type": "integer", "description": "Timeout in seconds (default: 120, max: 600)" },
                        "working_directory": { "type": "string", "description": "Working directory (default: workspace root)" }
                    },
                    "required": ["command"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "speak".into(),
                description: "Send message to a channel".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "Message content" },
                        "adapter": { "type": "string", "description": "Adapter name (defaults to current channel)" },
                        "channel": { "type": "string", "description": "Channel ID (defaults to current channel)" },
                        "reply_to": { "type": "string", "description": "Message ID to reply to" }
                    },
                    "required": ["content"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "switch_channel".into(),
                description: "Change current channel".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "adapter": { "type": "string", "description": "Adapter name" },
                        "channel": { "type": "string", "description": "Channel ID" }
                    },
                    "required": ["adapter", "channel"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "sleep".into(),
                description: "Pause the worker loop".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "minutes": { "type": "integer", "description": "Minutes to sleep (omit for indefinite)" }
                    }
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "watch".into(),
                description: "Manage watched channels for wake notifications".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "add": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "adapter": { "type": "string" },
                                    "id": { "type": "string" },
                                    "name": { "type": "string" }
                                },
                                "required": ["adapter", "id"]
                            },
                            "description": "Channels to add to watch list"
                        },
                        "remove": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "adapter": { "type": "string" },
                                    "id": { "type": "string" }
                                },
                                "required": ["adapter", "id"]
                            },
                            "description": "Channels to remove from watch list"
                        }
                    }
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "summary".into(),
                description: "Exit the worker loop with a summary".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "summary": { "type": "string", "description": "Summary of work done, state, next steps" }
                    },
                    "required": ["summary"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "create_flash".into(),
                description: "Send message to another worker (peer-to-peer)".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target_dyad": { "type": "string", "description": "Target dyad name" },
                        "target_side": { "type": "string", "enum": ["left", "right"], "description": "Target side" },
                        "content": { "type": "string", "description": "Message content" },
                        "ttl_minutes": { "type": "integer", "description": "Time-to-live in minutes (default: 60)" }
                    },
                    "required": ["target_dyad", "target_side", "content"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "request_model".into(),
                description: "Switch to a different LLM model".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "model": { "type": "string", "description": "Model name (key in orchestrator config)" }
                    },
                    "required": ["model"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "switch_roles".into(),
                description: "Switch roles with partner worker".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "search_embeddings".into(),
                description: "Search embeddings, returns first result and cursor".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query text" }
                    },
                    "required": ["query"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "next_embedding".into(),
                description: "Continue embedding search with cursor".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "cursor": { "type": "string", "description": "Cursor from previous search" }
                    },
                    "required": ["cursor"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "create_move".into(),
                description: "Create a move (summarizes a range of messages)".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "channel": {
                            "type": "object",
                            "properties": {
                                "adapter": { "type": "string" },
                                "id": { "type": "string" },
                                "name": { "type": "string" }
                            },
                            "required": ["adapter", "id"]
                        },
                        "content": { "type": "string", "description": "The move summary" },
                        "start_message_id": { "type": "string", "description": "First message in range" },
                        "end_message_id": { "type": "string", "description": "Last message in range" }
                    },
                    "required": ["channel", "content", "start_message_id", "end_message_id"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "create_moment".into(),
                description: "Create a moment (summarizes a range of moves)".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "channel": {
                            "type": "object",
                            "properties": {
                                "adapter": { "type": "string" },
                                "id": { "type": "string" },
                                "name": { "type": "string" }
                            },
                            "required": ["adapter", "id"]
                        },
                        "content": { "type": "string", "description": "The moment summary" },
                        "start_move_id": { "type": "string", "description": "First move in range" },
                        "end_move_id": { "type": "string", "description": "Last move in range" }
                    },
                    "required": ["channel", "content", "start_move_id", "end_move_id"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "adapter".into(),
                description: "Execute any adapter operation".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "adapter": { "type": "string", "description": "Adapter name" },
                        "request": { "type": "object", "description": "Full OutboundRequest object" }
                    },
                    "required": ["adapter", "request"]
                }),
            },
        },
    ]
}

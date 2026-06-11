//! The model client (wall chs. 01, 07, 09): two protocols — the
//! Anthropic Messages API and the OpenAI-compatible chat completions
//! API — chosen by the model's `provider` field, both speaking tools.
//! API keys are read from the environment at call time via
//! `api_key_env` indirection; the key never lives in config or in
//! this struct.
//!
//! Pure: request-body construction and response parsing. Effectful:
//! the HTTP send loop with retries and timeouts.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use river_core::config::{ModelConfig, Provider};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 8192;
const MAX_ATTEMPTS: u32 = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
}

/// One requested tool invocation. `arguments` is JSON text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Assistant messages only.
    pub tool_calls: Vec<ToolCall>,
    /// Tool messages only.
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// What the registry advertises for one tool (wall ch. 07).
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON schema for the parameters object.
    pub parameters: Value,
}

#[derive(Debug, PartialEq)]
pub struct ChatResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    /// Reported prompt token count, when the API provides usage data.
    /// Feeds the context estimator's calibration (wall ch. 03).
    pub prompt_tokens: Option<u64>,
}

/// The model-call seam: the turn loop talks to this, tests fake it.
pub trait Chat {
    fn chat(
        &self,
        system: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> impl Future<Output = anyhow::Result<ChatResponse>> + Send;
}

impl Chat for ModelClient {
    async fn chat(
        &self,
        system: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> anyhow::Result<ChatResponse> {
        ModelClient::chat(self, system, messages, tools).await
    }
}

pub struct ModelClient {
    http: reqwest::Client,
    provider: Provider,
    endpoint: String,
    model_name: String,
    api_key_env: Option<String>,
}

impl ModelClient {
    pub fn new(config: &ModelConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(Duration::from_secs(config.request_timeout_seconds))
            .build()?;
        Ok(Self {
            http,
            provider: config.provider,
            endpoint: config.endpoint.trim_end_matches('/').to_string(),
            model_name: config.name.clone(),
            api_key_env: config.api_key_env.clone(),
        })
    }

    pub async fn chat(
        &self,
        system: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> anyhow::Result<ChatResponse> {
        let (url, body) = match self.provider {
            Provider::Anthropic => (
                format!("{}/messages", self.endpoint),
                build_anthropic_body(&self.model_name, system, messages, tools),
            ),
            Provider::Openai => (
                format!("{}/chat/completions", self.endpoint),
                build_openai_body(&self.model_name, system, messages, tools),
            ),
        };

        let api_key = match &self.api_key_env {
            Some(var) => Some(std::env::var(var).map_err(|_| {
                anyhow::anyhow!("api_key_env {var} is not set in the environment")
            })?),
            None => None,
        };

        let mut last_error = None;
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                let backoff = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                tokio::time::sleep(backoff).await;
            }

            let mut request = self.http.post(&url).json(&body);
            request = match (self.provider, &api_key) {
                (Provider::Anthropic, Some(key)) => request
                    .header("x-api-key", key)
                    .header("anthropic-version", ANTHROPIC_VERSION),
                (Provider::Anthropic, None) => {
                    request.header("anthropic-version", ANTHROPIC_VERSION)
                }
                (Provider::Openai, Some(key)) => {
                    request.header("authorization", format!("Bearer {key}"))
                }
                (Provider::Openai, None) => request,
            };

            match request.send().await {
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "model request failed to send");
                    last_error = Some(anyhow::anyhow!(e));
                }
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    if status.is_success() {
                        let parsed = match self.provider {
                            Provider::Anthropic => parse_anthropic_response(&text),
                            Provider::Openai => parse_openai_response(&text),
                        };
                        return parsed.map_err(|e| anyhow::anyhow!("model response: {e}"));
                    }
                    tracing::warn!(attempt, %status, "model request failed");
                    last_error = Some(anyhow::anyhow!(
                        "model request failed with {status}: {}",
                        text.chars().take(500).collect::<String>()
                    ));
                    if !is_retryable(status.as_u16()) {
                        break;
                    }
                }
            }
        }
        Err(last_error.expect("at least one attempt ran"))
    }
}

/// Retry on rate limits, server errors, and overload; never on client
/// errors — a malformed request does not improve with repetition.
pub fn is_retryable(status: u16) -> bool {
    status == 429 || status >= 500
}

fn arguments_value(arguments: &str) -> Value {
    serde_json::from_str(arguments).unwrap_or_else(|_| json!({}))
}

pub fn build_anthropic_body(
    model: &str,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSchema],
) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for msg in messages {
        match msg.role {
            Role::User => out.push(json!({ "role": "user", "content": msg.content })),
            Role::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                if !msg.content.is_empty() {
                    blocks.push(json!({ "type": "text", "text": msg.content }));
                }
                for call in &msg.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": arguments_value(&call.arguments),
                    }));
                }
                out.push(json!({ "role": "assistant", "content": blocks }));
            }
            Role::Tool => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": msg.tool_call_id,
                    "content": msg.content,
                });
                // Consecutive tool results join the same user message —
                // the protocol wants them in one.
                let appended = out
                    .last_mut()
                    .and_then(|last| {
                        (last["role"] == "user" && last["content"].is_array()).then(|| {
                            last["content"]
                                .as_array_mut()
                                .expect("checked array")
                                .push(block.clone())
                        })
                    })
                    .is_some();
                if !appended {
                    out.push(json!({ "role": "user", "content": [block] }));
                }
            }
        }
    }

    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": out,
    });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
    }
    body
}

pub fn build_openai_body(
    model: &str,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSchema],
) -> Value {
    let mut out = vec![json!({ "role": "system", "content": system })];
    for msg in messages {
        match msg.role {
            Role::User => out.push(json!({ "role": "user", "content": msg.content })),
            Role::Assistant => {
                let mut m = json!({ "role": "assistant" });
                // content may be null only when tool_calls carry the
                // message; an empty assistant line must still send "".
                m["content"] = if msg.content.is_empty() && !msg.tool_calls.is_empty() {
                    Value::Null
                } else {
                    Value::String(msg.content.clone())
                };
                if !msg.tool_calls.is_empty() {
                    m["tool_calls"] = msg
                        .tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": { "name": call.name, "arguments": call.arguments },
                            })
                        })
                        .collect();
                }
                out.push(m);
            }
            Role::Tool => out.push(json!({
                "role": "tool",
                "tool_call_id": msg.tool_call_id,
                "content": msg.content,
            })),
        }
    }

    let mut body = json!({
        "model": model,
        "messages": out,
    });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    },
                })
            })
            .collect();
    }
    body
}

pub fn parse_anthropic_response(text: &str) -> Result<ChatResponse, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let blocks = value["content"].as_array().ok_or("missing content array")?;
    let content = blocks
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    let tool_calls = blocks
        .iter()
        .filter(|block| block["type"] == "tool_use")
        .map(|block| ToolCall {
            id: block["id"].as_str().unwrap_or_default().to_string(),
            name: block["name"].as_str().unwrap_or_default().to_string(),
            arguments: block["input"].to_string(),
        })
        .collect();
    let prompt_tokens = value["usage"]["input_tokens"].as_u64();
    Ok(ChatResponse {
        content,
        tool_calls,
        prompt_tokens,
    })
}

pub fn parse_openai_response(text: &str) -> Result<ChatResponse, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let message = &value["choices"][0]["message"];
    if message.is_null() {
        return Err("missing choices[0].message".to_string());
    }
    let content = message["content"].as_str().unwrap_or_default().to_string();
    let tool_calls = message["tool_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .map(|call| ToolCall {
                    id: call["id"].as_str().unwrap_or_default().to_string(),
                    name: call["function"]["name"].as_str().unwrap_or_default().to_string(),
                    arguments: call["function"]["arguments"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let prompt_tokens = value["usage"]["prompt_tokens"].as_u64();
    Ok(ChatResponse {
        content,
        tool_calls,
        prompt_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Vec<ToolSchema> {
        vec![ToolSchema {
            name: "read".into(),
            description: "read a file".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}}}),
        }]
    }

    fn tool_turn() -> Vec<ChatMessage> {
        vec![
            ChatMessage::user("read notes.md"),
            ChatMessage::assistant(
                "reading it now",
                vec![ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: r#"{"path":"notes.md"}"#.into(),
                }],
            ),
            ChatMessage::tool_result("call_1", "the notes say hello"),
            ChatMessage::tool_result("call_1b", "second result"),
        ]
    }

    #[test]
    fn anthropic_body_builds_tool_blocks() {
        let body = build_anthropic_body("claude", "sys", &tool_turn(), &schema());
        let messages = body["messages"].as_array().unwrap();
        // user, assistant, ONE user message holding both tool results
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["content"][0]["type"], "text");
        assert_eq!(messages[1]["content"][1]["type"], "tool_use");
        assert_eq!(messages[1]["content"][1]["input"]["path"], "notes.md");
        let results = messages[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2, "consecutive tool results coalesce");
        assert_eq!(results[0]["tool_use_id"], "call_1");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn openai_body_builds_tool_calls() {
        let body = build_openai_body("gemma4", "sys", &tool_turn(), &schema());
        let messages = body["messages"].as_array().unwrap();
        // system, user, assistant, tool, tool
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "read");
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
        assert_eq!(body["tools"][0]["function"]["name"], "read");
    }

    #[test]
    fn empty_tools_omits_the_field() {
        let body = build_openai_body("m", "s", &[ChatMessage::user("hi")], &[]);
        assert!(body.get("tools").is_none());
        let body = build_anthropic_body("m", "s", &[ChatMessage::user("hi")], &[]);
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn parses_anthropic_tool_use() {
        let text = r#"{
            "content": [
                {"type": "text", "text": "let me check"},
                {"type": "tool_use", "id": "tu_1", "name": "read",
                 "input": {"path": "notes.md"}}
            ],
            "usage": {"input_tokens": 42}
        }"#;
        let parsed = parse_anthropic_response(text).unwrap();
        assert_eq!(parsed.content, "let me check");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "read");
        assert!(parsed.tool_calls[0].arguments.contains("notes.md"));
        assert_eq!(parsed.prompt_tokens, Some(42));
    }

    #[test]
    fn parses_openai_tool_calls() {
        let text = r#"{
            "choices": [{"message": {"content": null, "tool_calls": [
                {"id": "call_1", "type": "function",
                 "function": {"name": "read", "arguments": "{\"path\":\"notes.md\"}"}}
            ]}}],
            "usage": {"prompt_tokens": 42}
        }"#;
        let parsed = parse_openai_response(text).unwrap();
        assert_eq!(parsed.content, "");
        assert_eq!(parsed.tool_calls[0].id, "call_1");
        assert_eq!(parsed.tool_calls[0].arguments, r#"{"path":"notes.md"}"#);
    }

    #[test]
    fn parses_plain_text_responses() {
        let anthropic = r#"{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":5}}"#;
        let parsed = parse_anthropic_response(anthropic).unwrap();
        assert_eq!(parsed.content, "hi");
        assert!(parsed.tool_calls.is_empty());

        let openai = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        let parsed = parse_openai_response(openai).unwrap();
        assert_eq!(parsed.content, "hi");
        assert!(parsed.tool_calls.is_empty());
        assert_eq!(parsed.prompt_tokens, None);
    }

    #[test]
    fn malformed_responses_are_errors() {
        assert!(parse_anthropic_response("{}").is_err());
        assert!(parse_openai_response("{}").is_err());
        assert!(parse_openai_response("not json").is_err());
    }

    #[test]
    fn retryability() {
        assert!(is_retryable(429));
        assert!(is_retryable(500));
        assert!(is_retryable(529));
        assert!(!is_retryable(400));
        assert!(!is_retryable(401));
        assert!(!is_retryable(404));
    }
}

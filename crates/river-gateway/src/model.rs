//! The model client (wall chs. 01, 09): two protocols — the Anthropic
//! Messages API and the OpenAI-compatible chat completions API —
//! chosen by the model's `provider` field. API keys are read from the
//! environment at call time via `api_key_env` indirection; the key
//! never lives in config or in this struct.
//!
//! Pure: request-body construction and response parsing. Effectful:
//! the HTTP send loop with retries and timeouts.

use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::config::{ModelConfig, Provider};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 8192;
const MAX_ATTEMPTS: u32 = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, PartialEq)]
pub struct ChatResponse {
    pub content: String,
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
    ) -> impl Future<Output = anyhow::Result<ChatResponse>> + Send;
}

impl Chat for ModelClient {
    async fn chat(&self, system: &str, messages: &[ChatMessage]) -> anyhow::Result<ChatResponse> {
        ModelClient::chat(self, system, messages).await
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
            .timeout(REQUEST_TIMEOUT)
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
    ) -> anyhow::Result<ChatResponse> {
        let (url, body) = match self.provider {
            Provider::Anthropic => (
                format!("{}/messages", self.endpoint),
                build_anthropic_body(&self.model_name, system, messages),
            ),
            Provider::Openai => (
                format!("{}/chat/completions", self.endpoint),
                build_openai_body(&self.model_name, system, messages),
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

pub fn build_anthropic_body(model: &str, system: &str, messages: &[ChatMessage]) -> Value {
    json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": messages,
    })
}

pub fn build_openai_body(model: &str, system: &str, messages: &[ChatMessage]) -> Value {
    let mut all = vec![json!({ "role": "system", "content": system })];
    all.extend(messages.iter().map(|m| serde_json::to_value(m).unwrap()));
    json!({
        "model": model,
        "messages": all,
    })
}

pub fn parse_anthropic_response(text: &str) -> Result<ChatResponse, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let content = value["content"]
        .as_array()
        .ok_or("missing content array")?
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    let prompt_tokens = value["usage"]["input_tokens"].as_u64();
    Ok(ChatResponse {
        content,
        prompt_tokens,
    })
}

pub fn parse_openai_response(text: &str) -> Result<ChatResponse, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let content = value["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("missing choices[0].message.content")?
        .to_string();
    let prompt_tokens = value["usage"]["prompt_tokens"].as_u64();
    Ok(ChatResponse {
        content,
        prompt_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: Role::User,
                content: "hello".into(),
            },
            ChatMessage {
                role: Role::Assistant,
                content: "hi".into(),
            },
        ]
    }

    #[test]
    fn anthropic_body_shape() {
        let body = build_anthropic_body("claude-sonnet-4", "be honest", &messages());
        assert_eq!(body["model"], "claude-sonnet-4");
        assert_eq!(body["system"], "be honest");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert!(body["max_tokens"].as_u64().unwrap() > 0);
    }

    #[test]
    fn openai_body_puts_system_first() {
        let body = build_openai_body("qwen3:8b", "be honest", &messages());
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "be honest");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn parses_anthropic_response() {
        let text = r#"{
            "content": [
                {"type": "text", "text": "good "},
                {"type": "text", "text": "morning"}
            ],
            "usage": {"input_tokens": 42, "output_tokens": 7}
        }"#;
        let parsed = parse_anthropic_response(text).unwrap();
        assert_eq!(parsed.content, "good morning");
        assert_eq!(parsed.prompt_tokens, Some(42));
    }

    #[test]
    fn parses_openai_response() {
        let text = r#"{
            "choices": [{"message": {"role": "assistant", "content": "good morning"}}],
            "usage": {"prompt_tokens": 42, "completion_tokens": 7}
        }"#;
        let parsed = parse_openai_response(text).unwrap();
        assert_eq!(parsed.content, "good morning");
        assert_eq!(parsed.prompt_tokens, Some(42));
    }

    #[test]
    fn missing_usage_is_tolerated() {
        let text = r#"{"choices": [{"message": {"content": "ok"}}]}"#;
        let parsed = parse_openai_response(text).unwrap();
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

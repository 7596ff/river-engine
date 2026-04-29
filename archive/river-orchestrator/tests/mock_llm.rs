//! Mock LLM server for integration tests with role-aware responses.

use axum::{Router, Json, routing::post, extract::State};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// OpenAI-compatible message for mock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<MockToolCall>>,
}

/// Tool call structure matching OpenAI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub r#type: String,  // Always "function"
    pub function: MockFunctionCall,
}

/// Function call details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockFunctionCall {
    pub name: String,
    pub arguments: String,  // JSON string
}

/// Chat completion request from worker.
#[derive(Debug, Deserialize)]
pub struct MockChatRequest {
    pub model: String,
    pub messages: Vec<MockChatMessage>,
}

/// Chat completion response to worker.
#[derive(Debug, Serialize)]
pub struct MockChatResponse {
    pub id: String,
    pub object: String,  // "chat.completion"
    pub created: u64,
    pub model: String,
    pub choices: Vec<MockChoice>,
    pub usage: MockUsage,
}

/// Response choice.
#[derive(Debug, Serialize)]
pub struct MockChoice {
    pub index: u32,
    pub message: MockChatMessage,
    pub finish_reason: String,  // "tool_calls" or "stop"
}

/// Token usage stats.
#[derive(Debug, Serialize)]
pub struct MockUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Mock server state tracking call count.
#[derive(Clone)]
struct MockLlmState {
    call_count: Arc<RwLock<u32>>,
}

/// Handle chat completion requests with role-aware responses.
async fn handle_chat_completion(
    State(state): State<MockLlmState>,
    Json(request): Json<MockChatRequest>,
) -> Json<MockChatResponse> {
    let mut count = state.call_count.write().await;
    *count += 1;
    let call_number = *count;

    // Determine role from messages (last system message with "baton" keyword)
    let is_actor = request.messages.iter().rev()
        .find(|m| m.role == "system" && m.content.as_ref().map_or(false, |c| c.contains("Actor")))
        .is_some();

    // Role-aware response templates (per D-16 from CONTEXT.md)
    let (content, tool_calls) = if is_actor {
        // Actor: action-oriented, includes tool calls
        let content = format!("I'll read the latest context to understand the current state. Call {}", call_number);
        let tool_calls = vec![
            MockToolCall {
                id: format!("call-{}", call_number),
                r#type: "function".to_string(),
                function: MockFunctionCall {
                    name: "read_history".to_string(),
                    arguments: r#"{"adapter":"tui","channel":"test-channel"}"#.to_string(),
                },
            },
        ];
        (Some(content), Some(tool_calls))
    } else {
        // Spectator: observational, reflective
        let content = format!("I notice the actor is working through the context. Call {}", call_number);
        let tool_calls = vec![
            MockToolCall {
                id: format!("call-{}", call_number),
                r#type: "function".to_string(),
                function: MockFunctionCall {
                    name: "read_history".to_string(),
                    arguments: r#"{"adapter":"tui","channel":"test-channel"}"#.to_string(),
                },
            },
        ];
        (Some(content), Some(tool_calls))
    };

    Json(MockChatResponse {
        id: format!("chatcmpl-{}", call_number),
        object: "chat.completion".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        model: "mock-gpt-4".to_string(),
        choices: vec![MockChoice {
            index: 0,
            message: MockChatMessage {
                role: "assistant".to_string(),
                content,
                tool_calls,
            },
            finish_reason: "tool_calls".to_string(),
        }],
        usage: MockUsage {
            prompt_tokens: 10,
            completion_tokens: 8,
            total_tokens: 18,
        },
    })
}

/// Mock LLM server handle.
pub struct MockLlmServer {
    pub endpoint: String,
    pub port: u16,
}

/// Start mock LLM server on specified port.
pub async fn start_mock_llm(port: u16) -> Result<MockLlmServer, Box<dyn std::error::Error>> {
    let state = MockLlmState {
        call_count: Arc::new(RwLock::new(0)),
    };

    let app = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completion))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    let actual_port = listener.local_addr()?.port();

    // Spawn server in background
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Ok(MockLlmServer {
        endpoint: format!("http://127.0.0.1:{}", actual_port),
        port: actual_port,
    })
}

# Gateway Continuous Tool Loop Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the continuous tool loop that runs the agent's WAKE → THINK → ACT → SETTLE → SLEEP cycle.

**Architecture:** Actor-based loop running as a tokio task, receiving events through an mpsc channel. Messages arriving mid-generation are queued and injected with tool results. The loop owns the agent's cognitive state and transitions through phases based on model responses.

**Tech Stack:** Rust, tokio (async runtime + channels), reqwest (HTTP client for model API), serde_json (serialization)

**Spec:** `docs/superpowers/specs/2026-03-17-gateway-loop-design.md`

---

## File Structure

```
crates/river-gateway/src/
├── loop/
│   ├── mod.rs          # AgentLoop struct, run(), phase methods
│   ├── state.rs        # LoopState, LoopEvent, WakeTrigger enums
│   ├── context.rs      # ContextBuilder, ChatMessage
│   ├── queue.rs        # MessageQueue (thread-safe)
│   └── model.rs        # ModelClient, request/response types
├── api/
│   └── routes.rs       # Update /incoming to send LoopEvent
├── state.rs            # Add loop_tx, message_queue to AppState
└── server.rs           # Spawn AgentLoop on startup
```

**File Responsibilities:**

| File | Responsibility |
|------|----------------|
| `loop/state.rs` | Pure data types for loop state machine |
| `loop/queue.rs` | Thread-safe message queue with Mutex |
| `loop/context.rs` | Build conversation context from workspace + messages |
| `loop/model.rs` | HTTP client for OpenAI-compatible model API |
| `loop/mod.rs` | AgentLoop actor that ties everything together |

---

## Task 1: Loop State Types

**Files:**
- Create: `crates/river-gateway/src/loop/state.rs`
- Modify: `crates/river-gateway/src/loop/mod.rs`

### Step 1.1: Write tests for state types

- [ ] Create `crates/river-gateway/src/loop/state.rs` with test module:

```rust
//! Loop state machine types

use crate::api::routes::IncomingMessage;
use serde::{Deserialize, Serialize};

/// Events that can wake or signal the loop
#[derive(Debug, Clone)]
pub enum LoopEvent {
    /// Message from communication adapter
    Message(IncomingMessage),
    /// Heartbeat timer fired
    Heartbeat,
    /// Graceful shutdown requested
    Shutdown,
}

/// What caused the agent to wake
#[derive(Debug, Clone)]
pub enum WakeTrigger {
    /// User or external message
    Message(IncomingMessage),
    /// Scheduled heartbeat
    Heartbeat,
}

/// The agent's current phase in the cycle
#[derive(Debug, Clone, Default)]
pub enum LoopState {
    /// Waiting for next event
    #[default]
    Sleeping,
    /// Woke up, assembling context
    Waking { trigger: WakeTrigger },
    /// Model is generating
    Thinking,
    /// Executing tool calls
    Acting,
    /// Cycle complete, committing state
    Settling,
}

impl LoopState {
    /// Check if loop is in a phase where messages should be queued
    pub fn should_queue_messages(&self) -> bool {
        matches!(self, LoopState::Thinking | LoopState::Acting)
    }

    /// Check if loop is sleeping
    pub fn is_sleeping(&self) -> bool {
        matches!(self, LoopState::Sleeping)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_queue_messages() {
        assert!(!LoopState::Sleeping.should_queue_messages());
        assert!(!LoopState::Settling.should_queue_messages());
        assert!(LoopState::Thinking.should_queue_messages());
        assert!(LoopState::Acting.should_queue_messages());
    }

    #[test]
    fn test_is_sleeping() {
        assert!(LoopState::Sleeping.is_sleeping());
        assert!(!LoopState::Thinking.is_sleeping());
        assert!(!LoopState::Acting.is_sleeping());
    }

    #[test]
    fn test_default_state_is_sleeping() {
        assert!(LoopState::default().is_sleeping());
    }
}
```

- [ ] **Step 1.2: Run tests to verify they pass**

Run: `cargo test -p river-gateway loop::state --lib`
Expected: 3 tests pass

- [ ] **Step 1.3: Update loop/mod.rs to export state module**

Replace contents of `crates/river-gateway/src/loop/mod.rs`:

```rust
//! Agent loop module

pub mod state;

pub use state::{LoopEvent, LoopState, WakeTrigger};
```

- [ ] **Step 1.4: Run all loop tests**

Run: `cargo test -p river-gateway loop --lib`
Expected: PASS

- [ ] **Step 1.5: Commit**

```bash
git add crates/river-gateway/src/loop/
git commit -m "feat(gateway): add loop state machine types"
```

---

## Task 2: Message Queue

**Files:**
- Create: `crates/river-gateway/src/loop/queue.rs`
- Modify: `crates/river-gateway/src/loop/mod.rs`

### Step 2.1: Write failing test for MessageQueue

- [ ] Create `crates/river-gateway/src/loop/queue.rs`:

```rust
//! Thread-safe message queue for mid-generation messages

use crate::api::routes::IncomingMessage;
use std::collections::VecDeque;
use std::sync::Mutex;

/// Thread-safe queue for messages arriving mid-generation
pub struct MessageQueue {
    inner: Mutex<VecDeque<IncomingMessage>>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Add a message to the queue
    pub fn push(&self, msg: IncomingMessage) {
        let mut queue = self.inner.lock().unwrap();
        queue.push_back(msg);
    }

    /// Drain all messages from the queue
    pub fn drain(&self) -> Vec<IncomingMessage> {
        let mut queue = self.inner.lock().unwrap();
        queue.drain(..).collect()
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        let queue = self.inner.lock().unwrap();
        queue.is_empty()
    }

    /// Get queue length
    pub fn len(&self) -> usize {
        let queue = self.inner.lock().unwrap();
        queue.len()
    }
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::routes::Author;

    fn test_message(content: &str) -> IncomingMessage {
        IncomingMessage {
            adapter: "test".to_string(),
            event_type: "message".to_string(),
            channel: "general".to_string(),
            author: Author {
                id: "user1".to_string(),
                name: "Test User".to_string(),
            },
            content: content.to_string(),
            message_id: None,
            metadata: None,
        }
    }

    #[test]
    fn test_new_queue_is_empty() {
        let queue = MessageQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_push_and_drain() {
        let queue = MessageQueue::new();

        queue.push(test_message("hello"));
        queue.push(test_message("world"));

        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 2);

        let messages = queue.drain();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].content, "world");

        assert!(queue.is_empty());
    }

    #[test]
    fn test_drain_empty_queue() {
        let queue = MessageQueue::new();
        let messages = queue.drain();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let queue = Arc::new(MessageQueue::new());
        let mut handles = vec![];

        // Spawn threads that push messages
        for i in 0..10 {
            let q = queue.clone();
            handles.push(thread::spawn(move || {
                q.push(test_message(&format!("msg{}", i)));
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(queue.len(), 10);
    }
}
```

- [ ] **Step 2.2: Run tests**

Run: `cargo test -p river-gateway loop::queue --lib`
Expected: 4 tests pass

- [ ] **Step 2.3: Update loop/mod.rs**

```rust
//! Agent loop module

pub mod state;
pub mod queue;

pub use state::{LoopEvent, LoopState, WakeTrigger};
pub use queue::MessageQueue;
```

- [ ] **Step 2.4: Run all loop tests**

Run: `cargo test -p river-gateway loop --lib`
Expected: PASS

- [ ] **Step 2.5: Commit**

```bash
git add crates/river-gateway/src/loop/
git commit -m "feat(gateway): add thread-safe message queue"
```

---

## Task 3: ChatMessage and Context Types

**Files:**
- Create: `crates/river-gateway/src/loop/context.rs`
- Modify: `crates/river-gateway/src/loop/mod.rs`

### Step 3.1: Create ChatMessage type

- [ ] Create `crates/river-gateway/src/loop/context.rs`:

```rust
//! Context assembly for model calls

use crate::api::routes::IncomingMessage;
use crate::tools::{ToolCallResponse, ToolSchema};
use crate::r#loop::state::WakeTrigger;
use river_core::ContextStatus;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A message in the chat format (OpenAI-compatible)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Tool call as returned by the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub r#type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: Option<String>, tool_calls: Option<Vec<ToolCallRequest>>) -> Self {
        Self {
            role: "assistant".to_string(),
            content,
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }
}

/// Builds conversation context for model calls
pub struct ContextBuilder {
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSchema>,
}

impl ContextBuilder {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            tools: Vec::new(),
        }
    }

    /// Clear all messages (for new cycle)
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Get messages for API call
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Get tools for API call
    pub fn tools(&self) -> &[ToolSchema] {
        &self.tools
    }

    /// Set available tools
    pub fn set_tools(&mut self, tools: Vec<ToolSchema>) {
        self.tools = tools;
    }

    /// Add a message
    pub fn add_message(&mut self, msg: ChatMessage) {
        self.messages.push(msg);
    }

    /// Assemble context for a new wake cycle
    pub async fn assemble(
        &mut self,
        workspace: &Path,
        trigger: WakeTrigger,
        queued_messages: Vec<IncomingMessage>,
    ) {
        // Load system prompt from workspace files
        let system_prompt = self.build_system_prompt(workspace).await;
        self.messages.push(ChatMessage::system(system_prompt));

        // Load continuity state
        if let Some(state) = self.load_continuity_state(workspace).await {
            self.messages.push(ChatMessage::system(format!(
                "Continuing session. Last cycle you were:\n{}",
                state
            )));
        }

        // Add any queued messages first
        for msg in queued_messages {
            self.messages.push(ChatMessage::user(format!(
                "[{}] {}: {}",
                msg.channel, msg.author.name, msg.content
            )));
        }

        // Add wake trigger
        self.messages.push(self.format_trigger(&trigger));
    }

    async fn build_system_prompt(&self, workspace: &Path) -> String {
        let mut parts = Vec::new();

        // Load workspace files
        for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
            if let Ok(content) = tokio::fs::read_to_string(workspace.join(filename)).await {
                parts.push(content);
            }
        }

        // Add system state
        let now = chrono::Utc::now();
        parts.push(format!("Current time: {}", now.to_rfc3339()));

        if parts.is_empty() {
            "You are an AI assistant.".to_string()
        } else {
            parts.join("\n\n---\n\n")
        }
    }

    async fn load_continuity_state(&self, workspace: &Path) -> Option<String> {
        let path = workspace.join("thinking/current-state.md");
        tokio::fs::read_to_string(path).await.ok()
    }

    fn format_trigger(&self, trigger: &WakeTrigger) -> ChatMessage {
        match trigger {
            WakeTrigger::Message(msg) => ChatMessage::user(format!(
                "[{}] {}: {}",
                msg.channel, msg.author.name, msg.content
            )),
            WakeTrigger::Heartbeat => ChatMessage::system(
                "Heartbeat wake. No new messages. Check on your tasks and state."
            ),
        }
    }

    /// Add tool results with any incoming messages
    pub fn add_tool_results(
        &mut self,
        results: Vec<ToolCallResponse>,
        incoming: Vec<IncomingMessage>,
        context_status: ContextStatus,
    ) {
        // Add each tool result
        for result in results {
            let content = match result.result {
                Ok(r) => r.output,
                Err(e) => format!("Error: {}", e),
            };
            self.messages.push(ChatMessage::tool(result.tool_call_id, content));
        }

        // Add context status
        self.messages.push(ChatMessage::system(format!(
            "Context: {}/{} ({:.1}%)",
            context_status.used, context_status.limit, context_status.percent()
        )));

        // Add any incoming messages
        if !incoming.is_empty() {
            let mut content = String::from("Messages received during tool execution:\n");
            for msg in incoming {
                content.push_str(&format!(
                    "- [{}] {}: {}\n",
                    msg.channel, msg.author.name, msg.content
                ));
            }
            self.messages.push(ChatMessage::system(content));
        }
    }

    /// Add assistant message from model response
    pub fn add_assistant_response(&mut self, content: Option<String>, tool_calls: Option<Vec<ToolCallRequest>>) {
        self.messages.push(ChatMessage::assistant(content, tool_calls));
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_system() {
        let msg = ChatMessage::system("Hello");
        assert_eq!(msg.role, "system");
        assert_eq!(msg.content, Some("Hello".to_string()));
        assert!(msg.tool_calls.is_none());
    }

    #[test]
    fn test_chat_message_user() {
        let msg = ChatMessage::user("Hi there");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, Some("Hi there".to_string()));
    }

    #[test]
    fn test_chat_message_tool() {
        let msg = ChatMessage::tool("call_123", "Result");
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.tool_call_id, Some("call_123".to_string()));
        assert_eq!(msg.content, Some("Result".to_string()));
    }

    #[test]
    fn test_context_builder_clear() {
        let mut builder = ContextBuilder::new();
        builder.add_message(ChatMessage::system("test"));
        assert_eq!(builder.messages().len(), 1);
        builder.clear();
        assert!(builder.messages().is_empty());
    }

    #[test]
    fn test_chat_message_serialization() {
        let msg = ChatMessage::system("test");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"system\""));
        assert!(json.contains("\"content\":\"test\""));
        // Optional None fields should be skipped
        assert!(!json.contains("tool_calls"));
    }
}
```

- [ ] **Step 3.2: Run tests**

Run: `cargo test -p river-gateway loop::context --lib`
Expected: 5 tests pass

- [ ] **Step 3.3: Update loop/mod.rs**

```rust
//! Agent loop module

pub mod state;
pub mod queue;
pub mod context;

pub use state::{LoopEvent, LoopState, WakeTrigger};
pub use queue::MessageQueue;
pub use context::{ChatMessage, ContextBuilder, ToolCallRequest, FunctionCall};
```

- [ ] **Step 3.4: Run all loop tests**

Run: `cargo test -p river-gateway loop --lib`
Expected: PASS

- [ ] **Step 3.5: Commit**

```bash
git add crates/river-gateway/src/loop/
git commit -m "feat(gateway): add context builder and chat message types"
```

---

## Task 4: Model Client

**Files:**
- Create: `crates/river-gateway/src/loop/model.rs`
- Modify: `crates/river-gateway/src/loop/mod.rs`

### Step 4.1: Create ModelClient

- [ ] Create `crates/river-gateway/src/loop/model.rs`:

```rust
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
    pub fn new(url: String, model: String, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self { url, model, http }
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

        Ok(ModelResponse::from(completion))
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

impl From<ChatCompletionResponse> for ModelResponse {
    fn from(resp: ChatCompletionResponse) -> Self {
        let choice = resp.choices.into_iter().next().unwrap_or(Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: None,
            },
        });

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

        Self {
            content: choice.message.content,
            tool_calls,
            usage: resp.usage,
        }
    }
}

impl ModelResponse {
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
        );
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
```

- [ ] **Step 4.2: Run tests**

Run: `cargo test -p river-gateway loop::model --lib`
Expected: 3 tests pass

- [ ] **Step 4.3: Update loop/mod.rs**

```rust
//! Agent loop module

pub mod state;
pub mod queue;
pub mod context;
pub mod model;

pub use state::{LoopEvent, LoopState, WakeTrigger};
pub use queue::MessageQueue;
pub use context::{ChatMessage, ContextBuilder, ToolCallRequest, FunctionCall};
pub use model::{ModelClient, ModelResponse, Usage};
```

- [ ] **Step 4.4: Run all loop tests**

Run: `cargo test -p river-gateway loop --lib`
Expected: PASS

- [ ] **Step 4.5: Commit**

```bash
git add crates/river-gateway/src/loop/
git commit -m "feat(gateway): add model client for OpenAI-compatible API"
```

---

## Task 5: AgentLoop Core

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs`

### Step 5.1: Add AgentLoop struct and phase methods

- [ ] Rewrite `crates/river-gateway/src/loop/mod.rs`:

```rust
//! Agent loop module - the heart of the agent

pub mod state;
pub mod queue;
pub mod context;
pub mod model;

pub use state::{LoopEvent, LoopState, WakeTrigger};
pub use queue::MessageQueue;
pub use context::{ChatMessage, ContextBuilder, ToolCallRequest, FunctionCall};
pub use model::{ModelClient, ModelResponse, Usage};

use crate::db::Database;
use crate::tools::{ToolExecutor, ToolCall};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};

/// Configuration for the agent loop
#[derive(Debug, Clone)]
pub struct LoopConfig {
    /// Workspace path for loading context files
    pub workspace: PathBuf,
    /// Default heartbeat interval in minutes
    pub default_heartbeat_minutes: u32,
    /// Context limit (tokens)
    pub context_limit: u64,
    /// Model timeout
    pub model_timeout: Duration,
    /// Maximum tool calls per generation (safety limit)
    pub max_tool_calls_per_generation: usize,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            default_heartbeat_minutes: 45,
            context_limit: 65536,
            model_timeout: Duration::from_secs(120),
            max_tool_calls_per_generation: 50,
        }
    }
}

/// The agent loop actor
pub struct AgentLoop {
    state: LoopState,
    event_rx: mpsc::Receiver<LoopEvent>,
    message_queue: Arc<MessageQueue>,
    model_client: ModelClient,
    context: ContextBuilder,
    tool_executor: Arc<RwLock<ToolExecutor>>,
    db: Arc<Mutex<Database>>,
    config: LoopConfig,
    pending_tool_calls: Vec<ToolCallRequest>,
    shutdown_requested: bool,
}

impl AgentLoop {
    pub fn new(
        event_rx: mpsc::Receiver<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        db: Arc<Mutex<Database>>,
        config: LoopConfig,
    ) -> Self {
        Self {
            state: LoopState::Sleeping,
            event_rx,
            message_queue,
            model_client,
            context: ContextBuilder::new(),
            tool_executor,
            db,
            config,
            pending_tool_calls: Vec::new(),
            shutdown_requested: false,
        }
    }

    /// Run the continuous loop
    pub async fn run(&mut self) {
        tracing::info!("Agent loop started");

        loop {
            match &self.state {
                LoopState::Sleeping => {
                    self.sleep_phase().await;
                }
                LoopState::Waking { .. } => {
                    self.wake_phase().await;
                }
                LoopState::Thinking => {
                    self.think_phase().await;
                }
                LoopState::Acting => {
                    self.act_phase().await;
                }
                LoopState::Settling => {
                    self.settle_phase().await;
                }
            }

            if self.shutdown_requested && self.state.is_sleeping() {
                break;
            }
        }

        tracing::info!("Agent loop stopped");
    }

    async fn sleep_phase(&mut self) {
        let heartbeat_delay = Duration::from_secs(
            self.config.default_heartbeat_minutes as u64 * 60
        );

        tokio::select! {
            event = self.event_rx.recv() => {
                match event {
                    Some(LoopEvent::Message(msg)) => {
                        tracing::info!("Wake: message from {} in {}", msg.author.name, msg.channel);
                        self.state = LoopState::Waking {
                            trigger: WakeTrigger::Message(msg)
                        };
                    }
                    Some(LoopEvent::Heartbeat) => {
                        tracing::info!("Wake: heartbeat");
                        self.state = LoopState::Waking {
                            trigger: WakeTrigger::Heartbeat
                        };
                    }
                    Some(LoopEvent::Shutdown) => {
                        tracing::info!("Shutdown requested");
                        self.shutdown_requested = true;
                    }
                    None => {
                        tracing::info!("Event channel closed");
                        self.shutdown_requested = true;
                    }
                }
            }
            _ = tokio::time::sleep(heartbeat_delay) => {
                tracing::info!("Wake: heartbeat timer");
                self.state = LoopState::Waking {
                    trigger: WakeTrigger::Heartbeat
                };
            }
        }
    }

    async fn wake_phase(&mut self) {
        let trigger = match std::mem::take(&mut self.state) {
            LoopState::Waking { trigger } => trigger,
            _ => {
                tracing::error!("Invalid state in wake_phase");
                self.state = LoopState::Sleeping;
                return;
            }
        };

        // Drain any messages that arrived before we woke
        let queued_messages = self.message_queue.drain();
        if !queued_messages.is_empty() {
            tracing::info!("Processing {} queued messages", queued_messages.len());
        }

        // Assemble context
        self.context.clear();
        self.context.assemble(
            &self.config.workspace,
            trigger,
            queued_messages,
        ).await;

        // Load tool schemas
        let executor = self.tool_executor.read().await;
        self.context.set_tools(executor.schemas());

        self.state = LoopState::Thinking;
    }

    async fn think_phase(&mut self) {
        tracing::debug!("Calling model...");

        let response = match self.model_client.complete(
            self.context.messages(),
            self.context.tools(),
        ).await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::error!("Model call failed: {}", e);
                self.state = LoopState::Settling;
                return;
            }
        };

        tracing::debug!(
            "Model response: {} tokens, {} tool calls",
            response.usage.total_tokens,
            response.tool_calls.len()
        );

        // Add assistant message to context
        self.context.add_assistant_response(
            response.content.clone(),
            if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            },
        );

        // Update context usage tracking
        {
            let mut executor = self.tool_executor.write().await;
            executor.add_context(response.usage.total_tokens as u64);
        }

        if response.tool_calls.is_empty() {
            // No tool calls - cycle complete
            if let Some(content) = &response.content {
                tracing::debug!("Assistant said: {}", content);
            }
            self.state = LoopState::Settling;
        } else {
            // Has tool calls - execute them
            self.pending_tool_calls = response.tool_calls;
            self.state = LoopState::Acting;
        }
    }

    async fn act_phase(&mut self) {
        let tool_calls = std::mem::take(&mut self.pending_tool_calls);
        tracing::debug!("Executing {} tool calls", tool_calls.len());

        // Convert to executor format and execute
        let mut results = Vec::new();
        {
            let mut executor = self.tool_executor.write().await;
            for tc in &tool_calls {
                let call = ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                };
                let result = executor.execute(&call);
                tracing::debug!("Tool {}: {:?}", tc.function.name, result.result.is_ok());
                results.push(result);
            }
        }

        // Drain any messages that arrived during tool execution
        let incoming_messages = self.message_queue.drain();
        if !incoming_messages.is_empty() {
            tracing::info!("{} messages arrived during tool execution", incoming_messages.len());
        }

        // Get current context status
        let context_status = {
            let executor = self.tool_executor.read().await;
            executor.context_status()
        };

        if context_status.is_near_limit() {
            tracing::warn!("Context at {:.1}% - approaching limit", context_status.percent());
        }

        // Add tool results and incoming messages to context
        self.context.add_tool_results(results, incoming_messages, context_status);

        // Back to thinking
        self.state = LoopState::Thinking;
    }

    async fn settle_phase(&mut self) {
        tracing::debug!("Settling...");

        // TODO: Persist messages to database
        // TODO: Git commit if workspace changed

        // Check if messages arrived during settle
        if !self.message_queue.is_empty() {
            let messages = self.message_queue.drain();
            let msg = messages.into_iter().next().unwrap();
            tracing::info!("Message arrived during settle, immediate wake");
            self.state = LoopState::Waking {
                trigger: WakeTrigger::Message(msg),
            };
        } else {
            self.state = LoopState::Sleeping;
        }
    }
}

impl Default for LoopState {
    fn default() -> Self {
        Self::Sleeping
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_config_default() {
        let config = LoopConfig::default();
        assert_eq!(config.default_heartbeat_minutes, 45);
        assert_eq!(config.context_limit, 65536);
        assert_eq!(config.max_tool_calls_per_generation, 50);
    }
}
```

- [ ] **Step 5.2: Run all loop tests**

Run: `cargo test -p river-gateway loop --lib`
Expected: PASS

- [ ] **Step 5.3: Run cargo check**

Run: `cargo check -p river-gateway`
Expected: No errors

- [ ] **Step 5.4: Commit**

```bash
git add crates/river-gateway/src/loop/
git commit -m "feat(gateway): implement AgentLoop actor with all phases"
```

---

## Task 6: Integration - Update AppState

**Files:**
- Modify: `crates/river-gateway/src/state.rs`

### Step 6.1: Add loop channel and queue to AppState

- [ ] Read current state.rs and update it:

```rust
//! Shared application state

use crate::db::Database;
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::redis::{RedisClient, RedisConfig};
use crate::r#loop::{LoopEvent, MessageQueue};
use crate::tools::{ToolExecutor, ToolRegistry};
use river_core::{AgentBirth, SnowflakeGenerator};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, RwLock};

/// Shared application state
pub struct AppState {
    pub config: GatewayConfig,
    pub db: Arc<Mutex<Database>>,
    pub snowflake_gen: Arc<SnowflakeGenerator>,
    pub tool_executor: Arc<RwLock<ToolExecutor>>,
    pub embedding_client: Option<Arc<EmbeddingClient>>,
    pub redis_client: Option<Arc<RedisClient>>,
    /// Channel to send events to the agent loop
    pub loop_tx: mpsc::Sender<LoopEvent>,
    /// Queue for messages arriving mid-generation
    pub message_queue: Arc<MessageQueue>,
}

/// Gateway configuration (runtime)
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub model_url: String,
    pub model_name: String,
    pub context_limit: u64,
    pub heartbeat_minutes: u32,
    pub agent_birth: AgentBirth,
    pub agent_name: String,
    pub embedding: Option<EmbeddingConfig>,
    pub redis: Option<RedisConfig>,
}

impl AppState {
    pub fn new(
        config: GatewayConfig,
        db: Arc<Mutex<Database>>,
        registry: ToolRegistry,
        embedding_client: Option<EmbeddingClient>,
        redis_client: Option<RedisClient>,
        loop_tx: mpsc::Sender<LoopEvent>,
        message_queue: Arc<MessageQueue>,
    ) -> Self {
        let executor = ToolExecutor::new(registry, config.context_limit);

        Self {
            snowflake_gen: Arc::new(SnowflakeGenerator::new(config.agent_birth)),
            db,
            tool_executor: Arc::new(RwLock::new(executor)),
            embedding_client: embedding_client.map(Arc::new),
            redis_client: redis_client.map(Arc::new),
            loop_tx,
            message_queue,
            config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::tools::ToolRegistry;

    #[test]
    fn test_state_creation() {
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth: AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap(),
            agent_name: "test".to_string(),
            embedding: None,
            redis: None,
        };

        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let (tx, _rx) = mpsc::channel(100);
        let queue = Arc::new(MessageQueue::new());
        let state = AppState::new(config, db, registry, None, None, tx, queue);

        assert_eq!(state.config.port, 3000);
        assert_eq!(state.config.context_limit, 65536);
        assert!(state.embedding_client.is_none());
        assert!(state.redis_client.is_none());
    }
}
```

- [ ] **Step 6.2: Run tests**

Run: `cargo test -p river-gateway state --lib`
Expected: PASS

- [ ] **Step 6.3: Commit**

```bash
git add crates/river-gateway/src/state.rs
git commit -m "feat(gateway): add loop channel and queue to AppState"
```

---

## Task 7: Integration - Update HTTP Handler

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

### Step 7.1: Update /incoming to send events to loop

- [ ] Update the `handle_incoming` function in `routes.rs`:

```rust
async fn handle_incoming(
    State(state): State<Arc<AppState>>,
    Json(msg): Json<IncomingMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    tracing::info!(
        "Received message from {} in {}",
        msg.author.name,
        msg.channel
    );

    // Send to the loop
    if state.loop_tx.send(LoopEvent::Message(msg)).await.is_err() {
        tracing::error!("Failed to send message to loop - channel closed");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(serde_json::json!({
        "status": "delivered"
    })))
}
```

- [ ] **Step 7.2: Add import for LoopEvent**

Add to the imports at the top of `routes.rs`:

```rust
use crate::r#loop::LoopEvent;
```

- [ ] **Step 7.3: Run tests**

Run: `cargo test -p river-gateway api --lib`
Expected: Tests may need updating due to AppState changes

- [ ] **Step 7.4: Update test helper in routes.rs**

Update `test_state()` function in the tests module:

```rust
fn test_state() -> Arc<AppState> {
    use crate::r#loop::MessageQueue;
    use tokio::sync::mpsc;

    let config = GatewayConfig {
        workspace: PathBuf::from("/tmp/test"),
        data_dir: PathBuf::from("/tmp/test"),
        port: 3000,
        model_url: "http://localhost:8080".to_string(),
        model_name: "test".to_string(),
        context_limit: 65536,
        heartbeat_minutes: 45,
        agent_birth: AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap(),
        agent_name: "test-agent".to_string(),
        embedding: None,
        redis: None,
    };

    let db = Arc::new(std::sync::Mutex::new(Database::open_in_memory().unwrap()));
    let registry = ToolRegistry::new();
    let (tx, _rx) = mpsc::channel(100);
    let queue = Arc::new(MessageQueue::new());
    Arc::new(AppState::new(config, db, registry, None, None, tx, queue))
}
```

- [ ] **Step 7.5: Run tests again**

Run: `cargo test -p river-gateway api --lib`
Expected: PASS

- [ ] **Step 7.6: Commit**

```bash
git add crates/river-gateway/src/api/routes.rs
git commit -m "feat(gateway): route incoming messages to agent loop"
```

---

## Task 8: Integration - Update Server Startup

**Files:**
- Modify: `crates/river-gateway/src/server.rs`

### Step 8.1: Spawn AgentLoop on startup

- [ ] Update `server.rs` to create and spawn the agent loop:

```rust
//! Server setup and initialization

use crate::api::create_router;
use crate::db::init_db;
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::redis::{RedisClient, RedisConfig};
use crate::state::{AppState, GatewayConfig};
use crate::r#loop::{AgentLoop, LoopConfig, LoopEvent, MessageQueue, ModelClient};
use crate::tools::{
    BashTool, EditTool, EmbedTool, GlobTool, GrepTool, MemoryDeleteTool, MemoryDeleteBySourceTool,
    MemorySearchTool, ReadTool, ToolRegistry, WriteTool,
};
use chrono::{Datelike, Timelike};
use river_core::AgentBirth;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Server configuration from CLI args
pub struct ServerConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub agent_name: String,
    pub model_url: Option<String>,
    pub model_name: Option<String>,
    pub embedding_url: Option<String>,
    pub redis_url: Option<String>,
    pub orchestrator_url: Option<String>,
}

/// Initialize and run the gateway server
pub async fn run(config: ServerConfig) -> anyhow::Result<()> {
    // Initialize database
    let db_path = config.data_dir.join("river.db");
    let db = init_db(&db_path)?;

    // Create embedding client if configured
    let embedding_client = if let Some(url) = &config.embedding_url {
        let embed_config = EmbeddingConfig {
            url: url.clone(),
            ..Default::default()
        };
        Some(EmbeddingClient::new(embed_config))
    } else {
        None
    };

    // Create Redis client if configured
    let redis_client = if let Some(url) = &config.redis_url {
        let redis_config = RedisConfig {
            url: url.clone(),
            agent_name: config.agent_name.clone(),
        };
        Some(RedisClient::new(redis_config).await?)
    } else {
        None
    };

    // Create agent birth (current time)
    let now = chrono::Utc::now();
    let agent_birth = AgentBirth::new(
        now.year() as u16,
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )?;

    // Create gateway config
    let model_url = config.model_url.clone().unwrap_or_else(|| "http://localhost:8080".to_string());
    let model_name = config.model_name.clone().unwrap_or_else(|| "default".to_string());
    let agent_name = config.agent_name.clone();

    let gateway_config = GatewayConfig {
        workspace: config.workspace.clone(),
        data_dir: config.data_dir.clone(),
        port: config.port,
        model_url: model_url.clone(),
        model_name: model_name.clone(),
        context_limit: 65536,
        heartbeat_minutes: 45,
        agent_birth,
        agent_name: agent_name.clone(),
        embedding: config.embedding_url.as_ref().map(|url| EmbeddingConfig {
            url: url.clone(),
            ..Default::default()
        }),
        redis: config.redis_url.as_ref().map(|url| RedisConfig {
            url: url.clone(),
            agent_name: agent_name.clone(),
        }),
    };

    // Wrap database in Arc for sharing
    let db_arc = Arc::new(std::sync::Mutex::new(db));
    let snowflake_gen = Arc::new(river_core::SnowflakeGenerator::new(gateway_config.agent_birth));

    // Create tool registry with core tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(&config.workspace)));
    registry.register(Box::new(WriteTool::new(&config.workspace)));
    registry.register(Box::new(EditTool::new(&config.workspace)));
    registry.register(Box::new(GlobTool::new(&config.workspace)));
    registry.register(Box::new(GrepTool::new(&config.workspace)));
    registry.register(Box::new(BashTool::new(&config.workspace)));

    // Register memory tools if embedding client is available
    if let Some(ref embed_client) = embedding_client {
        let embed_arc = Arc::new(embed_client.clone());
        registry.register(Box::new(EmbedTool::new(
            db_arc.clone(),
            embed_arc.clone(),
            snowflake_gen.clone(),
        )));
        registry.register(Box::new(MemorySearchTool::new(db_arc.clone(), embed_arc.clone())));
        registry.register(Box::new(MemoryDeleteTool::new(db_arc.clone())));
        registry.register(Box::new(MemoryDeleteBySourceTool::new(db_arc.clone())));
        tracing::info!("Registered memory tools (embed, memory_search, memory_delete, memory_delete_by_source)");
    }

    // Register Redis tools if client is available
    if let Some(ref redis) = redis_client {
        let redis_arc = Arc::new(redis.clone());
        use crate::redis::*;
        registry.register(Box::new(WorkingMemorySetTool::new(redis_arc.clone())));
        registry.register(Box::new(WorkingMemoryGetTool::new(redis_arc.clone())));
        registry.register(Box::new(WorkingMemoryDeleteTool::new(redis_arc.clone())));
        registry.register(Box::new(MediumTermSetTool::new(redis_arc.clone())));
        registry.register(Box::new(MediumTermGetTool::new(redis_arc.clone())));
        registry.register(Box::new(ResourceLockTool::new(redis_arc.clone())));
        registry.register(Box::new(CounterIncrementTool::new(redis_arc.clone())));
        registry.register(Box::new(CounterGetTool::new(redis_arc.clone())));
        registry.register(Box::new(CacheSetTool::new(redis_arc.clone())));
        registry.register(Box::new(CacheGetTool::new(redis_arc.clone())));
        tracing::info!("Registered Redis tools (10 tools)");
    }

    tracing::info!("Registered {} tools total", registry.names().len());

    // Create event channel for the loop
    let (event_tx, event_rx) = mpsc::channel::<LoopEvent>(100);

    // Create message queue (shared between HTTP handlers and loop)
    let message_queue = Arc::new(MessageQueue::new());

    // Create tool executor (shared)
    let tool_executor = Arc::new(tokio::sync::RwLock::new(
        crate::tools::ToolExecutor::new(registry, gateway_config.context_limit)
    ));

    // Create model client
    let model_client = ModelClient::new(
        model_url,
        model_name,
        Duration::from_secs(120),
    );

    // Create loop config
    let loop_config = LoopConfig {
        workspace: config.workspace.clone(),
        default_heartbeat_minutes: 45,
        context_limit: 65536,
        model_timeout: Duration::from_secs(120),
        max_tool_calls_per_generation: 50,
    };

    // Spawn the agent loop
    let mut agent_loop = AgentLoop::new(
        event_rx,
        message_queue.clone(),
        model_client,
        tool_executor.clone(),
        db_arc.clone(),
        loop_config,
    );

    let loop_handle = tokio::spawn(async move {
        agent_loop.run().await;
    });

    tracing::info!("Agent loop spawned");

    // Create a dummy registry for AppState (tools are in executor now)
    let empty_registry = ToolRegistry::new();

    // Create app state (now with loop channel)
    let state = Arc::new(AppState {
        config: gateway_config,
        db: db_arc,
        snowflake_gen,
        tool_executor,
        embedding_client: embedding_client.map(Arc::new),
        redis_client: redis_client.map(Arc::new),
        loop_tx: event_tx,
        message_queue,
    });

    // Create router
    let app = create_router(state);

    // Start heartbeat task if orchestrator configured
    if let Some(orchestrator_url) = &config.orchestrator_url {
        let gateway_url = format!("http://127.0.0.1:{}", config.port);
        let heartbeat_client = crate::heartbeat::HeartbeatClient::new(
            orchestrator_url.clone(),
            config.agent_name.clone(),
            gateway_url,
        );

        tokio::spawn(async move {
            heartbeat_client.run_loop(30).await;
        });

        tracing::info!("Started heartbeat to orchestrator: {}", orchestrator_url);
    }

    // Bind and serve
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    tracing::info!("Gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    // Wait for loop to finish on shutdown
    let _ = loop_handle.await;

    Ok(())
}
```

- [ ] **Step 8.2: Run cargo check**

Run: `cargo check -p river-gateway`
Expected: No errors (warnings OK)

- [ ] **Step 8.3: Run all tests**

Run: `cargo test -p river-gateway --lib`
Expected: PASS

- [ ] **Step 8.4: Commit**

```bash
git add crates/river-gateway/src/server.rs
git commit -m "feat(gateway): spawn agent loop on server startup"
```

---

## Task 9: Final Integration Test

**Files:**
- Run integration tests

### Step 9.1: Build the project

- [ ] Run: `cargo build -p river-gateway`
Expected: Success

### Step 9.2: Run all tests

- [ ] Run: `cargo test -p river-gateway`
Expected: All tests pass

### Step 9.3: Final commit

- [ ] Commit any remaining changes:

```bash
git add -A
git commit -m "feat(gateway): complete continuous tool loop implementation"
```

---

## Summary

This plan implements the continuous tool loop in 9 tasks:

1. **Loop State Types** - LoopState, LoopEvent, WakeTrigger enums
2. **Message Queue** - Thread-safe queue for mid-generation messages
3. **ChatMessage and Context** - Context builder and message types
4. **Model Client** - OpenAI-compatible HTTP client
5. **AgentLoop Core** - The actor with all phase methods
6. **Update AppState** - Add loop channel and queue
7. **Update HTTP Handler** - Route /incoming to loop
8. **Update Server Startup** - Spawn loop on startup
9. **Final Integration Test** - Verify everything works

After completion, the gateway will:
- Run a continuous WAKE → THINK → ACT → SETTLE → SLEEP cycle
- Wake on incoming messages or heartbeat timer
- Execute tools and return results to model
- Inject mid-generation messages with tool results
- Track context usage and warn when approaching limit

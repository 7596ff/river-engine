# Gateway Continuous Tool Loop Design

**Version:** 1.0
**Date:** 2026-03-17
**Status:** Draft
**Parent Spec:** 2026-03-16-river-engine-design.md

---

## 1. Overview

This spec details the implementation of the continuous tool loop in river-gateway. The loop is the heart of the agent - all cognition happens here.

### Design Philosophy

- The agent is never "done" - generations chain together in continuous cognition
- The end of one generation is the beginning of another
- The loop is the mind; new information must reach it as it arrives
- Sleep is a wait state, not termination
- Communication happens explicitly through tool calls, not implicit "responses"

### Architecture Style

**Actor-based.** The loop runs as a tokio task, receiving events through a channel. This cleanly separates HTTP handling from loop logic and naturally models continuous operation.

---

## 2. Core Components

### 2.1 LoopEvent

Events that can wake or signal the loop.

```rust
pub enum LoopEvent {
    /// Message from communication adapter
    Message(IncomingMessage),
    /// Heartbeat timer fired
    Heartbeat,
    /// Graceful shutdown requested
    Shutdown,
}
```

### 2.2 WakeTrigger

What caused the agent to wake (captured for context assembly).

```rust
pub enum WakeTrigger {
    /// User or external message
    Message(IncomingMessage),
    /// Scheduled heartbeat
    Heartbeat,
    /// Subagent spawn (future)
    SubagentSpawn { task: String },
}
```

### 2.3 LoopState

The agent's current phase in the cycle.

```rust
pub enum LoopState {
    /// Waiting for next event
    Sleeping,
    /// Woke up, assembling context
    Waking { trigger: WakeTrigger },
    /// Model is generating
    Thinking,
    /// Executing tool calls
    Acting { pending: Vec<ToolCall> },
    /// Cycle complete, committing state
    Settling,
}
```

### 2.4 MessageQueue

Thread-safe queue for messages arriving mid-generation.

```rust
pub struct MessageQueue {
    inner: Mutex<VecDeque<IncomingMessage>>,
}

impl MessageQueue {
    pub fn new() -> Self;
    pub fn push(&self, msg: IncomingMessage);
    pub fn drain(&self) -> Vec<IncomingMessage>;
    pub fn is_empty(&self) -> bool;
}
```

### 2.5 AgentLoop

The actor that runs the continuous loop.

```rust
pub struct AgentLoop {
    // Current state
    state: LoopState,

    // Event channel
    event_rx: mpsc::Receiver<LoopEvent>,

    // Mid-generation message queue
    message_queue: Arc<MessageQueue>,

    // Model communication
    model_client: ModelClient,

    // Context assembly
    context: ContextBuilder,

    // Tool execution
    tool_executor: Arc<RwLock<ToolExecutor>>,

    // Database access
    db: Arc<Mutex<Database>>,

    // Configuration
    config: LoopConfig,

    // Heartbeat scheduling
    next_heartbeat: Option<Instant>,
    heartbeat_handle: Option<JoinHandle<()>>,
}
```

### 2.6 LoopConfig

Configuration for loop behavior.

```rust
pub struct LoopConfig {
    /// Workspace path for loading context files
    pub workspace: PathBuf,
    /// Default heartbeat interval
    pub default_heartbeat_minutes: u32,
    /// Context limit (tokens)
    pub context_limit: u64,
    /// Model timeout
    pub model_timeout: Duration,
    /// Maximum tool calls per generation (safety limit)
    pub max_tool_calls_per_generation: usize,
}
```

---

## 3. Loop Lifecycle

### 3.1 State Machine

```
                    ┌─────────────────────────────────────┐
                    │                                     │
                    ▼                                     │
┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐│
│ SLEEPING │──▶│  WAKING  │──▶│ THINKING │──▶│  ACTING  ││
│          │   │          │   │          │   │          ││
└──────────┘   └──────────┘   └──────────┘   └────┬─────┘│
     ▲                                            │      │
     │                                            │      │
     │         ┌──────────┐                       │      │
     └─────────│ SETTLING │◀──────────────────────┘      │
               │          │   (no tool calls)            │
               └──────────┘                              │
                                                         │
                    (has tool calls) ────────────────────┘
```

### 3.2 Main Loop

```rust
impl AgentLoop {
    pub async fn run(&mut self) {
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
                LoopState::Acting { .. } => {
                    self.act_phase().await;
                }
                LoopState::Settling => {
                    self.settle_phase().await;
                }
            }

            // Check for shutdown
            if matches!(self.state, LoopState::Sleeping) {
                if self.check_shutdown() {
                    break;
                }
            }
        }
    }
}
```

---

## 4. Phase Implementations

### 4.1 SLEEP Phase

Wait for the next wake trigger.

```rust
async fn sleep_phase(&mut self) {
    // Cancel any pending heartbeat timer
    self.cancel_heartbeat_timer();

    // Schedule next heartbeat
    let heartbeat_delay = self.next_heartbeat
        .unwrap_or_else(|| Duration::from_secs(self.config.default_heartbeat_minutes as u64 * 60));

    let event_rx = &mut self.event_rx;

    tokio::select! {
        // Wait for external event
        event = event_rx.recv() => {
            match event {
                Some(LoopEvent::Message(msg)) => {
                    self.state = LoopState::Waking {
                        trigger: WakeTrigger::Message(msg)
                    };
                }
                Some(LoopEvent::Heartbeat) => {
                    self.state = LoopState::Waking {
                        trigger: WakeTrigger::Heartbeat
                    };
                }
                Some(LoopEvent::Shutdown) | None => {
                    // Will exit on next iteration
                }
            }
        }
        // Heartbeat timer
        _ = tokio::time::sleep(heartbeat_delay) => {
            self.state = LoopState::Waking {
                trigger: WakeTrigger::Heartbeat
            };
        }
    }
}
```

### 4.2 WAKE Phase

Assemble context for the model.

```rust
async fn wake_phase(&mut self) {
    let trigger = match &self.state {
        LoopState::Waking { trigger } => trigger.clone(),
        _ => unreachable!(),
    };

    // Drain any messages that arrived before we woke
    let queued_messages = self.message_queue.drain();

    // Assemble context
    self.context.clear();
    self.context.assemble(&self.config.workspace, trigger, queued_messages).await;

    // Load tool schemas
    let executor = self.tool_executor.read().await;
    self.context.set_tools(executor.schemas());

    self.state = LoopState::Thinking;
}
```

### 4.3 THINK Phase

Call the model and get a response.

```rust
async fn think_phase(&mut self) {
    let response = match self.model_client.complete(&self.context).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Model call failed: {}", e);
            // On model failure, settle and sleep
            // Agent will retry on next wake
            self.state = LoopState::Settling;
            return;
        }
    };

    // Update context with assistant message
    self.context.add_assistant_message(&response);

    // Track context usage
    {
        let mut executor = self.tool_executor.write().await;
        executor.add_context(response.usage.total_tokens as u64);
    }

    if response.tool_calls.is_empty() {
        // No tool calls - cycle complete
        self.state = LoopState::Settling;
    } else {
        // Has tool calls - execute them
        self.state = LoopState::Acting {
            pending: response.tool_calls
        };
    }
}
```

### 4.4 ACT Phase

Execute tool calls and collect results.

```rust
async fn act_phase(&mut self) {
    let pending = match &self.state {
        LoopState::Acting { pending } => pending.clone(),
        _ => unreachable!(),
    };

    // Execute tools
    let mut results = Vec::new();
    {
        let mut executor = self.tool_executor.write().await;
        for call in &pending {
            let result = executor.execute(call);
            results.push(result);
        }
    }

    // Drain any messages that arrived during tool execution
    let incoming_messages = self.message_queue.drain();

    // Get current context status
    let context_status = {
        let executor = self.tool_executor.read().await;
        executor.context_status()
    };

    // Add tool results and incoming messages to context
    self.context.add_tool_results(results, incoming_messages, context_status);

    // Check context limit
    if context_status.is_near_limit() {
        tracing::warn!("Context at {}% - approaching limit", context_status.percent());
        // Agent sees this in context_status and can call rotate_context
    }

    // Back to thinking
    self.state = LoopState::Thinking;
}
```

### 4.5 SETTLE Phase

Commit state and prepare for sleep.

```rust
async fn settle_phase(&mut self) {
    // Persist messages to database
    self.persist_conversation().await;

    // Git commit if workspace changed
    if let Err(e) = self.git_commit_if_changed().await {
        tracing::warn!("Git commit failed: {}", e);
        // Not fatal - continue
    }

    // Reset heartbeat to default (agent may have adjusted it)
    self.next_heartbeat = None;

    // Check if messages arrived during settle
    if !self.message_queue.is_empty() {
        // Immediate wake - don't sleep
        let msg = self.message_queue.drain().remove(0);
        self.state = LoopState::Waking {
            trigger: WakeTrigger::Message(msg),
        };
    } else {
        self.state = LoopState::Sleeping;
    }
}
```

---

## 5. Context Assembly

### 5.1 ContextBuilder

Manages the conversation context sent to the model.

```rust
pub struct ContextBuilder {
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSchema>,
    system_prompt: String,
}

impl ContextBuilder {
    pub fn new() -> Self;

    /// Clear all messages (for new cycle)
    pub fn clear(&mut self);

    /// Assemble context for a new wake
    pub async fn assemble(
        &mut self,
        workspace: &Path,
        trigger: WakeTrigger,
        queued_messages: Vec<IncomingMessage>,
    );

    /// Add assistant response
    pub fn add_assistant_message(&mut self, response: &ModelResponse);

    /// Add tool results with any incoming messages
    pub fn add_tool_results(
        &mut self,
        results: Vec<ToolCallResponse>,
        incoming: Vec<IncomingMessage>,
        context_status: ContextStatus,
    );

    /// Set available tools
    pub fn set_tools(&mut self, tools: Vec<ToolSchema>);

    /// Get messages for API call
    pub fn messages(&self) -> &[ChatMessage];

    /// Get tools for API call
    pub fn tools(&self) -> &[ToolSchema];
}
```

### 5.2 System Prompt Assembly

```rust
async fn build_system_prompt(workspace: &Path) -> String {
    let mut parts = Vec::new();

    // Load workspace files
    if let Ok(agents) = tokio::fs::read_to_string(workspace.join("AGENTS.md")).await {
        parts.push(agents);
    }
    if let Ok(identity) = tokio::fs::read_to_string(workspace.join("IDENTITY.md")).await {
        parts.push(identity);
    }
    if let Ok(rules) = tokio::fs::read_to_string(workspace.join("RULES.md")).await {
        parts.push(rules);
    }

    // Add system state
    let now = chrono::Utc::now();
    parts.push(format!("Current time: {}", now.to_rfc3339()));

    parts.join("\n\n---\n\n")
}
```

### 5.3 Continuity State

```rust
async fn load_continuity_state(workspace: &Path) -> Option<String> {
    let path = workspace.join("thinking/current-state.md");
    tokio::fs::read_to_string(path).await.ok()
}

fn format_continuity_message(state: Option<String>, trigger: &WakeTrigger) -> ChatMessage {
    let mut content = String::new();

    if let Some(state) = state {
        content.push_str(&format!("Continuing session. Last cycle you were:\n{}\n\n", state));
    }

    match trigger {
        WakeTrigger::Message(msg) => {
            content.push_str(&format!(
                "New message from {} in #{}: {}",
                msg.author.name, msg.channel, msg.content
            ));
        }
        WakeTrigger::Heartbeat => {
            content.push_str("Heartbeat wake. No new messages.");
        }
        WakeTrigger::SubagentSpawn { task } => {
            content.push_str(&format!("Subagent spawned for task: {}", task));
        }
    }

    ChatMessage::user(content)
}
```

### 5.4 Tool Results with Incoming Messages

When adding tool results to context, include any messages that arrived during execution:

```rust
fn format_tool_results(
    results: Vec<ToolCallResponse>,
    incoming: Vec<IncomingMessage>,
    context_status: ContextStatus,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    // Add each tool result
    for result in results {
        messages.push(ChatMessage::tool(
            result.tool_call_id,
            match result.result {
                Ok(r) => r.output,
                Err(e) => format!("Error: {}", e),
            },
        ));
    }

    // Add context status as system message
    messages.push(ChatMessage::system(format!(
        "Context: {}/{}  ({:.1}%)",
        context_status.used, context_status.limit, context_status.percent()
    )));

    // Add any incoming messages
    if !incoming.is_empty() {
        let mut content = String::from("Messages received during tool execution:\n");
        for msg in incoming {
            content.push_str(&format!(
                "- {} in #{}: {}\n",
                msg.author.name, msg.channel, msg.content
            ));
        }
        messages.push(ChatMessage::system(content));
    }

    messages
}
```

---

## 6. Model Client

### 6.1 ModelClient

OpenAI-compatible client for model inference.

```rust
pub struct ModelClient {
    url: String,
    model: String,
    http: reqwest::Client,
    timeout: Duration,
}

impl ModelClient {
    pub fn new(url: String, model: String, timeout: Duration) -> Self {
        Self {
            url,
            model,
            http: reqwest::Client::new(),
            timeout,
        }
    }

    pub async fn complete(&self, context: &ContextBuilder) -> Result<ModelResponse, ModelError> {
        let request = ChatCompletionRequest {
            model: &self.model,
            messages: context.messages(),
            tools: if context.tools().is_empty() {
                None
            } else {
                Some(context.tools())
            },
        };

        let response = self.http
            .post(format!("{}/v1/chat/completions", self.url))
            .timeout(self.timeout)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ModelError::ApiError { status, body });
        }

        let completion: ChatCompletionResponse = response.json().await?;
        Ok(ModelResponse::from(completion))
    }
}
```

### 6.2 Request/Response Types

```rust
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
    function: FunctionCall,
}

#[derive(Deserialize)]
struct FunctionCall {
    name: String,
    arguments: String,  // JSON string
}

pub struct ModelResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}
```

---

## 7. Integration with Gateway

### 7.1 Startup

In `server.rs`, spawn the loop actor:

```rust
pub async fn run(config: ServerConfig) -> anyhow::Result<()> {
    // ... existing setup ...

    // Create event channel
    let (event_tx, event_rx) = mpsc::channel::<LoopEvent>(100);

    // Create message queue (shared with HTTP handlers)
    let message_queue = Arc::new(MessageQueue::new());

    // Create model client
    let model_client = ModelClient::new(
        config.model_url.clone(),
        config.model_name.clone(),
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

    // Create and spawn agent loop
    let mut agent_loop = AgentLoop::new(
        event_rx,
        message_queue.clone(),
        model_client,
        loop_config,
        tool_executor.clone(),
        db_arc.clone(),
    );

    let loop_handle = tokio::spawn(async move {
        agent_loop.run().await;
    });

    // Store event_tx and message_queue in AppState for HTTP handlers
    let state = Arc::new(AppState {
        // ... existing fields ...
        loop_tx: event_tx,
        message_queue,
    });

    // ... rest of startup ...
}
```

### 7.2 HTTP Handler Update

Update `/incoming` to send events to the loop:

```rust
async fn handle_incoming(
    State(state): State<Arc<AppState>>,
    Json(msg): Json<IncomingMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // If loop is in THINK or ACT phase, queue the message
    // Otherwise, send as wake event

    // For simplicity, always send to channel - loop handles queuing
    if state.loop_tx.send(LoopEvent::Message(msg)).await.is_err() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(serde_json::json!({
        "status": "delivered"
    })))
}
```

### 7.3 AppState Updates

```rust
pub struct AppState {
    // ... existing fields ...

    /// Channel to send events to the agent loop
    pub loop_tx: mpsc::Sender<LoopEvent>,

    /// Queue for messages arriving mid-generation
    pub message_queue: Arc<MessageQueue>,
}
```

---

## 8. Git Integration

### 8.1 Auto-commit After Settle

```rust
async fn git_commit_if_changed(&self) -> Result<(), GitError> {
    let workspace = &self.config.workspace;

    // Check for changes
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace)
        .output()
        .await?;

    if status.stdout.is_empty() {
        return Ok(());  // No changes
    }

    // Check for conflicts
    let conflicts = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=U"])
        .current_dir(workspace)
        .output()
        .await?;

    if !conflicts.stdout.is_empty() {
        // Has conflicts - notify agent on next wake
        self.pending_notifications.push(
            "Git conflict detected. Workspace has conflicts that need resolution."
        );
        return Ok(());
    }

    // Stage all changes
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(workspace)
        .output()
        .await?;

    // Commit with timestamp
    let timestamp = chrono::Utc::now().to_rfc3339();
    Command::new("git")
        .args(["commit", "-m", &timestamp])
        .current_dir(workspace)
        .output()
        .await?;

    Ok(())
}
```

---

## 9. File Structure

```
crates/river-gateway/src/
├── loop/
│   ├── mod.rs          # AgentLoop struct and run()
│   ├── state.rs        # LoopState, LoopEvent, WakeTrigger
│   ├── context.rs      # ContextBuilder
│   ├── queue.rs        # MessageQueue
│   └── model.rs        # ModelClient, request/response types
├── session/
│   └── mod.rs          # (future: session management)
├── api/
│   ├── mod.rs
│   └── routes.rs       # Updated /incoming handler
├── state.rs            # AppState with loop_tx, message_queue
├── server.rs           # Spawn loop on startup
└── ...
```

---

## 10. Error Handling

### 10.1 Model Failures

- Transient failures (timeout, network): Log, transition to SETTLE, retry on next wake
- Persistent failures: After N consecutive failures, extend heartbeat interval exponentially

### 10.2 Tool Failures

- Tool errors are passed to the model as error results
- The agent decides how to handle them
- Infrastructure errors (database down) are logged and the tool returns an error

### 10.3 Git Failures

- Git errors are non-fatal
- Conflicts are surfaced to the agent on next wake
- Other errors are logged but don't stop the loop

---

## 11. Testing Strategy

### 11.1 Unit Tests

- `LoopState` transitions
- `MessageQueue` thread safety
- `ContextBuilder` assembly
- `ModelClient` request formatting

### 11.2 Integration Tests

- Full loop cycle with mock model
- Message injection mid-generation
- Git commit after settle
- Heartbeat scheduling

### 11.3 Mock Model

```rust
struct MockModelClient {
    responses: VecDeque<ModelResponse>,
}

impl MockModelClient {
    fn queue_response(&mut self, response: ModelResponse);
    async fn complete(&mut self, _: &ContextBuilder) -> Result<ModelResponse, ModelError>;
}
```

---

## 12. Future Considerations

### 12.1 Not In This Spec

- Sub-sessions (independent context windows)
- Subagent spawning
- Plugin tool registration
- Context rotation tool
- Schedule heartbeat tool

### 12.2 Dependencies

This spec depends on:
- Existing tool system (implemented)
- Database message storage (implemented)
- Context tracking (implemented)

This spec enables:
- Session management (future spec)
- Subagent system (future spec)
- Communication tools (future spec)

---

## Appendix A: ChatMessage Type

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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

    pub fn assistant(content: Option<String>, tool_calls: Option<Vec<ToolCall>>) -> Self {
        Self {
            role: "assistant".to_string(),
            content,
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool(tool_call_id: String, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            name: None,
        }
    }
}
```

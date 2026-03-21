# Agent Loop Architecture

This document details every step of the main agent loop in `river-gateway`. The loop is a state machine that cycles through five phases: **Sleeping → Waking → Thinking → Acting → Settling**.

## Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              AGENT LOOP                                      │
│                                                                              │
│   ┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐          │
│   │ SLEEPING │────▶│  WAKING  │────▶│ THINKING │────▶│  ACTING  │          │
│   └──────────┘     └──────────┘     └──────────┘     └──────────┘          │
│        ▲                                                   │                 │
│        │                                                   ▼                 │
│        │                                            ┌──────────┐            │
│        └────────────────────────────────────────────│ SETTLING │            │
│                                                     └──────────┘            │
└─────────────────────────────────────────────────────────────────────────────┘
```

## State Machine

| State | Description | Transitions To |
|-------|-------------|----------------|
| `Sleeping` | Idle, waiting for events | `Waking` |
| `Waking` | Building context for model call | `Thinking` |
| `Thinking` | Calling the model | `Acting` or `Settling` |
| `Acting` | Executing tool calls | `Thinking` or `Settling` |
| `Settling` | Persisting state, handling rotation, git commit | `Sleeping` |

---

## AgentLoop Structure

**File:** `loop/mod.rs:56-80`

The `AgentLoop` struct holds all state for a running agent:

```rust
pub struct AgentLoop {
    state: LoopState,                           // Current state machine state
    event_rx: mpsc::Receiver<LoopEvent>,        // Channel for incoming events
    message_queue: Arc<MessageQueue>,           // Buffers messages during non-sleeping phases
    model_client: ModelClient,                  // HTTP client for model API
    context: ContextBuilder,                    // Builds conversation context for model calls
    tool_executor: Arc<RwLock<ToolExecutor>>,   // Registry and executor for tools
    db: Arc<Mutex<Database>>,                   // SQLite database for persistence
    snowflake_gen: Arc<SnowflakeGenerator>,     // Generates unique IDs
    heartbeat_scheduler: Arc<HeartbeatScheduler>, // Manages heartbeat timing
    context_rotation: Arc<ContextRotation>,     // Tracks rotation requests
    config: LoopConfig,                         // Configuration settings
    shutdown_requested: bool,                   // Graceful shutdown flag
    git: GitOps,                                // Git operations for workspace
    pending_notifications: Vec<String>,         // System notifications for next wake
    needs_context_reset: bool,                  // Whether to rebuild context from scratch
    context_id: Option<Snowflake>,              // Current context ID (for persistence)
    context_file: Option<ContextFile>,          // JSONL file for context persistence
    last_prompt_tokens: u64,                    // Last known prompt token count from API
}
```

### Key Fields Explained

- **`last_prompt_tokens`**: The authoritative source of truth for context usage. Updated from `response.usage.prompt_tokens` after each model call. Used for all threshold checks (80% warning, 90% auto-rotation, 95% hard limit).

- **`needs_context_reset`**: Set to `true` on first startup and after context rotation. When true, the next wake phase builds fresh context from scratch rather than accumulating.

- **`context_file`**: JSONL file persistence. Each message is appended as a JSON line. On restart, this file is reloaded to resume context.

- **`context_id`**: Snowflake ID for the current context. Used for database tracking and archival.

---

## Context Tracking: Single Source of Truth

**File:** `loop/mod.rs:128-133`

Context usage is tracked via a single, authoritative mechanism:

```rust
fn context_status(&self) -> ContextStatus {
    ContextStatus {
        used: self.last_prompt_tokens,
        limit: self.config.context_limit,
    }
}
```

This helper method returns the current context status based on:
- **`used`**: The `prompt_tokens` value from the last model API response
- **`limit`**: The configured context limit (default: 65536 tokens)

### Why Single Source of Truth Matters

Previously, context was tracked in two places (AgentLoop and ToolExecutor) which could diverge. Now:
- `last_prompt_tokens` is updated once per model call from the API response
- All threshold checks use this single value
- No estimation or accumulation bugs possible

---

## Phase 1: Sleeping

**File:** `loop/mod.rs:266-301`

The agent is idle, waiting for something to happen. This phase blocks on an async select between the event channel and a heartbeat timer.

### What Happens

```rust
async fn sleep_phase(&mut self) {
    // 1. Get the delay until next heartbeat from the scheduler
    let heartbeat_delay = self.heartbeat_scheduler.take_delay();

    // 2. Race between event channel and heartbeat timer
    tokio::select! {
        event = self.event_rx.recv() => {
            // Handle incoming event
        }
        _ = tokio::time::sleep(heartbeat_delay) => {
            // Heartbeat timer expired
        }
    }
}
```

### Step-by-Step

1. **Get heartbeat delay** (`heartbeat_scheduler.take_delay()`)
   - Returns the duration until the next scheduled heartbeat
   - Default interval: 45 minutes
   - Can be modified by the agent via the `schedule_heartbeat` tool
   - Returns the delay and clears any pending heartbeat so it doesn't fire twice

2. **Wait for events** (`tokio::select!`)
   - Uses Tokio's select macro to race between two futures
   - Whichever completes first wins
   - The other future is cancelled

3. **Handle event types:**

   | Event | Action |
   |-------|--------|
   | `LoopEvent::InboxUpdate(paths)` | Log "Wake: inbox update with N files", transition to `Waking` with `WakeTrigger::Inbox(paths)` |
   | `LoopEvent::Heartbeat` | Log "Wake: heartbeat", transition to `Waking` with `WakeTrigger::Heartbeat` |
   | `LoopEvent::Shutdown` | Log "Shutdown requested", set `shutdown_requested = true`, remain Sleeping |
   | Channel closed (`None`) | Log "Event channel closed", set `shutdown_requested = true` |
   | Timer expires | Log "Wake: heartbeat timer", transition to `Waking` with `WakeTrigger::Heartbeat` |

### Exit Conditions

- **→ Waking:** Event received (inbox update or heartbeat) or heartbeat timer expired
- **→ (stays Sleeping):** Shutdown requested (loop will exit after checking `shutdown_requested && is_sleeping()`)

---

## Phase 2: Waking

**File:** `loop/mod.rs:303-430`

Prepares context for the model call. Behavior depends on whether this is a fresh start (`needs_context_reset = true`) or continuing an existing session.

### What Happens

```rust
async fn wake_phase(&mut self) {
    // 1. Extract the wake trigger from state
    // 2. Drain any queued messages that arrived before wake
    // 3. IF needs_context_reset:
    //      - Clear context and assemble fresh from workspace files
    //      - Load previous context from JSONL file (for restart recovery)
    //      - Persist queued messages to context file
    //      - Clear needs_context_reset flag
    //    ELSE:
    //      - Add queued messages to existing context
    //      - Add wake trigger to context
    // 4. Inject 80% context warning if needed
    // 5. Add any pending system notifications
    // 6. Load tool schemas
    // 7. Transition to Thinking
}
```

### Step-by-Step

#### Step 2.1: Extract Wake Trigger

```rust
let trigger = match std::mem::replace(&mut self.state, LoopState::Sleeping) {
    LoopState::Waking { trigger } => trigger,
    _ => {
        tracing::error!("Invalid state in wake_phase");
        return;
    }
};
```

Uses `std::mem::replace` to take ownership of the trigger while temporarily setting state to Sleeping. This pattern avoids borrowing issues. If somehow called in the wrong state, logs an error and returns early.

#### Step 2.2: Drain Queued Messages

```rust
let queued_messages = self.message_queue.drain();
if !queued_messages.is_empty() {
    tracing::info!("Processing {} queued messages", queued_messages.len());
}
```

Messages can arrive via the message queue while the agent is in non-sleeping phases. This drains all of them so they can be included in the context.

#### Step 2.3a: Fresh Context Build (First Wake or Post-Rotation)

**Condition:** `self.needs_context_reset == true`

```rust
tracing::info!("Building fresh context (first wake or post-rotation)");
self.context.clear();
self.context.assemble(&self.config.workspace, trigger, queued_messages.clone()).await;
```

**`ContextBuilder::assemble()` does:**

1. **Build system prompt** (`build_system_prompt`)
   - Reads workspace files in order: `AGENTS.md`, `IDENTITY.md`, `RULES.md`
   - Each file that exists is added to the prompt
   - Adds current timestamp: `Current time: {ISO 8601 timestamp}`
   - Joins all parts with `\n\n---\n\n` separators
   - If no files found, defaults to: "You are an AI assistant."

2. **Load continuity state** (`load_continuity_state`)
   - Attempts to read `thinking/current-state.md` from workspace
   - If exists, adds system message: "Continuing session. Last cycle you were:\n{content}"
   - Provides the agent with memory of what it was doing before rotation

3. **Add queued messages**
   - Each message formatted as: `[{channel}] {author.name}: {content}`
   - Added as user messages

4. **Add wake trigger**
   - Inbox: Lists all inbox file paths with new messages
   - Heartbeat: Adds `:heartbeat:` as user message

**Then load context from file:**

```rust
if let Some(ref file) = self.context_file {
    match file.load() {
        Ok(messages) => {
            for msg in messages {
                self.context.add_message(msg);
            }
            tracing::info!(message_count = count, "Loaded context from file");
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to load context file");
        }
    }
}
```

This loads the previous conversation from the JSONL file. On restart, this restores the full context that was being built before shutdown.

**Persist queued messages to file:**

```rust
for msg in queued_messages {
    let chat_msg = ChatMessage::user(format!("[{}] {}: {}", msg.channel, msg.author.name, msg.content));
    if let Some(ref file) = self.context_file {
        if let Err(e) = file.append(&chat_msg) {
            tracing::error!(error = %e, "Failed to append queued message to context file");
        }
    }
}
```

Messages were already added via `assemble()`, but we also persist them to the JSONL file for durability.

**Clear the reset flag:**

```rust
self.needs_context_reset = false;
```

#### Step 2.3b: Accumulating Context (Normal Operation)

**Condition:** `self.needs_context_reset == false`

```rust
tracing::debug!("Adding to existing context (accumulating)");

// Add queued messages
for msg in queued_messages {
    let chat_msg = ChatMessage::user(format!("[{}] {}: {}", msg.channel, msg.author.name, msg.content));
    self.context.add_message(chat_msg.clone());

    // Also persist to file
    if let Some(ref file) = self.context_file {
        if let Err(e) = file.append(&chat_msg) {
            tracing::error!(error = %e, "Failed to append queued message to context file");
        }
    }
}

// Add wake trigger
match &trigger {
    WakeTrigger::Inbox(paths) => {
        let file_list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        let chat_msg = ChatMessage::user(format!("New messages in inbox files:\n{}", file_list.join("\n")));
        self.context.add_message(chat_msg);
    }
    WakeTrigger::Heartbeat => {
        // Heartbeats are transient - not persisted to file
        self.context.add_message(ChatMessage::user(":heartbeat:"));
    }
}
```

Key differences from fresh build:
- **No clearing** - existing conversation preserved
- **No re-reading** system prompt files (they're already in context)
- **No reloading** from JSONL file (context is already in memory)
- Just appends new messages and triggers

#### Step 2.4: Inject 80% Context Warning

```rust
let context_percent = (self.last_prompt_tokens as f64 / self.config.context_limit as f64) * 100.0;
if context_percent >= 80.0 && context_percent < 90.0 {
    self.context.add_message(ChatMessage::system(format!(
        "WARNING: Context at {:.1}%. Consider summarizing and calling rotate_context soon.",
        context_percent
    )));
}
```

When context is between 80-90%, inject a system message warning the agent. This gives the agent a chance to manually rotate with a summary before automatic rotation kicks in at 90%.

#### Step 2.5: Add Pending Notifications

```rust
if !self.pending_notifications.is_empty() {
    let notifications = std::mem::take(&mut self.pending_notifications);
    let notification_text = format!(
        "SYSTEM NOTIFICATIONS:\n{}",
        notifications.iter().map(|n| format!("- {}", n)).collect::<Vec<_>>().join("\n")
    );
    self.context.add_message(ChatMessage::system(notification_text));
}
```

System notifications (like git conflicts) are accumulated in `pending_notifications` and surfaced to the agent on the next wake.

#### Step 2.6: Load Tool Schemas

```rust
let executor = self.tool_executor.read().await;
self.context.set_tools(executor.schemas());
```

Gets the current tool schemas from the executor. This allows tools to be dynamically added or removed.

#### Step 2.7: Transition to Thinking

```rust
self.state = LoopState::Thinking;
```

### Context Structure

**Fresh Context (after reset):**
```
┌─────────────────────────────────────────────────────────────────┐
│ 1. System Prompt (from AGENTS.md, IDENTITY.md, RULES.md)        │
│ 2. Continuity State (from thinking/current-state.md)           │
│ 3. Queued Messages (arrived before wake)                        │
│ 4. Wake Trigger (inbox notification or heartbeat)               │
│ 5. Loaded Context (from context.jsonl file)                     │
│ 6. 80% Warning (if applicable)                                  │
│ 7. System Notifications (git conflicts, etc.)                   │
└─────────────────────────────────────────────────────────────────┘
```

**Accumulated Context (normal operation):**
```
┌─────────────────────────────────────────────────────────────────┐
│ [Previous context preserved]                                     │
│ + New queued messages                                           │
│ + New wake trigger                                              │
│ + 80% Warning (if applicable)                                   │
│ + New notifications (if any)                                    │
└─────────────────────────────────────────────────────────────────┘
```

### Exit Condition

- **→ Thinking:** Always (unless error extracting trigger)

---

## Phase 3: Thinking

**File:** `loop/mod.rs:432-534`

Calls the model with assembled context and tools. Includes hard limit protection and auto-rotation triggering.

### What Happens

```rust
async fn think_phase(&mut self) {
    // 1. Check 95% hard limit - force rotation if exceeded
    // 2. Call the model API
    // 3. Update last_prompt_tokens from response
    // 4. Check 90% threshold - request auto-rotation if exceeded
    // 5. Add assistant response to context and file
    // 6. Decide next state based on tool calls
}
```

### Step-by-Step

#### Step 3.1: 95% Hard Limit Gate

```rust
let context_percent = self.context_status().percent();
if context_percent >= 95.0 {
    tracing::error!(
        percent = format!("{:.1}", context_percent),
        tokens = self.last_prompt_tokens,
        limit = self.config.context_limit,
        "Context at 95%+ - forcing immediate rotation, skipping model call"
    );
    self.context_rotation.request_auto();
    self.state = LoopState::Settling;
    return;
}
```

This is a safety mechanism that prevents context overflow. If context is already at 95%+:
- Log an error with current metrics
- Request automatic rotation
- Skip the model call entirely
- Transition directly to Settling to handle the rotation

This gate catches cases where:
- The 90% auto-rotation didn't trigger (bug)
- Rotation was requested but not yet processed
- Context grew unexpectedly

#### Step 3.2: Call Model API

```rust
let response = match self.model_client.complete(
    self.context.messages(),
    self.context.tools(),
).await {
    Ok(resp) => resp,
    Err(e) => {
        tracing::error!(error = %e, "Model call failed - transitioning to Settling");
        self.state = LoopState::Settling;
        return;
    }
};

tracing::info!(
    tokens_total = response.usage.total_tokens,
    tokens_prompt = response.usage.prompt_tokens,
    tokens_completion = response.usage.completion_tokens,
    tool_calls = response.tool_calls.len(),
    has_content = response.content.is_some(),
    "Model response received"
);
```

**`ModelClient::complete()` does:**

1. Build OpenAI-compatible request:
   ```json
   {
     "model": "model-name",
     "messages": [...],
     "tools": [{"type": "function", "function": {...}}]
   }
   ```

2. POST to `{model_url}/v1/chat/completions`

3. Parse response into `ModelResponse`:
   - `content: Option<String>` - Text response from the model
   - `tool_calls: Vec<ToolCallRequest>` - Requested tool executions
   - `usage: Usage` - Token counts (prompt_tokens, completion_tokens, total_tokens)

4. On error → log and transition to Settling

#### Step 3.3: Update Token Tracking

```rust
self.last_prompt_tokens = response.usage.prompt_tokens as u64;
```

This is the authoritative update of context usage. The `prompt_tokens` value from the API response tells us exactly how many tokens the current context consumes.

#### Step 3.4: Check 90% Auto-Rotation Threshold

```rust
let context_percent = (self.last_prompt_tokens as f64 / self.config.context_limit as f64) * 100.0;
if context_percent >= 90.0 {
    tracing::warn!(
        percent = format!("{:.1}", context_percent),
        "Context at 90%+ - triggering auto-rotation"
    );
    self.context_rotation.request_auto();
}
```

When context reaches 90%:
- Log a warning with the exact percentage
- Request automatic rotation via `context_rotation.request_auto()`
- The rotation will be processed in the Settling phase
- Note: This doesn't immediately stop execution - the current response is still processed

#### Step 3.5: Add Response to Context and File

```rust
// Add to in-memory context
self.context.add_assistant_response(
    response.content.clone(),
    if response.tool_calls.is_empty() { None } else { Some(response.tool_calls.clone()) },
);

// Persist to JSONL file
if let Some(ref file) = self.context_file {
    let msg = ChatMessage::assistant(
        response.content.clone(),
        if response.tool_calls.is_empty() { None } else { Some(response.tool_calls.clone()) },
    );
    if let Err(e) = file.append(&msg) {
        tracing::error!(error = %e, "Failed to append assistant message to context file");
    }
}
```

The assistant's response is:
1. Added to the in-memory context for subsequent model calls
2. Persisted to the JSONL file for durability and restart recovery

#### Step 3.6: Decide Next State

```rust
if response.tool_calls.is_empty() {
    // No tool calls - conversation turn complete
    if let Some(content) = &response.content {
        tracing::info!(
            content_len = content.len(),
            content_preview = %content.chars().take(300).collect::<String>(),
            "Assistant response (no tool calls) - transitioning to Settling"
        );
    } else {
        tracing::info!("No content and no tool calls - transitioning to Settling");
    }
    self.state = LoopState::Settling;
} else {
    // Has tool calls - need to execute them
    tracing::info!(
        tool_call_count = response.tool_calls.len(),
        tools = ?response.tool_calls.iter().map(|t| &t.function.name).collect::<Vec<_>>(),
        "Transitioning to Acting phase"
    );
    self.state = LoopState::Acting { pending: response.tool_calls };
}
```

| Condition | Next State |
|-----------|------------|
| `tool_calls.is_empty()` | → Settling |
| `tool_calls` present | → Acting (with pending calls) |

### Exit Conditions

- **→ Settling:** 95% hard limit hit (skip model call)
- **→ Settling:** Model call failed (error handling)
- **→ Settling:** No tool calls (conversation turn complete)
- **→ Acting:** Tool calls present (need to execute)

---

## Phase 4: Acting

**File:** `loop/mod.rs:536-632`

Executes tool calls requested by the model.

### What Happens

```rust
async fn act_phase(&mut self) {
    // 1. Extract pending tool calls from state
    // 2. Execute each tool call sequentially
    // 3. Drain messages that arrived during execution
    // 4. Persist tool results to context file
    // 5. Add results to in-memory context
    // 6. Check if rotation requested, decide next state
}
```

### Step-by-Step

#### Step 4.1: Extract Tool Calls

```rust
let tool_calls = match std::mem::replace(&mut self.state, LoopState::Thinking) {
    LoopState::Acting { pending } => pending,
    _ => {
        tracing::error!("Invalid state in act_phase - expected Acting");
        self.state = LoopState::Settling;
        return;
    }
};

tracing::info!(
    tool_call_count = tool_calls.len(),
    tools = ?tool_calls.iter().map(|t| &t.function.name).collect::<Vec<_>>(),
    "Act phase: executing tool calls"
);
```

Takes ownership of the pending tool calls while temporarily setting state to Thinking. Logs the tools being executed for debugging.

#### Step 4.2: Execute Each Tool

```rust
let mut results = Vec::new();
{
    let mut executor = self.tool_executor.write().await;
    for (i, tc) in tool_calls.iter().enumerate() {
        tracing::info!(
            index = i,
            tool = %tc.function.name,
            call_id = %tc.id,
            args_raw = %tc.function.arguments,
            "Processing tool call"
        );

        // Parse JSON arguments
        let arguments = match serde_json::from_str(&tc.function.arguments) {
            Ok(args) => args,
            Err(e) => {
                tracing::warn!(
                    tool = %tc.function.name,
                    error = %e,
                    args_raw = %tc.function.arguments,
                    "Invalid JSON arguments - using empty object"
                );
                serde_json::Value::Object(serde_json::Map::new())
            }
        };

        let call = ToolCall {
            id: tc.id.clone(),
            name: tc.function.name.clone(),
            arguments,
        };

        let result = executor.execute(&call);
        let success = result.result.is_ok();
        tracing::info!(
            tool = %tc.function.name,
            call_id = %tc.id,
            success = success,
            "Tool execution complete"
        );
        results.push(result);
    }
}
```

For each tool call:
1. Log the tool name, call ID, and raw arguments
2. Parse the JSON arguments string into a `serde_json::Value`
3. If parsing fails, use an empty object and log a warning
4. Create a `ToolCall` struct with id, name, and parsed arguments
5. Execute via `executor.execute(&call)`
6. Log whether execution succeeded
7. Collect the result

**`ToolExecutor::execute()` does:**

1. Look up the tool in the registry by name
2. Call `tool.execute(arguments)` - this runs the tool's implementation
3. Return `ToolCallResponse { tool_call_id, result }` where result is `Ok(ToolResult)` or `Err(String)`

#### Step 4.3: Drain Messages During Execution

```rust
let incoming_messages = self.message_queue.drain();
if !incoming_messages.is_empty() {
    tracing::info!("{} messages arrived during tool execution", incoming_messages.len());
}
```

While tools execute (especially slow ones like shell commands), new messages may arrive. These are captured from the queue so they can be included in the context for the next model call.

#### Step 4.4: Persist Tool Results to File

```rust
for result in &results {
    let content = match &result.result {
        Ok(r) => r.output.clone(),
        Err(e) => format!("Error: {}", e),
    };

    if let Some(ref file) = self.context_file {
        let chat_msg = ChatMessage::tool(&result.tool_call_id, content);
        if let Err(e) = file.append(&chat_msg) {
            tracing::error!(error = %e, "Failed to append tool result to context file");
        }
    }
}
```

Each tool result is persisted to the JSONL file for durability:
- Success: The raw output string
- Error: Prefixed with "Error: "

#### Step 4.5: Add Results to Context

```rust
self.context.add_tool_results(results, incoming_messages);
```

**`ContextBuilder::add_tool_results()` does:**

1. For each tool result, add as `ChatMessage::tool(tool_call_id, content)`
   - Success: raw output
   - Error: "Error: {message}"

2. If incoming messages exist, add a system message:
   ```
   Messages received during tool execution:
   - [channel] author: content
   - [channel] author: content
   ```

#### Step 4.6: Decide Next State

```rust
if self.context_rotation.is_requested() {
    self.state = LoopState::Settling;
} else {
    self.state = LoopState::Thinking;
}
```

Simple check:
- If rotation was requested (either manually via `rotate_context` tool or automatically at 90%) → go to Settling to process it
- Otherwise → go back to Thinking for another model call

### Exit Conditions

- **→ Settling:** Context rotation was requested
- **→ Thinking:** Continue conversation (model sees tool results)

---

## Phase 5: Settling

**File:** `loop/mod.rs:693-758`

Handles housekeeping tasks: context rotation, message persistence, git commits.

### What Happens

```rust
async fn settle_phase(&mut self) {
    // 1. Handle context rotation if requested
    // 2. Persist conversation messages to database
    // 3. Git commit if workspace changed
    // 4. Transition to Sleeping
}
```

### Step-by-Step

#### Step 5.1: Handle Context Rotation

```rust
if let Some(summary_opt) = self.context_rotation.take_request() {
    tracing::info!(has_summary = summary_opt.is_some(), "Processing context rotation");

    // Archive current context to database
    if let Err(e) = self.archive_current_context(summary_opt.as_deref()) {
        tracing::error!(error = %e, "Failed to archive context");
    } else {
        // Create new context
        let result = if let Some(ref s) = summary_opt {
            self.create_context_with_summary(s)
        } else {
            tracing::warn!("Auto-rotation with no summary - continuity lost");
            self.create_fresh_context()
        };

        if let Err(e) = result {
            tracing::error!(error = %e, "Failed to create new context");
        }

        // Flag for context rebuild on next wake
        self.needs_context_reset = true;
        self.last_prompt_tokens = 0;
    }
}
```

**`context_rotation.take_request()`** returns:
- `None` - No rotation requested
- `Some(Some(summary))` - Manual rotation with summary (from `rotate_context` tool)
- `Some(None)` - Automatic rotation without summary (from 90% threshold)

**`archive_current_context(summary)` does:**

1. Read the raw bytes from `context.jsonl`
2. Generate an archive timestamp snowflake
3. Store to SQLite `contexts` table:
   - `id`: Original context snowflake
   - `archived_at`: Archive timestamp
   - `token_count`: Final token count
   - `summary`: User's summary (or NULL for auto)
   - `blob`: Raw JSONL bytes

**Creating new context:**

- **With summary** (`create_context_with_summary`): Creates new JSONL file with the summary as the first system message. This provides continuity.
- **Without summary** (`create_fresh_context`): Creates empty JSONL file. Logs a warning because continuity is lost.

**After rotation:**
- Set `needs_context_reset = true` - next wake will rebuild from scratch
- Set `last_prompt_tokens = 0` - reset the token counter

#### Step 5.2: Persist Messages to Database

```rust
self.persist_messages();
```

**`persist_messages()` does:**

1. Lock database
2. For each message in context:
   - Skip system messages (they're context-specific, not conversation)
   - Convert `ChatMessage` → `Message` (database format)
   - Generate snowflake ID
   - Set `session_id = "main"`
   - Serialize `tool_calls` to JSON if present
   - Insert into `messages` table

| Role | Persisted? | Notes |
|------|------------|-------|
| system | No | Context-specific (prompts, warnings) |
| user | Yes | Incoming messages |
| assistant | Yes | Model responses, tool_calls as JSON |
| tool | Yes | Tool execution results |

#### Step 5.3: Git Commit

```rust
if self.git.is_git_repo() {
    match self.git.commit_if_changed() {
        GitCommitResult::NoChanges => {
            tracing::debug!("No workspace changes to commit");
        }
        GitCommitResult::Committed { files, commit_hash } => {
            tracing::info!("Committed {} file(s) as {} ({})", files.len(), commit_hash, files.join(", "));
        }
        GitCommitResult::Conflicts { conflicting_files } => {
            tracing::warn!("Git conflicts detected in {} file(s)", conflicting_files.len());
            self.pending_notifications.push(format!(
                "Git conflict detected: The following files have merge conflicts: {}",
                conflicting_files.join(", ")
            ));
        }
        GitCommitResult::Error(e) => {
            tracing::warn!("Git commit failed: {}", e);
        }
    }
}
```

**`GitOps::commit_if_changed()` does:**

1. Check `git status` for changes
2. If changes exist:
   - `git add -A` to stage all changes
   - `git commit -m "agent: auto-commit"` to commit
3. Return result:

| Result | Action |
|--------|--------|
| `NoChanges` | Log debug message |
| `Committed { files, hash }` | Log info with files and commit hash |
| `Conflicts { files }` | Log warning, add to `pending_notifications` for agent |
| `Error(e)` | Log warning |

Git conflicts are not fatal - they're surfaced to the agent on the next wake as a system notification.

#### Step 5.4: Transition to Sleeping

```rust
self.state = LoopState::Sleeping;
```

Always goes to Sleeping. Any new messages will trigger an `InboxUpdate` event to wake the agent.

### Exit Condition

- **→ Sleeping:** Always

---

## Context Rotation System

### Threshold Behavior

| Threshold | Location | Action |
|-----------|----------|--------|
| 80% | `wake_phase` | Warning injected as system message |
| 90% | `think_phase` | Auto-rotation requested after model call |
| 95% | `think_phase` | Hard stop, skip model call, force rotation |

### Rotation Types

**Manual Rotation** (via `rotate_context` tool):
- Agent provides a summary of current work
- Summary becomes first message in new context
- Preserves continuity and intent

**Automatic Rotation** (at 90%):
- No summary provided
- New context starts empty (except system prompt)
- Logs warning about continuity loss
- Recovery relies on:
  - `thinking/current-state.md` file (if agent maintains it)
  - Database message history (last 50 messages loaded on fresh context)

### Rotation Flow

```
                                    ┌────────────────┐
                                    │  think_phase   │
                                    │  checks 95%    │
                                    └───────┬────────┘
                                            │
                        ┌───────────────────┼───────────────────┐
                        │                   │                   │
                        ▼                   ▼                   ▼
                    < 90%               90-95%              >= 95%
                        │                   │                   │
                        ▼                   ▼                   ▼
                   continue           request_auto()      request_auto()
                        │                   │             + skip model
                        │                   │             + → Settling
                        ▼                   ▼
                   act_phase           act_phase
                        │                   │
                        ▼                   ▼
              is_requested()?      is_requested()? ─── yes ───▶ Settling
                        │                   │
                       no                   │
                        │                   │
                        ▼                   ▼
                   Thinking             Settling
                                            │
                                            ▼
                                    take_request()
                                            │
                                            ▼
                                   archive_current_context()
                                            │
                                            ▼
                                   create_fresh_context()
                                   or create_context_with_summary()
                                            │
                                            ▼
                                   needs_context_reset = true
                                   last_prompt_tokens = 0
                                            │
                                            ▼
                                        Sleeping
```

---

## Context Persistence

### JSONL File

**Location:** `{workspace}/context.jsonl`

**Format:** One JSON object per line, each representing a `ChatMessage`:

```jsonl
{"role":"system","content":"You are an AI assistant..."}
{"role":"user","content":"[general] alice: Hello!"}
{"role":"assistant","content":"Hello! How can I help?"}
{"role":"user","content":":heartbeat:"}
{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{\"path\":\"file.txt\"}"}}]}
{"role":"tool","content":"File contents here...","tool_call_id":"call_1"}
```

### Database Schema

**contexts table:**
```sql
CREATE TABLE contexts (
    id BLOB PRIMARY KEY,           -- Snowflake ID (16 bytes)
    created_at INTEGER NOT NULL,   -- Unix timestamp
    archived_at BLOB,              -- Archive timestamp snowflake (NULL if active)
    token_count INTEGER,           -- Final token count at archive
    summary TEXT,                  -- User summary (NULL for auto-rotation)
    blob BLOB                      -- Raw JSONL bytes (NULL if active)
);
```

**messages table:**
```sql
CREATE TABLE messages (
    id BLOB PRIMARY KEY,           -- Snowflake ID (16 bytes)
    session_id TEXT,               -- "main" for primary session
    role TEXT,                     -- "user", "assistant", "tool"
    content TEXT,                  -- Message content
    tool_calls TEXT,               -- JSON array of tool calls
    tool_call_id TEXT,             -- For tool role messages
    name TEXT,                     -- Optional name field
    created_at INTEGER,            -- Unix timestamp
    metadata TEXT                  -- JSON metadata
);
```

---

## Configuration

**File:** `loop/mod.rs:26-53`

```rust
pub struct LoopConfig {
    pub workspace: PathBuf,              // Workspace for context files (default: ".")
    pub default_heartbeat_minutes: u32,  // Heartbeat interval (default: 45)
    pub context_limit: u64,              // Token limit (default: 65536)
    pub model_timeout: Duration,         // Model call timeout (default: 120s)
    pub max_tool_calls_per_generation: usize,  // Safety limit (default: 50)
    pub history_message_limit: usize,    // Messages to load on fresh context (default: 50)
}
```

---

## Key Types

### LoopState

```rust
pub enum LoopState {
    Sleeping,
    Waking { trigger: WakeTrigger },
    Thinking,
    Acting { pending: Vec<ToolCallRequest> },
    Settling,
}
```

### WakeTrigger

```rust
pub enum WakeTrigger {
    Inbox(Vec<PathBuf>),  // Inbox files with new messages
    Heartbeat,            // Scheduled heartbeat
}
```

### LoopEvent

```rust
pub enum LoopEvent {
    InboxUpdate(Vec<PathBuf>),  // New messages in inbox files
    Heartbeat,                   // Heartbeat timer fired
    Shutdown,                    // Graceful shutdown requested
}
```

### ToolCallRequest

```rust
pub struct ToolCallRequest {
    pub id: String,
    pub r#type: String,  // Always "function"
    pub function: FunctionCall,
}

pub struct FunctionCall {
    pub name: String,
    pub arguments: String,  // JSON string
}
```

### ToolCallResponse

```rust
pub struct ToolCallResponse {
    pub tool_call_id: String,
    pub result: Result<ToolResult, String>,
}

pub struct ToolResult {
    pub output: String,
}
```

---

## Error Handling

| Error | Location | Handling |
|-------|----------|----------|
| 95% context limit | think_phase | Force rotation, skip model call |
| Model call failed | think_phase | Log error, → Settling |
| Database lock failed | persist_messages | Log error, continue |
| Message persist failed | persist_messages | Log warning, continue |
| Git commit failed | settle_phase | Log warning, continue |
| Git conflicts | settle_phase | Log warning, add to pending_notifications |
| Invalid tool arguments | act_phase | Use empty object, log warning |
| Unknown tool | ToolExecutor | Return error result |
| Context file append failed | multiple | Log error, continue |
| Context archive failed | settle_phase | Log error, skip new context creation |

---

## Data Flow Diagram

```
                    ┌─────────────────┐
                    │   HTTP API      │
                    │  /incoming      │
                    └────────┬────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         INBOX FILES                                      │
│   workspace/inbox/{adapter}/{hierarchy}/{channel}.txt                   │
│   - One message per line: [status] timestamp msgId <name:id> content    │
│   - Agent reads with tools, marks [x] when processed                    │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         EVENT CHANNEL                                    │
│   mpsc::Receiver<LoopEvent>                                             │
│   - LoopEvent::InboxUpdate(Vec<PathBuf>)  ← files with new messages     │
│   - LoopEvent::Heartbeat                                                │
│   - LoopEvent::Shutdown                                                 │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         MESSAGE QUEUE                                    │
│   Arc<MessageQueue>                                                      │
│   - Buffers messages during non-sleeping phases                         │
│   - drain() returns all and clears                                      │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         CONTEXT BUILDER                                  │
│   - System prompt (AGENTS.md, IDENTITY.md, RULES.md)                    │
│   - Continuity state (thinking/current-state.md)                        │
│   - Current messages and tool results                                   │
│   - Tool schemas                                                        │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         MODEL CLIENT                                     │
│   POST {model_url}/v1/chat/completions                                  │
│   - OpenAI-compatible API                                               │
│   - Returns: content, tool_calls, usage                                 │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         TOOL EXECUTOR                                    │
│   - Registry of all tools                                               │
│   - Executes tool calls                                                 │
│   - Returns ToolCallResponse with result                                │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         CONTEXT FILE                                     │
│   workspace/context.jsonl                                               │
│   - JSONL format, one message per line                                  │
│   - Appended after each model call and tool execution                   │
│   - Loaded on restart to resume context                                 │
│   - Archived to database on rotation                                    │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         DATABASE                                         │
│   SQLite: messages + contexts tables                                    │
│   - messages: Persists user/assistant/tool messages                     │
│   - contexts: Archives rotated contexts with blob storage               │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         GIT OPS                                          │
│   - Auto-commits workspace changes                                      │
│   - Detects conflicts                                                   │
│   - Surfaces conflicts as notifications                                 │
└─────────────────────────────────────────────────────────────────────────┘
```

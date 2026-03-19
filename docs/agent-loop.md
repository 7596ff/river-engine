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
| `Settling` | Persisting state, git commit | `Sleeping` or `Waking` |

---

## Phase 1: Sleeping

**File:** `loop/mod.rs:156-191`

The agent is idle, waiting for something to happen.

### What Happens

```rust
async fn sleep_phase(&mut self) {
    let heartbeat_delay = self.heartbeat_scheduler.take_delay();

    tokio::select! {
        event = self.event_rx.recv() => { ... }
        _ = tokio::time::sleep(heartbeat_delay) => { ... }
    }
}
```

### Step-by-Step

1. **Get heartbeat delay** (`heartbeat_scheduler.take_delay()`)
   - Returns duration until next scheduled heartbeat
   - Default: 45 minutes
   - Can be modified by agent via `schedule_heartbeat` tool

2. **Wait for events** (`tokio::select!`)
   - Races between event channel and heartbeat timer
   - First one to complete wins

3. **Handle event types:**

   | Event | Action |
   |-------|--------|
   | `LoopEvent::Message(msg)` | Transition to `Waking` with `WakeTrigger::Message` |
   | `LoopEvent::Heartbeat` | Transition to `Waking` with `WakeTrigger::Heartbeat` |
   | `LoopEvent::Shutdown` | Set `shutdown_requested = true` |
   | Channel closed | Set `shutdown_requested = true` |
   | Timer expires | Transition to `Waking` with `WakeTrigger::Heartbeat` |

### Exit Conditions

- **→ Waking:** Message received or heartbeat timer expired
- **→ (stays Sleeping):** Shutdown requested (loop will exit after this iteration)

---

## Phase 2: Waking

**File:** `loop/mod.rs:196-280`

Prepares context for the model call. Behavior depends on whether this is a fresh start or continuing an existing session.

### What Happens

```rust
async fn wake_phase(&mut self) {
    // 1. Extract trigger
    // 2. Drain queued messages
    // 3. IF needs_context_reset:
    //      - Clear and assemble fresh context
    //      - Load conversation history from DB
    //      - Reset context tracking
    //    ELSE:
    //      - Add new messages to existing context (accumulate)
    // 4. Add notifications
    // 5. Set available tools
}
```

### Key Concept: Context Accumulation

The agent maintains context across multiple wake-sleep cycles. Context is only reset when:
- **First startup** (`needs_context_reset = true` initially)
- **Manual rotation** (agent calls `rotate_context` tool)
- **Automatic rotation** (context reaches 90% of limit)

Otherwise, each wake just **adds** to the existing context, preserving the full conversation.

### Step-by-Step

#### Step 2.1: Extract Wake Trigger
```rust
let trigger = match std::mem::replace(&mut self.state, LoopState::Sleeping) {
    LoopState::Waking { trigger } => trigger,
    _ => return,
};
```

#### Step 2.2: Drain Queued Messages
```rust
let queued_messages = self.message_queue.drain();
```

#### Step 2.3a: Fresh Context (First Wake or Post-Rotation)

**Condition:** `self.needs_context_reset == true`

```rust
self.context.clear();
self.context.assemble(&self.config.workspace, trigger, queued_messages).await;
self.load_conversation_history();
executor.reset_context();
self.needs_context_reset = false;
```

**`ContextBuilder::assemble()` does:**

1. **Build system prompt** (`build_system_prompt`)
   - Reads workspace files: `AGENTS.md`, `IDENTITY.md`, `RULES.md`
   - Adds current timestamp
   - Joins with `---` separators

2. **Load continuity state** (`load_continuity_state`)
   - Reads `thinking/current-state.md` from workspace
   - If exists, adds: "Continuing session. Last cycle you were: {content}"

3. **Add queued messages** - Each formatted as: `[channel] author: content`

4. **Add wake trigger** - Message or heartbeat notification

**`load_conversation_history()` does:**

1. Query database: `db.get_session_messages(PRIMARY_SESSION_ID, 50)`
2. Add history markers and convert DB messages to chat format
3. Provides continuity from before the rotation

#### Step 2.3b: Accumulating Context (Normal Operation)

**Condition:** `self.needs_context_reset == false`

```rust
// Just add new messages to existing context
for msg in queued_messages {
    self.context.add_message(ChatMessage::user(format!(...)));
}
// Add wake trigger
match &trigger {
    WakeTrigger::Message(msg) => self.context.add_message(ChatMessage::user(...)),
    WakeTrigger::Heartbeat => self.context.add_message(ChatMessage::system(...)),
}
```

- **No clearing** - existing conversation preserved
- **No re-reading** system prompt files
- **No reloading** history from DB (it's already in context)
- Just appends new messages/triggers

#### Step 2.4: Add Pending Notifications
```rust
if !self.pending_notifications.is_empty() {
    self.context.add_message(ChatMessage::system(notification_text));
}
```

#### Step 2.5: Load Tool Schemas
```rust
let executor = self.tool_executor.read().await;
self.context.set_tools(executor.schemas());
```

### Context Structure

**Fresh Context (after reset):**
```
┌─────────────────────────────────────────────────────────────────┐
│ 1. System Prompt (from AGENTS.md, IDENTITY.md, RULES.md)        │
│ 2. Continuity State (from thinking/current-state.md)           │
│ 3. Queued Messages (arrived before wake)                        │
│ 4. Wake Trigger (the message/heartbeat that woke us)            │
│ 5. Conversation History (last 50 messages from DB)             │
│ 6. System Notifications (git conflicts, etc.)                   │
└─────────────────────────────────────────────────────────────────┘
```

**Accumulated Context (normal operation):**
```
┌─────────────────────────────────────────────────────────────────┐
│ [Previous context preserved]                                     │
│ + New queued messages                                           │
│ + New wake trigger                                              │
│ + New notifications (if any)                                    │
└─────────────────────────────────────────────────────────────────┘
```

### Exit Condition

- **→ Thinking:** Always (unless error extracting trigger)

---

## Phase 3: Thinking

**File:** `loop/mod.rs:241-320`

Calls the model with assembled context and tools.

### What Happens

```rust
async fn think_phase(&mut self) {
    // 1. Call model
    // 2. Add response to context
    // 3. Update context tracking
    // 4. Decide next state
}
```

### Step-by-Step

#### Step 3.1: Call Model
```rust
let response = self.model_client.complete(
    self.context.messages(),
    self.context.tools(),
).await;
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
   - `content: Option<String>` - Text response
   - `tool_calls: Vec<ToolCallRequest>` - Requested tool executions
   - `usage: Usage` - Token counts

4. On error → transition to Settling

#### Step 3.2: Add Response to Context
```rust
self.context.add_assistant_response(
    response.content.clone(),
    if response.tool_calls.is_empty() { None } else { Some(response.tool_calls.clone()) },
);
```
- Adds assistant message with content and/or tool_calls
- Maintains conversation for next model call

#### Step 3.3: Update Context Tracking
```rust
let mut executor = self.tool_executor.write().await;
executor.add_context(response.usage.total_tokens as u64);
```
- Tracks cumulative token usage
- Used for automatic context rotation at 90%

#### Step 3.4: Decide Next State

| Condition | Next State |
|-----------|------------|
| `tool_calls.is_empty()` | → Settling |
| `tool_calls` present | → Acting |

### Exit Conditions

- **→ Settling:** No tool calls (conversation turn complete)
- **→ Acting:** Tool calls present (need to execute)
- **→ Settling:** Model call failed (error handling)

---

## Phase 4: Acting

**File:** `loop/mod.rs:322-426`

Executes tool calls requested by the model.

### What Happens

```rust
async fn act_phase(&mut self) {
    // 1. Extract pending tool calls
    // 2. Execute each tool
    // 3. Drain messages arrived during execution
    // 4. Add results to context
    // 5. Check for context rotation
}
```

### Step-by-Step

#### Step 4.1: Extract Tool Calls
```rust
let tool_calls = match std::mem::replace(&mut self.state, LoopState::Thinking) {
    LoopState::Acting { pending } => pending,
    _ => return,
};
```
- Takes ownership of pending tool calls
- Temporarily sets state to Thinking

#### Step 4.2: Execute Each Tool
```rust
let mut executor = self.tool_executor.write().await;
for tc in tool_calls.iter() {
    let arguments = serde_json::from_str(&tc.function.arguments)?;
    let call = ToolCall { id, name, arguments };
    let result = executor.execute(&call);
    results.push(result);
}
```

**`ToolExecutor::execute()` does:**

1. Look up tool in registry by name
2. Call `tool.execute(arguments)`
3. Track output size for context estimation
4. Return `ToolCallResponse { tool_call_id, result, context_status }`

**Tool execution patterns:**

Most tools use this pattern to run async code from sync context:
```rust
tokio::task::block_in_place(|| {
    tokio::runtime::Handle::current().block_on(async { ... })
})
```

#### Step 4.3: Drain Messages During Execution
```rust
let incoming_messages = self.message_queue.drain();
```
- Messages may arrive while tools execute (especially slow ones)
- These are captured and included in context

#### Step 4.4: Add Results to Context
```rust
self.context.add_tool_results(results, incoming_messages, context_status);
```

**`add_tool_results()` does:**

1. Add each tool result as `ChatMessage::tool(tool_call_id, content)`
   - Success: raw output
   - Error: "Error: {message}"

2. Add context status: "Context: {used}/{limit} ({percent}%)"

3. Add incoming messages (if any):
   ```
   Messages received during tool execution:
   - [channel] author: content
   ```

#### Step 4.5: Check Context Rotation
```rust
if let Some(reason) = self.context_rotation.take_request() {
    // Manual rotation requested via rotate_context tool
    self.state = LoopState::Settling;
} else if context_status.is_near_limit() {
    // Automatic rotation at 90%
    self.state = LoopState::Settling;
} else {
    // Continue thinking
    self.state = LoopState::Thinking;
}
```

### Exit Conditions

- **→ Settling:** Context rotation requested (manual or automatic)
- **→ Thinking:** Continue conversation (model sees tool results)

---

## Phase 5: Settling

**File:** `loop/mod.rs:556-606`

Persists state and prepares for next cycle.

### What Happens

```rust
async fn settle_phase(&mut self) {
    // 1. Persist messages to database
    // 2. Git commit if changes
    // 3. Check for immediate wake
}
```

### Step-by-Step

#### Step 5.1: Persist Messages
```rust
self.persist_messages();
```

**`persist_messages()` does:**

1. Lock database
2. For each message in context (skip system messages):
   - Convert `ChatMessage` → `Message` (DB format)
   - Generate snowflake ID
   - Set `session_id = "main"`
   - Insert into database

| Role | Persisted? |
|------|------------|
| system | No (context-specific) |
| user | Yes |
| assistant | Yes (with tool_calls JSON) |
| tool | Yes |

#### Step 5.2: Git Commit
```rust
if self.git.is_git_repo() {
    match self.git.commit_if_changed() { ... }
}
```

**`GitOps::commit_if_changed()` does:**

1. Check `git status` for changes
2. If changes exist:
   - `git add -A`
   - `git commit -m "agent: auto-commit"`
3. Return result:

| Result | Action |
|--------|--------|
| `NoChanges` | Log debug, continue |
| `Committed { files, hash }` | Log info |
| `Conflicts { files }` | Log warn, add to `pending_notifications` |
| `Error(e)` | Log warn |

#### Step 5.3: Check for Immediate Wake
```rust
let messages = self.message_queue.drain();
if let Some(msg) = messages.into_iter().next() {
    self.state = LoopState::Waking { trigger: WakeTrigger::Message(msg) };
} else {
    self.state = LoopState::Sleeping;
}
```
- If messages arrived during settle → immediate wake
- Otherwise → go to sleep

### Exit Conditions

- **→ Waking:** Message arrived during settling
- **→ Sleeping:** No pending messages

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
│                         EVENT CHANNEL                                    │
│   mpsc::Receiver<LoopEvent>                                             │
│   - LoopEvent::Message(IncomingMessage)                                 │
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
│   - Conversation history (from database)                                │
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
│   - Tracks context usage                                                │
└─────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         DATABASE                                         │
│   SQLite: messages table                                                │
│   - Persists user/assistant/tool messages                               │
│   - Loaded on wake for conversation history                             │
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

---

## Configuration

**File:** `loop/mod.rs:23-51`

```rust
pub struct LoopConfig {
    pub workspace: PathBuf,              // Workspace for context files
    pub default_heartbeat_minutes: u32,  // Default: 45
    pub context_limit: u64,              // Default: 65536 tokens
    pub model_timeout: Duration,         // Default: 120 seconds
    pub max_tool_calls_per_generation: usize,  // Default: 50
    pub history_message_limit: usize,    // Default: 50 messages
}
```

---

## Key Components

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
    Message(IncomingMessage),
    Heartbeat,
}
```

### LoopEvent
```rust
pub enum LoopEvent {
    Message(IncomingMessage),
    Heartbeat,
    Shutdown,
}
```

---

## Automatic Context Rotation

When context usage reaches 90% of the limit:

1. **Detection:** In act_phase, after tool execution
2. **Action:** Set `needs_context_reset = true`, transition to Settling
3. **On next wake:** Context is cleared and rebuilt from scratch
4. **Recovery:** Agent receives:
   - Fresh system prompt
   - Last 50 messages from database history
   - Continuity state file (`thinking/current-state.md`)
5. **Penalty:** Loss of full in-context conversation, but history provides continuity

### Context Accumulation Flow

```
First Wake (needs_context_reset=true)
    │
    ▼
Build Fresh Context ──────────────────────────────────────────────┐
    │                                                              │
    ▼                                                              │
Thinking → Acting → Settling ──► Sleeping                         │
    │         │                      │                             │
    │         │                      ▼                             │
    │         │              Wake (accumulate)                     │
    │         │                      │                             │
    │         ▼                      ▼                             │
    │    Context < 90%?         Add to existing context            │
    │         │                      │                             │
    │        YES ────────────────────┘                             │
    │         │                                                    │
    │        NO (≥90%)                                             │
    │         │                                                    │
    │         ▼                                                    │
    │    needs_context_reset = true                                │
    │         │                                                    │
    │         ▼                                                    │
    └─────► Settling ──► Sleeping ──► Wake ────────────────────────┘
                                        (rebuild fresh context)
```

---

## Message Persistence

### What Gets Saved

| Message Type | Saved? | Notes |
|--------------|--------|-------|
| System | No | Context-specific (prompts, notifications) |
| User | Yes | Incoming messages |
| Assistant | Yes | Model responses, tool_calls as JSON |
| Tool | Yes | Tool execution results |

### Schema

```sql
CREATE TABLE messages (
    id BLOB PRIMARY KEY,      -- Snowflake ID (16 bytes)
    session_id TEXT,          -- "main" for primary session
    role TEXT,                -- "user", "assistant", "tool"
    content TEXT,             -- Message content
    tool_calls TEXT,          -- JSON array of tool calls
    tool_call_id TEXT,        -- For tool role messages
    name TEXT,                -- Optional name field
    created_at INTEGER,       -- Unix timestamp
    metadata TEXT             -- JSON metadata
);
```

---

## Error Handling

| Error | Location | Handling |
|-------|----------|----------|
| Model call failed | think_phase | → Settling |
| Database lock failed | persist_messages | Log error, continue |
| Message persist failed | persist_messages | Log warning, continue |
| Git commit failed | settle_phase | Log warning, continue |
| Git conflicts | settle_phase | Add to pending_notifications |
| Invalid tool arguments | act_phase | Use empty object, log warning |
| Unknown tool | ToolExecutor | Return error result |

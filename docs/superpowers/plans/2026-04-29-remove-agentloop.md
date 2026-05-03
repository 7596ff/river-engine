# Remove AgentLoop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the deprecated `loop/` module, relocate its shared types to `model/` and `queue.rs`, and remove dead plumbing.

**Architecture:** Extract `ChatMessage`, `ToolCallRequest`, `FunctionCall` to `model/types.rs`. Move `ModelClient` and related types to `model/client.rs`. Move `MessageQueue` to top-level `queue.rs`. Delete the `loop/` directory. Update all imports. Remove dead `LoopEvent`/`loop_tx` plumbing from `AppState`, `server.rs`, and `api/routes.rs`.

**Tech Stack:** Rust, tokio, reqwest, serde, axum

**Baseline:** 551 tests passing. `cargo test` in workspace root. The `loop/` module is `#[deprecated]` and generates 159 warnings.

---

### Task 1: Create `model/types.rs` — shared message types

**Files:**
- Create: `crates/river-gateway/src/model/types.rs`
- Create: `crates/river-gateway/src/model/mod.rs`

- [ ] **Step 1: Create `model/types.rs`**

Extract `ChatMessage` from `loop/context.rs` (lines 11-64) and `ToolCallRequest`/`FunctionCall` from `loop/state.rs` (lines 8-19). These types have no dependencies on anything in `loop/`:

```rust
//! Shared types for model interaction

use serde::{Deserialize, Serialize};

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
```

- [ ] **Step 2: Create `model/mod.rs`**

```rust
//! Model interaction types and client

pub mod types;
pub mod client;

pub use types::{ChatMessage, ToolCallRequest, FunctionCall};
pub use client::{ModelClient, ModelResponse, Usage, Provider};
```

Note: `model/client.rs` does not exist yet — `mod.rs` will fail to compile until Task 2. That's fine; we don't build between tasks 1 and 2.

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/model/
git commit -m "add: model/types.rs and model/mod.rs — new home for shared types"
```

---

### Task 2: Create `model/client.rs` — move ModelClient

**Files:**
- Create: `crates/river-gateway/src/model/client.rs` (from `loop/model.rs`)

- [ ] **Step 1: Copy `loop/model.rs` to `model/client.rs`**

Copy the entire contents of `crates/river-gateway/src/loop/model.rs` to `crates/river-gateway/src/model/client.rs`.

- [ ] **Step 2: Fix internal imports**

In the new `model/client.rs`, replace these two imports at the top:

```rust
// OLD
use crate::r#loop::context::ChatMessage;
use crate::r#loop::state::{FunctionCall, ToolCallRequest};
```

With:

```rust
// NEW
use super::types::{ChatMessage, FunctionCall, ToolCallRequest};
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/model/client.rs
git commit -m "add: model/client.rs — ModelClient moved from loop/model.rs"
```

---

### Task 3: Create top-level `queue.rs` — move MessageQueue

**Files:**
- Create: `crates/river-gateway/src/queue.rs` (from `loop/queue.rs`)

- [ ] **Step 1: Copy `loop/queue.rs` to `queue.rs`**

Copy the entire contents of `crates/river-gateway/src/loop/queue.rs` to `crates/river-gateway/src/queue.rs`.

No import changes needed — `queue.rs` imports from `crate::api::IncomingMessage` which stays valid at the new location.

- [ ] **Step 2: Commit**

```bash
git add crates/river-gateway/src/queue.rs
git commit -m "add: queue.rs — MessageQueue moved from loop/queue.rs"
```

---

### Task 4: Update `lib.rs` — swap module declarations

**Files:**
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Replace `pub mod r#loop` with new modules**

In `lib.rs`, find:

```rust
pub mod r#loop;
```

Replace with:

```rust
pub mod model;
pub mod queue;
```

Do NOT delete `pub mod r#loop;` yet — both old and new modules coexist temporarily so we can update imports file-by-file. Add the new declarations right after where `pub mod r#loop;` currently sits.

Actually — we need both to exist briefly. Change the line to:

```rust
pub mod r#loop;
pub mod model;
pub mod queue;
```

- [ ] **Step 2: Commit**

```bash
git add crates/river-gateway/src/lib.rs
git commit -m "add: model and queue module declarations in lib.rs (loop still present)"
```

---

### Task 5: Update all consumer imports

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`
- Modify: `crates/river-gateway/src/agent/context.rs`
- Modify: `crates/river-gateway/src/spectator/mod.rs`
- Modify: `crates/river-gateway/src/server.rs`
- Modify: `crates/river-gateway/src/tools/subagent.rs`

These files import types from `crate::r#loop` that now live in `crate::model` or `crate::queue`.

- [ ] **Step 1: Update `agent/task.rs`**

Replace:

```rust
use crate::r#loop::{MessageQueue, ModelClient};
use crate::r#loop::context::ChatMessage;
use crate::r#loop::state::ToolCallRequest;
```

With:

```rust
use crate::model::{ChatMessage, ModelClient, ToolCallRequest};
use crate::queue::MessageQueue;
```

- [ ] **Step 2: Update `agent/context.rs`**

Replace:

```rust
use crate::r#loop::context::ChatMessage;
```

With:

```rust
use crate::model::ChatMessage;
```

- [ ] **Step 3: Update `spectator/mod.rs`**

Replace (line 12):

```rust
use crate::r#loop::ModelClient;
```

With:

```rust
use crate::model::ModelClient;
```

Also in the test module (line 327), replace:

```rust
use crate::r#loop::context::ChatMessage;
```

With:

```rust
use crate::model::ChatMessage;
```

- [ ] **Step 4: Update `server.rs`**

Replace (line 13):

```rust
use crate::r#loop::{MessageQueue, ModelClient};
```

With:

```rust
use crate::model::ModelClient;
use crate::queue::MessageQueue;
```

- [ ] **Step 5: Update `tools/subagent.rs`**

Replace:

```rust
use crate::r#loop::ModelClient;
```

With:

```rust
use crate::model::ModelClient;
```

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/agent/task.rs crates/river-gateway/src/agent/context.rs crates/river-gateway/src/spectator/mod.rs crates/river-gateway/src/server.rs crates/river-gateway/src/tools/subagent.rs
git commit -m "refactor: update imports from loop/ to model/ and queue/"
```

---

### Task 6: Rewrite SubagentRunner — remove ContextBuilder dependency

**Files:**
- Modify: `crates/river-gateway/src/subagent/runner.rs`

This is the only live consumer of `ContextBuilder`. Replace it with inline `Vec<ChatMessage>` + `Vec<ToolSchema>`.

- [ ] **Step 1: Update imports**

Replace:

```rust
use crate::r#loop::{ChatMessage, ContextBuilder, ModelClient, ToolCallRequest};
```

With:

```rust
use crate::model::{ChatMessage, ModelClient, ToolCallRequest};
```

- [ ] **Step 2: Replace `context` field in struct**

In the `SubagentRunner` struct, replace:

```rust
    context: ContextBuilder,
    config: SubagentConfig,
```

With:

```rust
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSchema>,
    config: SubagentConfig,
```

- [ ] **Step 3: Update constructor**

In `SubagentRunner::new()`, replace:

```rust
        Self {
            id,
            subagent_type,
            task,
            model_client,
            tool_executor,
            queue,
            shutdown_rx,
            result_tx: Some(result_tx),
            context: ContextBuilder::new(),
            config,
        }
```

With:

```rust
        Self {
            id,
            subagent_type,
            task,
            model_client,
            tool_executor,
            queue,
            shutdown_rx,
            result_tx: Some(result_tx),
            messages: Vec::new(),
            tools: Vec::new(),
            config,
        }
```

- [ ] **Step 4: Update `build_initial_context`**

Replace the entire method body:

```rust
    async fn build_initial_context(&mut self) {
        self.messages.clear();

        // System prompt for subagent
        let prefs = Preferences::load(&self.config.workspace);
        let time_str = format_current_time(prefs.timezone());
        let system_prompt = format!(
            "You are a subagent (ID: {}) spawned to complete a specific task.\n\n\
             Your task: {}\n\n\
             Guidelines:\n\
             - Focus only on the assigned task\n\
             - Use the available tools to complete the task\n\
             - Report your findings/results clearly\n\
             - You can send messages to parent using internal_send tool\n\
             - When done, simply stop making tool calls\n\n\
             Current time: {}",
            self.id,
            self.task,
            time_str
        );
        self.messages.push(ChatMessage::system(system_prompt));

        // Add the task as a user message
        self.messages.push(ChatMessage::user(format!("Execute task: {}", self.task)));

        // Set available tools
        self.tools = self.tool_executor.schemas();
    }
```

- [ ] **Step 5: Update `run_task_worker`**

Replace model call and response handling. Find every occurrence of `self.context.messages()` and replace with `&self.messages`, and `self.context.tools()` with `&self.tools`. Find `self.context.add_assistant_response(...)` and replace with `self.messages.push(ChatMessage::assistant(...))`.

Specifically, replace:

```rust
            let response = self
                .model_client
                .complete(self.context.messages(), self.context.tools())
                .await?;

            // Add assistant response to context
            let tool_calls = if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            };
            self.context
                .add_assistant_response(response.content.clone(), tool_calls);
```

With:

```rust
            let response = self
                .model_client
                .complete(&self.messages, &self.tools)
                .await?;

            // Add assistant response to messages
            let tool_calls = if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            };
            self.messages.push(ChatMessage::assistant(response.content.clone(), tool_calls));
```

- [ ] **Step 6: Update `run_long_running`**

Same pattern as Step 5. Replace:

```rust
            let response = self
                .model_client
                .complete(self.context.messages(), self.context.tools())
                .await?;

            let tool_calls = if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            };
            self.context
                .add_assistant_response(response.content.clone(), tool_calls);
```

With:

```rust
            let response = self
                .model_client
                .complete(&self.messages, &self.tools)
                .await?;

            let tool_calls = if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            };
            self.messages.push(ChatMessage::assistant(response.content.clone(), tool_calls));
```

- [ ] **Step 7: Update `process_parent_messages`**

Replace:

```rust
            self.context.add_message(ChatMessage::system(format!(
                "[Parent Message] {}",
                msg.content
            )));
```

With:

```rust
            self.messages.push(ChatMessage::system(format!(
                "[Parent Message] {}",
                msg.content
            )));
```

- [ ] **Step 8: Update `execute_tools` — inline `add_tool_results`**

Replace the end of the method:

```rust
        // Add results to context
        self.context
            .add_tool_results(results, Vec::new());

        Ok(())
```

With:

```rust
        // Add tool results to messages
        for result in results {
            let content = match result.result {
                Ok(r) => r.output,
                Err(e) => format!("Error: {}", e),
            };
            self.messages.push(ChatMessage::tool(result.tool_call_id, content));
        }

        Ok(())
```

Note: the `ToolCallResponse` type is imported via `use crate::tools::{ToolCall, ToolExecutor, ToolRegistry}` — but `execute_tools` uses the result of `self.tool_executor.execute(&call)` which returns `ToolCallResponse`. Check that `ToolCallResponse` is imported. The current code calls `add_tool_results(results, Vec::new())` which takes `Vec<ToolCallResponse>`. We need to access `result.tool_call_id` and `result.result`. Add `ToolCallResponse` to the tools import if not already present:

```rust
use crate::tools::{ToolCall, ToolCallResponse, ToolExecutor, ToolRegistry};
```

- [ ] **Step 9: Commit**

```bash
git add crates/river-gateway/src/subagent/runner.rs
git commit -m "refactor: remove ContextBuilder from SubagentRunner, use inline Vec<ChatMessage>"
```

---

### Task 7: Remove dead plumbing — LoopEvent, loop_tx, mpsc channel

**Files:**
- Modify: `crates/river-gateway/src/state.rs`
- Modify: `crates/river-gateway/src/server.rs`
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Update `state.rs` — remove `LoopEvent` and `loop_tx`**

Remove from imports:

```rust
use crate::r#loop::{LoopEvent, MessageQueue};
```

Replace with:

```rust
use crate::queue::MessageQueue;
```

Remove from `AppState` struct:

```rust
    pub loop_tx: mpsc::Sender<LoopEvent>,
```

Remove from `AppState::new()` parameter list:

```rust
        loop_tx: mpsc::Sender<LoopEvent>,
```

And from the struct initialization:

```rust
            loop_tx,
```

Remove `use tokio::sync::{mpsc, RwLock};` — change to just `use tokio::sync::RwLock;` (mpsc is no longer used in this file).

In the test, remove:

```rust
        let (loop_tx, _loop_rx) = mpsc::channel(256);
```

And update the `AppState::new()` call to remove the `loop_tx` argument. Also remove `use tokio::sync::{mpsc, RwLock};` from the test imports — replace with `use tokio::sync::RwLock;`.

- [ ] **Step 2: Update `server.rs` — remove mpsc channel creation**

Remove these lines (~line 314-315):

```rust
    // Create loop components (loop_tx used by API, message_queue shared)
    let (loop_tx, _loop_rx) = mpsc::channel(256);
```

Keep the `MessageQueue::new()` line.

Remove `loop_tx` from the `AppState::new()` call (~line 346-358). Find the call and remove the `loop_tx,` argument.

Remove `use tokio::sync::{mpsc, RwLock};` — replace with `use tokio::sync::RwLock;`.

- [ ] **Step 3: Update `api/routes.rs` — remove LoopEvent send**

Remove the import:

```rust
use crate::r#loop::LoopEvent;
```

In `handle_incoming`, remove this block (~lines 280-284):

```rust
    // Send inbox update to the loop
    if state.loop_tx.send(LoopEvent::InboxUpdate(vec![inbox_path.clone()])).await.is_err() {
        tracing::error!("Failed to send inbox update to loop - channel closed");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
```

In the test module, update both `test_state()` and `test_state_with_auth()`:

Remove `use crate::r#loop::MessageQueue;` and replace with `use crate::queue::MessageQueue;`.

Change function signatures from:

```rust
    fn test_state() -> (Arc<AppState>, mpsc::Receiver<LoopEvent>) {
```

To:

```rust
    fn test_state() -> Arc<AppState> {
```

And same for `test_state_with_auth`:

```rust
    fn test_state_with_auth(token: &str) -> Arc<AppState> {
```

Remove `let (loop_tx, loop_rx) = mpsc::channel(256);` from both helpers.

Remove `loop_tx` from both `AppState::new()` calls.

Change return from `(Arc::new(AppState::new(...)), loop_rx)` to `Arc::new(AppState::new(...))`.

Update all test functions that destructure the tuple. Change every:

```rust
        let (state, _rx) = test_state();
```

To:

```rust
        let state = test_state();
```

And every:

```rust
        let (state, _rx) = test_state_with_auth("secret-token");
```

To:

```rust
        let state = test_state_with_auth("secret-token");
```

Remove `use tokio::sync::{mpsc, RwLock};` from the test imports — replace with `use tokio::sync::RwLock;`.

- [ ] **Step 4: Build and test**

Run: `cd /home/cassie/river-engine && cargo test 2>&1 | tail -3`

Expected: All tests pass. The `handle_incoming` test should now pass (it was returning 503 before due to the dead channel; with the send removed it returns 200).

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/state.rs crates/river-gateway/src/server.rs crates/river-gateway/src/api/routes.rs
git commit -m "refactor: remove dead LoopEvent/loop_tx plumbing from AppState and API

Fixes handle_incoming returning 503 — the mpsc receiver was always dropped."
```

---

### Task 8: Delete `loop/` directory

**Files:**
- Delete: `crates/river-gateway/src/loop/` (entire directory)
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Remove `pub mod r#loop;` from `lib.rs`**

In `lib.rs`, delete the line:

```rust
pub mod r#loop;
```

Keep `pub mod model;` and `pub mod queue;` which were added in Task 4.

- [ ] **Step 2: Delete the `loop/` directory**

```bash
rm -rf crates/river-gateway/src/loop/
```

- [ ] **Step 3: Build the full workspace**

Run: `cd /home/cassie/river-engine && cargo build 2>&1 | head -20`

Expected: Clean build, no errors. All 159 deprecation warnings should be gone.

- [ ] **Step 4: Run all tests**

Run: `cd /home/cassie/river-engine && cargo test 2>&1 | grep "test result"`

Expected: All test suites pass. Total test count will be lower than 551 (the loop module's ~49 tests are gone). Expect roughly 502 tests passing.

- [ ] **Step 5: Verify no deprecation warnings remain**

Run: `cd /home/cassie/river-engine && cargo build 2>&1 | grep -i "deprecated" | head -5`

Expected: No output (no deprecation warnings from the loop module).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "remove: delete deprecated loop/ module entirely

Removes AgentLoop, ContextBuilder, ContextFile, LoopState, WakeTrigger,
LoopConfig, and LoopEvent. All shared types relocated to model/ and queue.rs.
~49 dead tests removed. 159 deprecation warnings eliminated."
```

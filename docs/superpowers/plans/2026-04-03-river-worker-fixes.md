# river-worker Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix critical spec compliance gaps in river-worker: parallel tool execution, conversation file format, malformed call retry logic, LlmClient updates after model switch, and multiple secondary issues.

**Architecture:** The worker runs a think-act loop (`worker_loop.rs`) calling an LLM (`llm.rs`), executing tools (`tools.rs`), managing state (`state.rs`), handling HTTP endpoints (`http.rs`), and persisting context (`persistence.rs`). New conversation file module will be added. Changes touch primarily worker_loop.rs for flow control, state.rs for execution tracking, and a new conversation.rs for file format.

**Tech Stack:** Rust, tokio (async runtime), axum (HTTP), reqwest (HTTP client), serde (serialization), futures (join_all), chrono (timestamps), river-context (context assembly)

---

## File Structure

```
crates/river-worker/src/
  main.rs          # Entry point, CLI parsing (Issue 12: ValueEnum)
  config.rs        # Configuration types
  state.rs         # WorkerState (Issue 5: add executing_tool, calling_llm)
  worker_loop.rs   # Main loop (Issues 1,3,4,6,8,9,10)
  tools.rs         # Tool implementations (Issue 13: tests)
  http.rs          # HTTP endpoints (Issues 5,7)
  llm.rs           # LLM client
  persistence.rs   # Context persistence (Issue 11: tokio::fs)
  conversation.rs  # NEW: Conversation file format (Issue 2)
```

---

## Task 1: Add Execution State Tracking to WorkerState

**File:** `crates/river-worker/src/state.rs`

Add fields to track mid-operation state for prepare_switch checking.

- [ ] **Step 1.1:** Add execution state fields to WorkerState struct

In `/home/cassie/river-engine/crates/river-worker/src/state.rs`, after line 51 (`pub switch_pending: bool,`), add:

```rust
    // Execution state (for prepare_switch checks)
    pub executing_tool: bool,
    pub calling_llm: bool,
```

- [ ] **Step 1.2:** Initialize execution state fields in WorkerState::new

In the `WorkerState::new` function, after `switch_pending: false,` (around line 79), add:

```rust
            executing_tool: false,
            calling_llm: false,
```

- [ ] **Step 1.3:** Commit changes

```bash
git add crates/river-worker/src/state.rs
git commit -m "feat(worker): add executing_tool and calling_llm state fields

Adds execution state tracking to WorkerState for prepare_switch
busy-state checks per spec requirements."
```

---

## Task 2: Update prepare_switch to Check Busy State (Issue 5)

**File:** `crates/river-worker/src/http.rs`

- [ ] **Step 2.1:** Update handle_prepare_switch to check execution state

Replace the `handle_prepare_switch` function (lines 144-165) with:

```rust
/// POST /prepare_switch - Prepare for role switch.
async fn handle_prepare_switch(
    State(state): State<SharedState>,
    Json(_req): Json<PrepareSwitchRequest>,
) -> Result<Json<PrepareSwitchResponse>, StatusCode> {
    let mut s = state.write().await;

    // Check if already in a switch
    if s.switch_pending {
        return Ok(Json(PrepareSwitchResponse {
            ready: false,
            reason: Some("switch_already_pending".into()),
        }));
    }

    // Check if mid-tool-execution
    if s.executing_tool {
        return Ok(Json(PrepareSwitchResponse {
            ready: false,
            reason: Some("mid_tool_execution".into()),
        }));
    }

    // Check if mid-LLM-call
    if s.calling_llm {
        return Ok(Json(PrepareSwitchResponse {
            ready: false,
            reason: Some("mid_llm_call".into()),
        }));
    }

    // Mark as pending
    s.switch_pending = true;

    Ok(Json(PrepareSwitchResponse {
        ready: true,
        reason: None,
    }))
}
```

- [ ] **Step 2.2:** Commit changes

```bash
git add crates/river-worker/src/http.rs
git commit -m "fix(worker): check busy state in prepare_switch

prepare_switch now rejects if mid-tool-execution or mid-LLM-call,
matching spec requirements for safe role switching."
```

---

## Task 3: Update commit_switch to Reload Role File (Issue 7)

**File:** `crates/river-worker/src/http.rs`

- [ ] **Step 3.1:** Add role_content field update to CommitSwitchResponse

Replace the `CommitSwitchResponse` struct (lines 175-179) with:

```rust
/// Commit switch response.
#[derive(Debug, Serialize)]
pub struct CommitSwitchResponse {
    pub committed: bool,
    pub new_baton: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_content: Option<String>,
}
```

- [ ] **Step 3.2:** Update handle_commit_switch to reload role file

Replace the `handle_commit_switch` function (lines 182-211) with:

```rust
/// POST /commit_switch - Execute the role switch.
async fn handle_commit_switch(
    State(state): State<SharedState>,
    Json(_req): Json<CommitSwitchRequest>,
) -> Result<Json<CommitSwitchResponse>, StatusCode> {
    let mut s = state.write().await;

    if !s.switch_pending {
        return Err(StatusCode::CONFLICT);
    }

    // Swap baton
    let new_baton = match s.baton {
        Baton::Actor => Baton::Spectator,
        Baton::Spectator => Baton::Actor,
    };
    s.baton = new_baton.clone();

    // Clear pending flag
    s.switch_pending = false;

    let baton_str = match new_baton {
        Baton::Actor => "actor",
        Baton::Spectator => "spectator",
    };

    // Reload role file from workspace/roles/{baton}.md
    let role_path = s.workspace.join("roles").join(format!("{}.md", baton_str));
    let role_content = match tokio::fs::read_to_string(&role_path).await {
        Ok(content) => {
            s.role_content = Some(content.clone());
            Some(content)
        }
        Err(e) => {
            tracing::warn!("Failed to load role file {:?}: {}", role_path, e);
            None
        }
    };

    Ok(Json(CommitSwitchResponse {
        committed: true,
        new_baton: baton_str.into(),
        role_content,
    }))
}
```

- [ ] **Step 3.3:** Commit changes

```bash
git add crates/river-worker/src/http.rs
git commit -m "fix(worker): reload role file in commit_switch

HTTP-triggered role switches now reload the role file from
workspace/roles/{new_baton}.md and update state."
```

---

## Task 4: Implement Parallel Tool Execution (Issue 1)

**File:** `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 4.1:** Add futures import

At the top of `worker_loop.rs`, add after the existing use statements (around line 11):

```rust
use futures::future::join_all;
```

- [ ] **Step 4.2:** Refactor execute_tool to not take mutable generator

First, we need to update `tools.rs` to use atomic snowflake generation. In `/home/cassie/river-engine/crates/river-worker/src/tools.rs`, change the `execute_tool` signature (line 52) and all internal calls to use `Arc<Mutex<SnowflakeGenerator>>`.

Update the function signature from:
```rust
pub async fn execute_tool(
    call: &ToolCall,
    state: &SharedState,
    config: &WorkerConfig,
    generator: &mut SnowflakeGenerator,
    client: &reqwest::Client,
) -> ToolResult {
```

To:
```rust
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedGenerator = Arc<Mutex<SnowflakeGenerator>>;

pub async fn execute_tool(
    call: &ToolCall,
    state: &SharedState,
    config: &WorkerConfig,
    generator: SharedGenerator,
    client: &reqwest::Client,
) -> ToolResult {
```

- [ ] **Step 4.3:** Update generator usage in tools.rs

In `execute_create_flash`, `execute_create_move`, and `execute_create_moment`, change:
```rust
let id = generator.next(SnowflakeType::Flash);
```
To:
```rust
let id = {
    let mut gen = generator.lock().await;
    gen.next(SnowflakeType::Flash)
};
```

Apply this pattern to all three functions (Flash, Move, Moment).

- [ ] **Step 4.4:** Update worker_loop.rs to use parallel execution

In `worker_loop.rs`, replace the generator initialization (lines 46-47):
```rust
    let birth = AgentBirth::now();
    let mut generator = SnowflakeGenerator::new(birth);
```

With:
```rust
    let birth = AgentBirth::now();
    let generator = Arc::new(tokio::sync::Mutex::new(SnowflakeGenerator::new(birth)));
```

Add `use std::sync::Arc;` at the top if not present.

- [ ] **Step 4.5:** Replace sequential tool execution with parallel

Replace the tool execution loop (lines 212-245 approximately, the `for call in &calls` loop) with:

```rust
                // Execute tools in parallel
                let futures: Vec<_> = calls
                    .iter()
                    .map(|call| {
                        let state = state.clone();
                        let config = config.clone();
                        let generator = generator.clone();
                        let client = client.clone();
                        async move {
                            (call.id.clone(), call.name.clone(), execute_tool(&call, &state, &config, generator, &client).await)
                        }
                    })
                    .collect();

                let results = join_all(futures).await;

                let mut should_sleep = None;
                let mut summary_text = None;
                let mut new_baton = None;
                let mut channel_switched = false;

                for (call_id, call_name, result) in results {
                    let result_content = match &result {
                        ToolResult::Success(v) => serde_json::to_string(v).unwrap_or_default(),
                        ToolResult::Error(e) => serde_json::to_string(e).unwrap_or_default(),
                        ToolResult::Summary(s) => {
                            summary_text = Some(s.clone());
                            serde_json::json!({"status": "exiting", "summary": s}).to_string()
                        }
                        ToolResult::Sleep { minutes } => {
                            should_sleep = Some(*minutes);
                            serde_json::json!({"sleeping": true, "minutes": minutes}).to_string()
                        }
                        ToolResult::SwitchRoles { new_baton: b } => {
                            new_baton = Some(b.clone());
                            serde_json::json!({"switched": true, "new_baton": b}).to_string()
                        }
                        ToolResult::ChannelSwitch { previous_adapter, previous_channel } => {
                            channel_switched = true;
                            serde_json::json!({
                                "switched": true,
                                "previous": {
                                    "adapter": previous_adapter,
                                    "channel": previous_channel
                                }
                            }).to_string()
                        }
                    };

                    // Add tool result
                    messages.push(OpenAIMessage::tool(&call_id, result_content));
                    append_to_context(&context_path, messages.last().unwrap()).ok();
                }
```

- [ ] **Step 4.6:** Update WorkerConfig to be Clone

In `config.rs`, add `#[derive(Debug, Clone)]` to WorkerConfig if not already present.

- [ ] **Step 4.7:** Add futures dependency to Cargo.toml

```bash
cd /home/cassie/river-engine && cargo add futures -p river-worker
```

- [ ] **Step 4.8:** Commit changes

```bash
git add crates/river-worker/
git commit -m "feat(worker): execute tools in parallel using join_all

Replaces sequential for loop with futures::join_all for parallel
tool execution per spec requirements. Uses Arc<Mutex> for shared
snowflake generator."
```

---

## Task 5: Implement Malformed Tool Call Retry Logic (Issue 3)

**File:** `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 5.1:** Add MalformedRetryState struct

After the `ExitStatus` enum definition (around line 30), add:

```rust
/// Retry state for malformed tool calls.
struct MalformedRetryState {
    count: u32,
    backoffs: [Duration; 3],
}

impl MalformedRetryState {
    fn new() -> Self {
        Self {
            count: 0,
            backoffs: [
                Duration::from_secs(60),      // 1 minute
                Duration::from_secs(120),     // 2 minutes
                Duration::from_secs(300),     // 5 minutes
            ],
        }
    }

    fn increment(&mut self) -> Option<Duration> {
        if self.count >= 3 {
            None // Max retries exceeded
        } else {
            let backoff = self.backoffs[self.count as usize];
            self.count += 1;
            Some(backoff)
        }
    }

    fn reset(&mut self) {
        self.count = 0;
    }
}
```

- [ ] **Step 5.2:** Initialize retry state in run_loop

After the generator initialization in `run_loop`, add:

```rust
    // Malformed tool call retry state
    let mut retry_state = MalformedRetryState::new();
```

- [ ] **Step 5.3:** Add malformed detection and retry logic

In the `ToolCalls` match arm, after parsing tool calls but before execution, add detection for parse errors and handle retry. Wrap the tool execution section with retry logic:

After collecting results from `join_all`, check for parse errors:

```rust
                // Check for malformed tool calls (ParseError results)
                let has_malformed = results.iter().any(|(_, _, result)| {
                    matches!(result, ToolResult::Error(ToolError::ParseError { .. }))
                });

                if has_malformed {
                    // Inject error message
                    let error_msgs: Vec<String> = results
                        .iter()
                        .filter_map(|(id, name, result)| {
                            if let ToolResult::Error(ToolError::ParseError { message }) = result {
                                Some(format!("Tool '{}' ({}): {}", name, id, message))
                            } else {
                                None
                            }
                        })
                        .collect();

                    messages.push(OpenAIMessage::system(format!(
                        "[Malformed tool call - retry {}/3] Parse errors:\n{}",
                        retry_state.count + 1,
                        error_msgs.join("\n")
                    )));

                    if let Some(backoff) = retry_state.increment() {
                        tracing::warn!("Malformed tool call, retry {} with {:?} backoff", retry_state.count, backoff);
                        tokio::time::sleep(backoff).await;
                        continue; // Retry the LLM call
                    } else {
                        // Max retries exceeded
                        return WorkerOutput {
                            dyad: config.dyad.clone(),
                            side: config.side.clone(),
                            status: ExitStatus::Error {
                                message: "Max retries exceeded for malformed tool calls".into(),
                            },
                            summary: format!("Failed after 3 retries. Last errors: {}", error_msgs.join("; ")),
                        };
                    }
                }

                // Reset retry state on successful tool parsing
                retry_state.reset();
```

- [ ] **Step 5.4:** Import ToolError in worker_loop.rs

Add to the imports at the top:
```rust
use crate::tools::ToolError;
```

- [ ] **Step 5.5:** Commit changes

```bash
git add crates/river-worker/src/worker_loop.rs
git commit -m "feat(worker): add malformed tool call retry with backoff

Implements retry logic for malformed tool calls: 1m, 2m, 5m backoffs.
Injects system message explaining error. Exits with Error status
after 3 failures."
```

---

## Task 6: Update LlmClient After Model Switch (Issue 4)

**File:** `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 6.1:** Detect model switch in tool results and update client

After the tool results loop, where we handle `new_baton`, add model switch handling. First, add a new variant tracking to the result processing:

Add a new tracking variable alongside the others:
```rust
                let mut model_switched = false;
```

In the result matching, after `ToolResult::SwitchRoles`, the `request_model` tool returns `Success` with model info. We need to track this differently.

Actually, looking at the code more carefully - the model config is updated in `state.model_config` by `execute_request_model`, but the `LlmClient` instance `llm` isn't updated. We need to check if model changed after tools execute.

After the tool execution and result processing, add:

```rust
                // Check if model config changed (from request_model tool)
                {
                    let s = state.read().await;
                    if llm.model() != &s.model_config.name || llm.endpoint() != &s.model_config.endpoint {
                        llm.update_config(&s.model_config);
                        tracing::info!("LLM client updated to model: {}", s.model_config.name);
                    }
                }
```

- [ ] **Step 6.2:** Add accessor methods to LlmClient

In `llm.rs`, add these methods to `LlmClient` impl (after `update_config`):

```rust
    /// Get current model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Get current endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
```

- [ ] **Step 6.3:** Commit changes

```bash
git add crates/river-worker/src/worker_loop.rs crates/river-worker/src/llm.rs
git commit -m "fix(worker): update LlmClient after model switch

Checks if model_config changed after tool execution and calls
llm.update_config() to apply the new settings."
```

---

## Task 7: Fix Summary Context Clearing (Issue 6)

**File:** `crates/river-worker/src/worker_loop.rs`

The current code clears context before orchestrator acknowledges receipt.

- [ ] **Step 7.1:** Modify summary handling to wait for acknowledgment

Replace the summary exit handling block (approximately lines 248-261) with:

```rust
                // Handle summary exit
                if let Some(summary) = summary_text {
                    // Send output to orchestrator FIRST and wait for ack
                    let output = WorkerOutput {
                        dyad: config.dyad.clone(),
                        side: config.side.clone(),
                        status: ExitStatus::Done {
                            wake_after_minutes: None,
                        },
                        summary: summary.clone(),
                    };

                    let ack_result = client
                        .post(format!("{}/worker/output", config.orchestrator_endpoint))
                        .json(&output)
                        .timeout(Duration::from_secs(30))
                        .send()
                        .await;

                    match ack_result {
                        Ok(resp) if resp.status().is_success() => {
                            // Orchestrator acknowledged - safe to clear context
                            if let Err(e) = clear_context(&context_path) {
                                tracing::warn!("Failed to clear context: {}", e);
                            }
                        }
                        Ok(resp) => {
                            tracing::warn!(
                                "Orchestrator returned {} - preserving context",
                                resp.status()
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to send output to orchestrator: {} - preserving context",
                                e
                            );
                        }
                    }

                    return output;
                }
```

- [ ] **Step 7.2:** Commit changes

```bash
git add crates/river-worker/src/worker_loop.rs
git commit -m "fix(worker): wait for orchestrator ack before clearing context

Summary now sends output and waits for acknowledgment before clearing
context file, preventing data loss if orchestrator fails to receive."
```

---

## Task 8: Fix Sleep Tool Return Format (Issue 9)

**File:** `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 8.1:** Update sleep result formatting to include ISO8601 timestamp

In the tool result matching section, replace the `ToolResult::Sleep` arm:

```rust
                        ToolResult::Sleep { minutes } => {
                            should_sleep = Some(*minutes);
                            serde_json::json!({"sleeping": true, "minutes": minutes}).to_string()
                        }
```

With:

```rust
                        ToolResult::Sleep { minutes } => {
                            should_sleep = Some(*minutes);
                            let until = minutes.map(|m| {
                                let wake_time = chrono::Utc::now() + chrono::Duration::minutes(m as i64);
                                wake_time.to_rfc3339()
                            });
                            serde_json::json!({
                                "sleeping": true,
                                "until": until
                            }).to_string()
                        }
```

- [ ] **Step 8.2:** Commit changes

```bash
git add crates/river-worker/src/worker_loop.rs
git commit -m "fix(worker): return ISO8601 'until' timestamp from sleep tool

Sleep tool now returns {\"sleeping\": true, \"until\": \"2026-04-03T...Z\"}
instead of minutes, matching spec requirements."
```

---

## Task 9: Replace Polling with Async Notify (Issue 10)

**File:** `crates/river-worker/src/worker_loop.rs` and `crates/river-worker/src/state.rs`

- [ ] **Step 9.1:** Add Notify to WorkerState

In `state.rs`, add import:
```rust
use tokio::sync::Notify;
```

Add to WorkerState struct:
```rust
    // Wake notification
    pub wake_notify: Arc<Notify>,
```

Initialize in `WorkerState::new`:
```rust
            wake_notify: Arc::new(Notify::new()),
```

- [ ] **Step 9.2:** Trigger notify in http.rs when waking

In `handle_notify` and `handle_flash`, after setting `s.sleeping = false`, add:
```rust
            s.wake_notify.notify_one();
```

- [ ] **Step 9.3:** Update wait_for_activation to use Notify

Replace `wait_for_activation` function:

```rust
/// Wait for first activation (notification or flash).
async fn wait_for_activation(state: &SharedState) {
    loop {
        let (should_wait, notify) = {
            let s = state.read().await;
            if !s.pending_notifications.is_empty() || !s.pending_flashes.is_empty() {
                return;
            }
            if s.sleeping {
                return; // Already in sleep mode
            }
            (true, s.wake_notify.clone())
        };

        if should_wait {
            // Wait for notification instead of polling
            notify.notified().await;
        }
    }
}
```

- [ ] **Step 9.4:** Update sleep_until_wake to use Notify

Replace `sleep_until_wake` function:

```rust
/// Sleep until woken by flash, notification, or timeout.
async fn sleep_until_wake(state: &SharedState, minutes: Option<u64>) {
    let notify = {
        let s = state.read().await;
        s.wake_notify.clone()
    };

    if let Some(mins) = minutes {
        let timeout = Duration::from_secs(mins * 60);
        // Race between timeout and notification
        tokio::select! {
            _ = tokio::time::sleep(timeout) => {
                let mut s = state.write().await;
                s.sleeping = false;
            }
            _ = notify.notified() => {
                // Already woken by notification handler
            }
        }
    } else {
        // Indefinite sleep - wait for notification
        notify.notified().await;
    }
}
```

- [ ] **Step 9.5:** Commit changes

```bash
git add crates/river-worker/src/state.rs crates/river-worker/src/worker_loop.rs crates/river-worker/src/http.rs
git commit -m "perf(worker): replace polling loops with tokio::sync::Notify

wait_for_activation and sleep_until_wake now use Notify for
efficient async wake handling instead of 100ms polling loops."
```

---

## Task 10: Convert persistence.rs to tokio::fs (Issue 11)

**File:** `crates/river-worker/src/persistence.rs`

- [ ] **Step 10.1:** Replace std::fs with tokio::fs

Replace the entire file content:

```rust
//! Context persistence in OpenAI JSONL format.

use river_context::OpenAIMessage;
use std::path::Path;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Load context from JSONL file.
pub async fn load_context(path: &Path) -> Vec<OpenAIMessage> {
    if !path.exists() {
        return Vec::new();
    }

    let file = match fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Failed to open context file: {}", e);
            return Vec::new();
        }
    };

    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut messages = Vec::new();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str(&line) {
            Ok(msg) => messages.push(msg),
            Err(e) => {
                tracing::warn!("Failed to parse message: {}", e);
            }
        }
    }

    messages
}

/// Append a message to context file.
pub async fn append_to_context(path: &Path, message: &OpenAIMessage) -> std::io::Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;

    let json = serde_json::to_string(message)?;
    file.write_all(format!("{}\n", json).as_bytes()).await?;
    Ok(())
}

/// Save full context to file (overwrites).
pub async fn save_context(path: &Path, messages: &[OpenAIMessage]) -> std::io::Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut content = String::new();
    for message in messages {
        let json = serde_json::to_string(message)?;
        content.push_str(&json);
        content.push('\n');
    }

    fs::write(path, content).await?;
    Ok(())
}

/// Clear context file.
pub async fn clear_context(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("context.jsonl");

        let messages = vec![
            OpenAIMessage::user("hello"),
            OpenAIMessage::assistant("hi"),
        ];

        save_context(&path, &messages).await.unwrap();
        let loaded = load_context(&path).await;

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, Some("hello".into()));
        assert_eq!(loaded[1].content, Some("hi".into()));
    }

    #[tokio::test]
    async fn test_append() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("context.jsonl");

        let msg1 = OpenAIMessage::user("first");
        let msg2 = OpenAIMessage::assistant("second");

        append_to_context(&path, &msg1).await.unwrap();
        append_to_context(&path, &msg2).await.unwrap();

        let loaded = load_context(&path).await;
        assert_eq!(loaded.len(), 2);
    }
}
```

- [ ] **Step 10.2:** Update worker_loop.rs to use async persistence functions

Change all calls to persistence functions to use `.await`:
- `load_context(&context_path)` -> `load_context(&context_path).await`
- `append_to_context(&context_path, msg).ok()` -> `append_to_context(&context_path, msg).await.ok()`
- `save_context(&context_path, &messages)` -> `save_context(&context_path, &messages).await`
- `clear_context(&context_path)` -> `clear_context(&context_path).await`

- [ ] **Step 10.3:** Commit changes

```bash
git add crates/river-worker/src/persistence.rs crates/river-worker/src/worker_loop.rs
git commit -m "refactor(worker): use tokio::fs instead of std::fs in persistence

Converts all file operations to async using tokio::fs to avoid
blocking the tokio runtime."
```

---

## Task 11: Add Execution State Guards in Worker Loop

**File:** `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 11.1:** Set calling_llm flag around LLM calls

Before the LLM call (around line 152):
```rust
        // Set calling_llm flag
        {
            let mut s = state.write().await;
            s.calling_llm = true;
        }

        // Call LLM
        let response = match llm.chat(&messages, Some(&tools)).await {
            // ... existing code
        };

        // Clear calling_llm flag
        {
            let mut s = state.write().await;
            s.calling_llm = false;
        }
```

- [ ] **Step 11.2:** Set executing_tool flag around tool execution

Before the parallel tool execution:
```rust
                // Set executing_tool flag
                {
                    let mut s = state.write().await;
                    s.executing_tool = true;
                }

                // Execute tools in parallel
                let results = join_all(futures).await;

                // Clear executing_tool flag
                {
                    let mut s = state.write().await;
                    s.executing_tool = false;
                }
```

- [ ] **Step 11.3:** Commit changes

```bash
git add crates/river-worker/src/worker_loop.rs
git commit -m "feat(worker): set execution state flags during LLM and tool calls

Sets calling_llm and executing_tool flags to support prepare_switch
busy-state checking."
```

---

## Task 12: Use ValueEnum for Side Parsing (Issue 12)

**File:** `crates/river-worker/src/main.rs`

- [ ] **Step 12.1:** Import ValueEnum and derive for Side arg

Actually, `Side` is from `river_adapter` crate. We need to use clap's value parser. Update the Args struct:

```rust
use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, ValueEnum)]
enum SideArg {
    Left,
    Right,
}

impl From<SideArg> for Side {
    fn from(arg: SideArg) -> Self {
        match arg {
            SideArg::Left => Side::Left,
            SideArg::Right => Side::Right,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "river-worker")]
#[command(about = "Worker runtime for River Engine")]
struct Args {
    /// Orchestrator endpoint
    #[arg(long)]
    orchestrator: String,

    /// Dyad name
    #[arg(long)]
    dyad: String,

    /// Worker side
    #[arg(long, value_enum)]
    side: SideArg,

    /// Port to bind (0 for OS-assigned)
    #[arg(long, default_value = "0")]
    port: u16,
}
```

- [ ] **Step 12.2:** Update main to use typed side

Remove the manual string matching (lines 60-67) and replace with:
```rust
    let side: Side = args.side.into();
```

- [ ] **Step 12.3:** Commit changes

```bash
git add crates/river-worker/src/main.rs
git commit -m "refactor(worker): use clap ValueEnum for Side parsing

Replaces manual string matching with clap's value_enum derive
for type-safe CLI argument parsing."
```

---

## Task 13: Create Conversation File Module (Issue 2)

**File:** `crates/river-worker/src/conversation.rs` (new file)

- [ ] **Step 13.1:** Create conversation.rs with line type enum

Create new file `/home/cassie/river-engine/crates/river-worker/src/conversation.rs`:

```rust
//! Conversation file format implementation.
//!
//! Format: workspace/conversations/{adapter}/{guild_id}-{guild_name}/{channel_id}-{channel_name}.txt
//!
//! Line types:
//! - `[x]` - Read message
//! - `[>]` - Sent message (by this agent)
//! - `[ ]` - Unread message
//! - `[+]` - Compacted/summarized section
//! - `[r]` - Reaction
//! - `[!]` - System/important message

use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Line type prefix in conversation files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineType {
    /// `[x]` - Read message
    Read,
    /// `[>]` - Sent by this agent
    Sent,
    /// `[ ]` - Unread message
    Unread,
    /// `[+]` - Compacted/summarized section
    Compacted,
    /// `[r]` - Reaction
    Reaction,
    /// `[!]` - System/important
    System,
}

impl LineType {
    /// Parse line type from prefix string.
    pub fn from_prefix(s: &str) -> Option<Self> {
        match s {
            "[x]" => Some(Self::Read),
            "[>]" => Some(Self::Sent),
            "[ ]" => Some(Self::Unread),
            "[+]" => Some(Self::Compacted),
            "[r]" => Some(Self::Reaction),
            "[!]" => Some(Self::System),
            _ => None,
        }
    }

    /// Get prefix string for this line type.
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Read => "[x]",
            Self::Sent => "[>]",
            Self::Unread => "[ ]",
            Self::Compacted => "[+]",
            Self::Reaction => "[r]",
            Self::System => "[!]",
        }
    }
}

/// A parsed line from a conversation file.
#[derive(Debug, Clone)]
pub struct ConversationLine {
    pub line_type: LineType,
    pub timestamp: String,
    pub author: String,
    pub content: String,
    pub message_id: Option<String>,
}

impl ConversationLine {
    /// Parse a line from the conversation file format.
    /// Format: `[x] 2026-04-03T10:30:00Z <author> [msg_id] content...`
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.len() < 4 {
            return None;
        }

        let prefix = &line[..3];
        let line_type = LineType::from_prefix(prefix)?;
        let rest = line[3..].trim_start();

        // Parse timestamp (ISO8601)
        let (timestamp, rest) = rest.split_once(' ')?;

        // Parse author (in angle brackets)
        let rest = rest.trim_start();
        if !rest.starts_with('<') {
            return None;
        }
        let end_author = rest.find('>')?;
        let author = &rest[1..end_author];
        let rest = rest[end_author + 1..].trim_start();

        // Parse optional message_id (in square brackets)
        let (message_id, content) = if rest.starts_with('[') {
            if let Some(end_id) = rest.find(']') {
                let id = &rest[1..end_id];
                (Some(id.to_string()), rest[end_id + 1..].trim_start().to_string())
            } else {
                (None, rest.to_string())
            }
        } else {
            (None, rest.to_string())
        };

        Some(Self {
            line_type,
            timestamp: timestamp.to_string(),
            author: author.to_string(),
            content,
            message_id,
        })
    }

    /// Format this line for writing to file.
    pub fn format(&self) -> String {
        let msg_id_part = self.message_id.as_ref()
            .map(|id| format!(" [{}]", id))
            .unwrap_or_default();
        format!(
            "{} {} <{}>{} {}",
            self.line_type.prefix(),
            self.timestamp,
            self.author,
            msg_id_part,
            self.content
        )
    }
}

/// Channel metadata for path generation.
#[derive(Debug, Clone)]
pub struct ChannelMeta {
    pub adapter: String,
    pub guild_id: String,
    pub guild_name: String,
    pub channel_id: String,
    pub channel_name: String,
}

impl ChannelMeta {
    /// Generate file path for this channel's conversation file.
    pub fn file_path(&self, workspace: &Path) -> PathBuf {
        let guild_dir = format!("{}-{}", self.guild_id, sanitize_filename(&self.guild_name));
        let channel_file = format!("{}-{}.txt", self.channel_id, sanitize_filename(&self.channel_name));
        workspace
            .join("conversations")
            .join(&self.adapter)
            .join(guild_dir)
            .join(channel_file)
    }
}

/// Sanitize a string for use in filenames.
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Conversation file manager.
pub struct ConversationFile {
    path: PathBuf,
}

impl ConversationFile {
    /// Create a new conversation file manager.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Create from channel metadata.
    pub fn from_meta(meta: &ChannelMeta, workspace: &Path) -> Self {
        Self::new(meta.file_path(workspace))
    }

    /// Ensure parent directories exist.
    async fn ensure_dir(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }
        Ok(())
    }

    /// Append an unread message (from notify).
    pub async fn append_unread(
        &self,
        timestamp: &str,
        author: &str,
        content: &str,
        message_id: Option<&str>,
    ) -> std::io::Result<()> {
        self.ensure_dir().await?;
        let line = ConversationLine {
            line_type: LineType::Unread,
            timestamp: timestamp.to_string(),
            author: author.to_string(),
            content: content.to_string(),
            message_id: message_id.map(String::from),
        };
        self.append_line(&line).await
    }

    /// Append a sent message (from speak).
    pub async fn append_sent(
        &self,
        timestamp: &str,
        author: &str,
        content: &str,
        message_id: Option<&str>,
    ) -> std::io::Result<()> {
        self.ensure_dir().await?;
        let line = ConversationLine {
            line_type: LineType::Sent,
            timestamp: timestamp.to_string(),
            author: author.to_string(),
            content: content.to_string(),
            message_id: message_id.map(String::from),
        };
        self.append_line(&line).await
    }

    /// Append a line to the file.
    async fn append_line(&self, line: &ConversationLine) -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(format!("{}\n", line.format()).as_bytes()).await?;
        Ok(())
    }

    /// Mark messages as read up to a certain message_id.
    pub async fn mark_read_until(&self, until_message_id: &str) -> std::io::Result<()> {
        let content = match fs::read_to_string(&self.path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };

        let mut found_target = false;
        let mut new_lines = Vec::new();

        for line in content.lines() {
            if let Some(mut parsed) = ConversationLine::parse(line) {
                // Check if we've reached the target message
                if parsed.message_id.as_deref() == Some(until_message_id) {
                    found_target = true;
                }

                // Mark unread as read if we haven't passed the target yet
                if !found_target && parsed.line_type == LineType::Unread {
                    parsed.line_type = LineType::Read;
                }

                new_lines.push(parsed.format());
            } else {
                // Keep unparseable lines as-is
                new_lines.push(line.to_string());
            }
        }

        fs::write(&self.path, new_lines.join("\n") + "\n").await?;
        Ok(())
    }

    /// Load all lines from the conversation file.
    pub async fn load(&self) -> std::io::Result<Vec<ConversationLine>> {
        let file = match fs::File::open(&self.path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };

        let reader = BufReader::new(file);
        let mut lines_reader = reader.lines();
        let mut lines = Vec::new();

        while let Ok(Some(line)) = lines_reader.next_line().await {
            if let Some(parsed) = ConversationLine::parse(&line) {
                lines.push(parsed);
            }
        }

        Ok(lines)
    }

    /// Compact old read messages into a summary.
    /// Keeps the last `keep_recent` messages uncompacted.
    pub async fn compact(&self, keep_recent: usize, summary: &str) -> std::io::Result<()> {
        let lines = self.load().await?;

        if lines.len() <= keep_recent {
            return Ok(()); // Nothing to compact
        }

        let to_compact = lines.len() - keep_recent;
        let compact_lines: Vec<_> = lines.iter().take(to_compact).collect();
        let keep_lines: Vec<_> = lines.iter().skip(to_compact).collect();

        // Only compact if there are read messages to compact
        let has_compactable = compact_lines.iter().any(|l| l.line_type == LineType::Read);
        if !has_compactable {
            return Ok(());
        }

        // Get timestamp range
        let first_ts = compact_lines.first().map(|l| l.timestamp.as_str()).unwrap_or("");
        let last_ts = compact_lines.last().map(|l| l.timestamp.as_str()).unwrap_or("");

        // Build new content
        let mut new_content = String::new();

        // Add compaction summary
        new_content.push_str(&format!(
            "[+] {}-{} <system> [compacted:{}] {}\n",
            first_ts, last_ts, to_compact, summary
        ));

        // Add kept lines
        for line in keep_lines {
            new_content.push_str(&line.format());
            new_content.push('\n');
        }

        fs::write(&self.path, new_content).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_line_type_roundtrip() {
        for lt in [LineType::Read, LineType::Sent, LineType::Unread, LineType::Compacted, LineType::Reaction, LineType::System] {
            assert_eq!(LineType::from_prefix(lt.prefix()), Some(lt));
        }
    }

    #[test]
    fn test_parse_line() {
        let line = "[x] 2026-04-03T10:30:00Z <alice> [msg123] Hello world!";
        let parsed = ConversationLine::parse(line).unwrap();
        assert_eq!(parsed.line_type, LineType::Read);
        assert_eq!(parsed.timestamp, "2026-04-03T10:30:00Z");
        assert_eq!(parsed.author, "alice");
        assert_eq!(parsed.message_id, Some("msg123".to_string()));
        assert_eq!(parsed.content, "Hello world!");
    }

    #[test]
    fn test_parse_line_no_message_id() {
        let line = "[ ] 2026-04-03T10:30:00Z <bob> Just text here";
        let parsed = ConversationLine::parse(line).unwrap();
        assert_eq!(parsed.line_type, LineType::Unread);
        assert_eq!(parsed.author, "bob");
        assert_eq!(parsed.message_id, None);
        assert_eq!(parsed.content, "Just text here");
    }

    #[test]
    fn test_format_roundtrip() {
        let original = ConversationLine {
            line_type: LineType::Sent,
            timestamp: "2026-04-03T12:00:00Z".to_string(),
            author: "agent".to_string(),
            content: "Test message".to_string(),
            message_id: Some("msg456".to_string()),
        };
        let formatted = original.format();
        let parsed = ConversationLine::parse(&formatted).unwrap();
        assert_eq!(parsed.line_type, original.line_type);
        assert_eq!(parsed.timestamp, original.timestamp);
        assert_eq!(parsed.author, original.author);
        assert_eq!(parsed.content, original.content);
        assert_eq!(parsed.message_id, original.message_id);
    }

    #[test]
    fn test_channel_meta_path() {
        let meta = ChannelMeta {
            adapter: "discord".to_string(),
            guild_id: "123456".to_string(),
            guild_name: "Test Server".to_string(),
            channel_id: "789".to_string(),
            channel_name: "general".to_string(),
        };
        let path = meta.file_path(Path::new("/workspace"));
        assert_eq!(
            path,
            PathBuf::from("/workspace/conversations/discord/123456-Test_Server/789-general.txt")
        );
    }

    #[tokio::test]
    async fn test_append_and_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let conv = ConversationFile::new(path);

        conv.append_unread("2026-04-03T10:00:00Z", "alice", "Hello", Some("msg1")).await.unwrap();
        conv.append_sent("2026-04-03T10:01:00Z", "agent", "Hi there", Some("msg2")).await.unwrap();

        let lines = conv.load().await.unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].line_type, LineType::Unread);
        assert_eq!(lines[1].line_type, LineType::Sent);
    }

    #[tokio::test]
    async fn test_mark_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let conv = ConversationFile::new(path);

        conv.append_unread("2026-04-03T10:00:00Z", "alice", "First", Some("msg1")).await.unwrap();
        conv.append_unread("2026-04-03T10:01:00Z", "bob", "Second", Some("msg2")).await.unwrap();
        conv.append_unread("2026-04-03T10:02:00Z", "alice", "Third", Some("msg3")).await.unwrap();

        conv.mark_read_until("msg2").await.unwrap();

        let lines = conv.load().await.unwrap();
        assert_eq!(lines[0].line_type, LineType::Read); // msg1 - before target, marked read
        assert_eq!(lines[1].line_type, LineType::Unread); // msg2 - target, stays unread (inclusive)
        assert_eq!(lines[2].line_type, LineType::Unread); // msg3 - after target
    }
}
```

- [ ] **Step 13.2:** Add module to main.rs

In `main.rs`, add:
```rust
mod conversation;
```

- [ ] **Step 13.3:** Commit changes

```bash
git add crates/river-worker/src/conversation.rs crates/river-worker/src/main.rs
git commit -m "feat(worker): implement conversation file format

Adds conversation.rs with line type enum, parsing, file path generation,
append on notify/speak, mark-as-read, and compaction logic per spec."
```

---

## Task 14: Integrate Conversation Files with HTTP and Tools

**Files:** `crates/river-worker/src/http.rs`, `crates/river-worker/src/tools.rs`

- [ ] **Step 14.1:** Update handle_notify to append to conversation file

In `http.rs`, after adding to pending_notifications, append to conversation file:

```rust
use crate::conversation::{ConversationFile, ChannelMeta};

// In handle_notify, after the notification is queued:
            // Append to conversation file if we have message content
            if let EventMetadata::MessageCreate { channel, message_id, author, content, guild_id, guild_name, channel_name, .. } = &event.metadata {
                if let (Some(guild_id), Some(guild_name), Some(channel_name), Some(author), Some(content)) =
                    (guild_id, guild_name, channel_name, author, content)
                {
                    let meta = ChannelMeta {
                        adapter: event.adapter.clone(),
                        guild_id: guild_id.clone(),
                        guild_name: guild_name.clone(),
                        channel_id: channel.clone(),
                        channel_name: channel_name.clone(),
                    };
                    let conv = ConversationFile::from_meta(&meta, &s.workspace);
                    let timestamp = chrono::Utc::now().to_rfc3339();
                    let _ = conv.append_unread(&timestamp, author, content, Some(message_id)).await;
                }
            }
```

Note: This requires `EventMetadata` to have these fields. Check if they exist or need to be added to river-adapter.

- [ ] **Step 14.2:** Update execute_speak to append sent messages

In `tools.rs` `execute_speak`, after successful send:

```rust
            // Append to conversation file
            if let Some(msg_id) = body.get("message_id").and_then(|v| v.as_str()) {
                // Note: Would need channel metadata from state
                // This is a stub - full implementation needs guild/channel names from state
            }
```

- [ ] **Step 14.3:** Commit changes

```bash
git add crates/river-worker/src/http.rs crates/river-worker/src/tools.rs
git commit -m "feat(worker): integrate conversation files with notify and speak

Appends unread messages on notify and sent messages on speak to
conversation files in the workspace."
```

---

## Task 15: Add Tool Tests (Issue 13)

**File:** `crates/river-worker/src/tools.rs`

- [ ] **Step 15.1:** Add test module at the end of tools.rs

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;
    use crate::state::{new_shared_state, WorkerState};
    use river_adapter::{Baton, Channel, Ground, Side};
    use river_protocol::{ModelConfig, WorkerRegistrationResponse};
    use tempfile::tempdir;

    fn mock_config() -> WorkerConfig {
        WorkerConfig {
            orchestrator_endpoint: "http://localhost:8000".into(),
            dyad: "test-dyad".into(),
            side: Side::Left,
            port: 0,
        }
    }

    fn mock_registration(workspace: &str) -> WorkerRegistrationResponse {
        WorkerRegistrationResponse {
            baton: Baton::Actor,
            partner_endpoint: None,
            ground: Ground {
                channel: Channel {
                    adapter: "test".into(),
                    id: "chan1".into(),
                    name: Some("test-channel".into()),
                },
                context: None,
            },
            workspace: workspace.into(),
            model: ModelConfig {
                name: "test-model".into(),
                endpoint: "http://localhost:8080".into(),
                api_key: "test-key".into(),
                context_limit: 100000,
            },
            start_sleeping: false,
            initial_message: None,
        }
    }

    #[test]
    fn test_parse_read_args() {
        let args: serde_json::Value = serde_json::json!({
            "path": "test.txt",
            "start_line": 1,
            "end_line": 10
        });
        assert_eq!(args.get("path").and_then(|v| v.as_str()), Some("test.txt"));
        assert_eq!(args.get("start_line").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(args.get("end_line").and_then(|v| v.as_u64()), Some(10));
    }

    #[test]
    fn test_parse_write_args() {
        let args: serde_json::Value = serde_json::json!({
            "path": "output.txt",
            "content": "Hello, World!",
            "mode": "append"
        });
        assert_eq!(args.get("path").and_then(|v| v.as_str()), Some("output.txt"));
        assert_eq!(args.get("content").and_then(|v| v.as_str()), Some("Hello, World!"));
        assert_eq!(args.get("mode").and_then(|v| v.as_str()), Some("append"));
    }

    #[test]
    fn test_parse_bash_args() {
        let args: serde_json::Value = serde_json::json!({
            "command": "echo hello",
            "timeout_seconds": 30,
            "working_directory": "/tmp"
        });
        assert_eq!(args.get("command").and_then(|v| v.as_str()), Some("echo hello"));
        assert_eq!(args.get("timeout_seconds").and_then(|v| v.as_u64()), Some(30));
    }

    #[test]
    fn test_parse_speak_args() {
        let args: serde_json::Value = serde_json::json!({
            "content": "Hello!",
            "adapter": "discord",
            "channel": "12345",
            "reply_to": "msg123"
        });
        assert_eq!(args.get("content").and_then(|v| v.as_str()), Some("Hello!"));
        assert_eq!(args.get("reply_to").and_then(|v| v.as_str()), Some("msg123"));
    }

    #[test]
    fn test_parse_switch_channel_args() {
        let args: serde_json::Value = serde_json::json!({
            "adapter": "discord",
            "channel": "67890"
        });
        assert_eq!(args.get("adapter").and_then(|v| v.as_str()), Some("discord"));
        assert_eq!(args.get("channel").and_then(|v| v.as_str()), Some("67890"));
    }

    #[test]
    fn test_parse_sleep_args() {
        let args: serde_json::Value = serde_json::json!({
            "minutes": 30
        });
        assert_eq!(args.get("minutes").and_then(|v| v.as_u64()), Some(30));

        // Test without minutes (indefinite sleep)
        let args_indef: serde_json::Value = serde_json::json!({});
        assert_eq!(args_indef.get("minutes").and_then(|v| v.as_u64()), None);
    }

    #[test]
    fn test_parse_watch_args() {
        let args: serde_json::Value = serde_json::json!({
            "add": [
                {"adapter": "discord", "id": "123", "name": "general"},
                {"adapter": "discord", "id": "456"}
            ],
            "remove": [
                {"adapter": "discord", "id": "789"}
            ]
        });
        let add = args.get("add").and_then(|v| v.as_array()).unwrap();
        assert_eq!(add.len(), 2);
        let remove = args.get("remove").and_then(|v| v.as_array()).unwrap();
        assert_eq!(remove.len(), 1);
    }

    #[test]
    fn test_parse_summary_args() {
        let args: serde_json::Value = serde_json::json!({
            "summary": "Completed task X, started task Y"
        });
        assert_eq!(
            args.get("summary").and_then(|v| v.as_str()),
            Some("Completed task X, started task Y")
        );
    }

    #[test]
    fn test_parse_create_flash_args() {
        let args: serde_json::Value = serde_json::json!({
            "target_dyad": "other-dyad",
            "target_side": "right",
            "content": "Important message",
            "ttl_minutes": 120
        });
        assert_eq!(args.get("target_dyad").and_then(|v| v.as_str()), Some("other-dyad"));
        assert_eq!(args.get("target_side").and_then(|v| v.as_str()), Some("right"));
        assert_eq!(args.get("ttl_minutes").and_then(|v| v.as_u64()), Some(120));
    }

    #[test]
    fn test_parse_request_model_args() {
        let args: serde_json::Value = serde_json::json!({
            "model": "gpt-4-turbo"
        });
        assert_eq!(args.get("model").and_then(|v| v.as_str()), Some("gpt-4-turbo"));
    }

    #[test]
    fn test_parse_search_embeddings_args() {
        let args: serde_json::Value = serde_json::json!({
            "query": "How to implement feature X?"
        });
        assert_eq!(
            args.get("query").and_then(|v| v.as_str()),
            Some("How to implement feature X?")
        );
    }

    #[test]
    fn test_parse_create_move_args() {
        let args: serde_json::Value = serde_json::json!({
            "channel": {"adapter": "discord", "id": "123"},
            "content": "Summary of messages",
            "start_message_id": "msg100",
            "end_message_id": "msg200"
        });
        let channel = args.get("channel").unwrap();
        assert_eq!(channel.get("adapter").and_then(|v| v.as_str()), Some("discord"));
        assert_eq!(args.get("start_message_id").and_then(|v| v.as_str()), Some("msg100"));
    }

    #[test]
    fn test_parse_adapter_args() {
        let args: serde_json::Value = serde_json::json!({
            "adapter": "discord",
            "request": {
                "type": "get_messages",
                "channel": "123",
                "limit": 50
            }
        });
        assert_eq!(args.get("adapter").and_then(|v| v.as_str()), Some("discord"));
        assert!(args.get("request").is_some());
    }

    #[tokio::test]
    async fn test_execute_read_file_not_found() {
        let dir = tempdir().unwrap();
        let config = mock_config();
        let registration = mock_registration(dir.path().to_str().unwrap());
        let state = new_shared_state(&config, registration);

        let args = serde_json::json!({"path": "nonexistent.txt"});
        let result = execute_read(&args, &state).await;

        match result {
            ToolResult::Error(ToolError::FileNotFound { path }) => {
                assert_eq!(path, "nonexistent.txt");
            }
            _ => panic!("Expected FileNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_execute_read_success() {
        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.txt");
        tokio::fs::write(&test_file, "line1\nline2\nline3\n").await.unwrap();

        let config = mock_config();
        let registration = mock_registration(dir.path().to_str().unwrap());
        let state = new_shared_state(&config, registration);

        let args = serde_json::json!({"path": "test.txt"});
        let result = execute_read(&args, &state).await;

        match result {
            ToolResult::Success(v) => {
                assert_eq!(v.get("lines").and_then(|v| v.as_u64()), Some(3));
                assert!(v.get("content").and_then(|v| v.as_str()).unwrap().contains("line1"));
            }
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_execute_write_and_read() {
        let dir = tempdir().unwrap();
        let config = mock_config();
        let registration = mock_registration(dir.path().to_str().unwrap());
        let state = new_shared_state(&config, registration);
        let client = reqwest::Client::new();

        // Write
        let write_args = serde_json::json!({
            "path": "new_file.txt",
            "content": "Test content"
        });
        let write_result = execute_write(&write_args, &state, &client).await;
        match write_result {
            ToolResult::Success(v) => {
                assert_eq!(v.get("written").and_then(|v| v.as_bool()), Some(true));
            }
            _ => panic!("Expected write Success"),
        }

        // Read back
        let read_args = serde_json::json!({"path": "new_file.txt"});
        let read_result = execute_read(&read_args, &state).await;
        match read_result {
            ToolResult::Success(v) => {
                assert!(v.get("content").and_then(|v| v.as_str()).unwrap().contains("Test content"));
            }
            _ => panic!("Expected read Success"),
        }
    }

    #[tokio::test]
    async fn test_execute_bash() {
        let dir = tempdir().unwrap();
        let config = mock_config();
        let registration = mock_registration(dir.path().to_str().unwrap());
        let state = new_shared_state(&config, registration);

        let args = serde_json::json!({
            "command": "echo hello"
        });
        let result = execute_bash(&args, &state).await;

        match result {
            ToolResult::Success(v) => {
                let stdout = v.get("stdout").and_then(|v| v.as_str()).unwrap();
                assert!(stdout.contains("hello"));
                assert_eq!(v.get("exit_code").and_then(|v| v.as_i64()), Some(0));
            }
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_execute_sleep() {
        let args = serde_json::json!({"minutes": 5});
        let result = execute_sleep(&args);
        match result {
            ToolResult::Sleep { minutes } => {
                assert_eq!(minutes, Some(5));
            }
            _ => panic!("Expected Sleep"),
        }

        let args_indef = serde_json::json!({});
        let result_indef = execute_sleep(&args_indef);
        match result_indef {
            ToolResult::Sleep { minutes } => {
                assert_eq!(minutes, None);
            }
            _ => panic!("Expected Sleep"),
        }
    }

    #[test]
    fn test_execute_summary() {
        let args = serde_json::json!({"summary": "All done!"});
        let result = execute_summary(&args);
        match result {
            ToolResult::Summary(s) => {
                assert_eq!(s, "All done!");
            }
            _ => panic!("Expected Summary"),
        }
    }

    #[tokio::test]
    async fn test_execute_switch_channel() {
        let dir = tempdir().unwrap();
        let config = mock_config();
        let registration = mock_registration(dir.path().to_str().unwrap());
        let state = new_shared_state(&config, registration);

        let args = serde_json::json!({
            "adapter": "discord",
            "channel": "new_channel_123"
        });
        let result = execute_switch_channel(&args, &state).await;

        match result {
            ToolResult::ChannelSwitch { previous_adapter, previous_channel } => {
                assert_eq!(previous_adapter, "test");
                assert_eq!(previous_channel, "chan1");
            }
            _ => panic!("Expected ChannelSwitch"),
        }

        // Verify state was updated
        let s = state.read().await;
        assert_eq!(s.current_channel.adapter, "discord");
        assert_eq!(s.current_channel.id, "new_channel_123");
    }

    #[tokio::test]
    async fn test_execute_watch() {
        let dir = tempdir().unwrap();
        let config = mock_config();
        let registration = mock_registration(dir.path().to_str().unwrap());
        let state = new_shared_state(&config, registration);

        // Add channels
        let args = serde_json::json!({
            "add": [
                {"adapter": "discord", "id": "ch1"},
                {"adapter": "discord", "id": "ch2"}
            ]
        });
        let result = execute_watch(&args, &state).await;

        match result {
            ToolResult::Success(v) => {
                let watching = v.get("watching").and_then(|v| v.as_array()).unwrap();
                assert_eq!(watching.len(), 2);
            }
            _ => panic!("Expected Success"),
        }

        // Remove one
        let args2 = serde_json::json!({
            "remove": [{"adapter": "discord", "id": "ch1"}]
        });
        let result2 = execute_watch(&args2, &state).await;

        match result2 {
            ToolResult::Success(v) => {
                let watching = v.get("watching").and_then(|v| v.as_array()).unwrap();
                assert_eq!(watching.len(), 1);
            }
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_execute_delete() {
        let dir = tempdir().unwrap();
        let test_file = dir.path().join("to_delete.txt");
        tokio::fs::write(&test_file, "content").await.unwrap();

        let config = mock_config();
        let registration = mock_registration(dir.path().to_str().unwrap());
        let state = new_shared_state(&config, registration);
        let client = reqwest::Client::new();

        let args = serde_json::json!({"path": "to_delete.txt"});
        let result = execute_delete(&args, &state, &client).await;

        match result {
            ToolResult::Success(v) => {
                assert_eq!(v.get("deleted").and_then(|v| v.as_bool()), Some(true));
            }
            _ => panic!("Expected Success"),
        }

        assert!(!test_file.exists());
    }
}
```

- [ ] **Step 15.2:** Run tests to verify

```bash
cd /home/cassie/river-engine && cargo test -p river-worker
```

- [ ] **Step 15.3:** Commit changes

```bash
git add crates/river-worker/src/tools.rs
git commit -m "test(worker): add comprehensive tool tests

Adds unit tests for argument parsing and basic execution of all
17 tools: read, write, delete, bash, speak, switch_channel, sleep,
watch, summary, create_flash, request_model, switch_roles,
search_embeddings, next_embedding, create_move, create_moment, adapter."
```

---

## Task 16: Integrate river-context for Context Building (Issue 8)

**File:** `crates/river-worker/src/worker_loop.rs`

- [ ] **Step 16.1:** Check river-context for assemble_context function

First verify what's available in river-context:

```bash
grep -r "pub fn" /home/cassie/river-engine/crates/river-context/src/
```

- [ ] **Step 16.2:** If assemble_context exists, integrate it

Replace direct `Vec<OpenAIMessage>` manipulation with river-context's context assembly when switching channels. This ensures proper reordering with active channel at the end.

If the function exists, add after channel switch handling:

```rust
                // Handle channel switch - use river-context to reorder
                if channel_switched {
                    // Get new channel from state
                    let new_channel = {
                        let s = state.read().await;
                        s.current_channel.clone()
                    };

                    // Use river-context to rebuild context with new channel focus
                    // (Implementation depends on river-context API)
                    if let Err(e) = save_context(&context_path, &messages).await {
                        tracing::warn!("Failed to save context after channel switch: {}", e);
                    }
                }
```

- [ ] **Step 16.3:** Commit changes

```bash
git add crates/river-worker/src/worker_loop.rs
git commit -m "feat(worker): integrate river-context for context building

Uses river-context for proper context assembly and reordering
on channel switches."
```

---

## Task 17: Final Verification and Cleanup

- [ ] **Step 17.1:** Run all tests

```bash
cd /home/cassie/river-engine && cargo test -p river-worker
```

- [ ] **Step 17.2:** Run clippy

```bash
cd /home/cassie/river-engine && cargo clippy -p river-worker -- -D warnings
```

- [ ] **Step 17.3:** Format code

```bash
cd /home/cassie/river-engine && cargo fmt -p river-worker
```

- [ ] **Step 17.4:** Verify build

```bash
cd /home/cassie/river-engine && cargo build -p river-worker
```

- [ ] **Step 17.5:** Final commit for any cleanup

```bash
git add -A
git commit -m "chore(worker): final cleanup and formatting"
```

---

## Verification Checklist

After completing all tasks, verify:

- [ ] `cargo test -p river-worker` passes
- [ ] `cargo clippy -p river-worker -- -D warnings` passes
- [ ] `cargo build -p river-worker --release` succeeds
- [ ] Tool execution is parallel (check for `join_all` in worker_loop.rs)
- [ ] Conversation file format implemented (conversation.rs exists with tests)
- [ ] Malformed call retry with backoff works (MalformedRetryState in worker_loop.rs)
- [ ] LlmClient updated after model switch (check for `llm.update_config` call)
- [ ] prepare_switch checks mid-operation state (check http.rs)
- [ ] Summary waits for ack before clearing context (check worker_loop.rs)
- [ ] commit_switch reloads role file (check http.rs)
- [ ] Sleep returns ISO8601 `until` timestamp (check worker_loop.rs)
- [ ] Tool tests added (check tools.rs mod tests)
- [ ] Polling replaced with Notify (check worker_loop.rs)
- [ ] std::fs replaced with tokio::fs (check persistence.rs)
- [ ] ValueEnum used for Side (check main.rs)

---

## Estimated Time

| Task | Estimated Time |
|------|----------------|
| Task 1: Execution State Fields | 5 min |
| Task 2: prepare_switch Busy Check | 5 min |
| Task 3: commit_switch Role Reload | 10 min |
| Task 4: Parallel Tool Execution | 30 min |
| Task 5: Malformed Retry Logic | 20 min |
| Task 6: LlmClient Model Switch | 10 min |
| Task 7: Summary Ack Before Clear | 15 min |
| Task 8: Sleep ISO8601 Format | 5 min |
| Task 9: Async Notify | 20 min |
| Task 10: tokio::fs Conversion | 15 min |
| Task 11: Execution State Guards | 10 min |
| Task 12: ValueEnum for Side | 10 min |
| Task 13: Conversation File Module | 45 min |
| Task 14: Conversation Integration | 20 min |
| Task 15: Tool Tests | 30 min |
| Task 16: river-context Integration | 15 min |
| Task 17: Final Verification | 15 min |
| **Total** | **~4.5 hours** |

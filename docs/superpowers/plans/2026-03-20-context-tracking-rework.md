# Context Tracking Rework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix context rotation bugs by removing redundant executor tracking and using API prompt_tokens as single source of truth.

**Architecture:** Remove context tracking from ToolExecutor, centralize in AgentLoop using last_prompt_tokens. Add 95% hard limit gate. Fix ContextRotation lock handling.

**Tech Stack:** Rust, tokio, river-core, river-gateway

---

## File Structure

| File | Changes |
|------|---------|
| `crates/river-gateway/src/tools/executor.rs` | Remove context_used, context_limit, tracking methods |
| `crates/river-gateway/src/tools/scheduling.rs` | Fix ContextRotation lock handling |
| `crates/river-gateway/src/loop/context.rs` | Remove ContextStatus from add_tool_results |
| `crates/river-gateway/src/loop/mod.rs` | Add context_status(), 95% gate, remove executor calls |
| `crates/river-gateway/src/subagent/runner.rs` | Remove add_context() calls |

---

### Task 1: Remove context_status from ToolCallResponse

**Files:**
- Modify: `crates/river-gateway/src/tools/executor.rs:17-22`

- [ ] **Step 1: Update ToolCallResponse struct**

Remove the `context_status` field:

```rust
/// Result of executing a tool call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResponse {
    pub tool_call_id: String,
    pub result: Result<ToolResult, String>,
}
```

- [ ] **Step 2: Update execute() to not include context_status**

In the `execute` method (around line 88-92), change the return to:

```rust
        ToolCallResponse {
            tool_call_id: call.id.clone(),
            result,
        }
```

- [ ] **Step 3: Run tests to see failures**

Run: `cargo test -p river-gateway executor`
Expected: Compilation errors in tests and context.rs

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/tools/executor.rs
git commit -m "refactor(executor): remove context_status from ToolCallResponse"
```

---

### Task 2: Remove context tracking fields and methods from ToolExecutor

**Files:**
- Modify: `crates/river-gateway/src/tools/executor.rs:25-127`

- [ ] **Step 1: Remove context tracking fields**

Update ToolExecutor struct:

```rust
/// Executes tools and tracks context
pub struct ToolExecutor {
    registry: ToolRegistry,
}
```

- [ ] **Step 2: Update new() constructor**

```rust
impl ToolExecutor {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
```

- [ ] **Step 3: Remove context tracking from execute()**

In the `execute` method, remove lines 54-56 (the context_used accumulation):

```rust
// DELETE these lines:
// let output_len = tool_result.output.len();
// self.context_used += (output_len as u64) / 4;
```

Keep the logging but remove the `output_len` variable that's no longer needed for tracking.

- [ ] **Step 4: Delete context tracking methods**

Remove these methods entirely (lines 100-126):
- `context_status()`
- `add_context()`
- `reset_context()`
- `context_warning()`

Keep only `schemas()`.

- [ ] **Step 5: Remove river_core::ContextStatus import**

Delete line 3:
```rust
// DELETE: use river_core::ContextStatus;
```

- [ ] **Step 6: Run tests to see failures**

Run: `cargo test -p river-gateway executor`
Expected: Test failures in test_context_tracking and test_context_warning_threshold

- [ ] **Step 7: Commit**

```bash
git add crates/river-gateway/src/tools/executor.rs
git commit -m "refactor(executor): remove context tracking fields and methods"
```

---

### Task 3: Update executor tests

**Files:**
- Modify: `crates/river-gateway/src/tools/executor.rs:129-214`

- [ ] **Step 1: Update test_executor**

Change the ToolExecutor construction (line 142):

```rust
        let mut executor = ToolExecutor::new(registry);
```

Remove `mut` since executor no longer tracks mutable context state:

```rust
        let executor = ToolExecutor::new(registry);
```

- [ ] **Step 2: Delete test_context_tracking**

Remove the entire test (lines 171-185).

- [ ] **Step 3: Update test_unknown_tool**

Change line 190:

```rust
        let executor = ToolExecutor::new(registry);
```

- [ ] **Step 4: Delete test_context_warning_threshold**

Remove the entire test (lines 203-213).

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-gateway executor`
Expected: PASS (remaining tests should work)

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/tools/executor.rs
git commit -m "test(executor): update tests for removed context tracking"
```

---

### Task 4: Update add_tool_results signature in context.rs

**Files:**
- Modify: `crates/river-gateway/src/loop/context.rs:178-211`

- [ ] **Step 1: Remove ContextStatus parameter**

Update the method signature:

```rust
    /// Add tool results with any incoming messages
    pub fn add_tool_results(
        &mut self,
        results: Vec<ToolCallResponse>,
        incoming: Vec<IncomingMessage>,
    ) {
```

- [ ] **Step 2: Remove context status injection**

Delete lines 194-198 (the context status message injection):

```rust
        // DELETE these lines:
        // self.messages.push(ChatMessage::system(format!(
        //     "Context: {}/{} ({:.1}%)",
        //     context_status.used, context_status.limit, context_status.percent()
        // )));
```

- [ ] **Step 3: Remove unused import**

Delete `use river_core::ContextStatus;` from line 6.

- [ ] **Step 4: Run tests to see failures**

Run: `cargo test -p river-gateway context`
Expected: Test failures in tests that pass ContextStatus

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/loop/context.rs
git commit -m "refactor(context): remove ContextStatus from add_tool_results"
```

---

### Task 5: Update context.rs tests

**Files:**
- Modify: `crates/river-gateway/src/loop/context.rs:362-539`

- [ ] **Step 1: Remove context_status from test helper calls**

Update all `add_tool_results` calls in tests. For each test, remove the `context_status` argument and update assertions.

In `test_add_tool_results_basic` (lines 362-382):

```rust
    #[test]
    fn test_add_tool_results_basic() {
        let mut builder = ContextBuilder::new();
        let results = vec![ToolCallResponse {
            tool_call_id: "call_1".to_string(),
            result: Ok(ToolResult::success("Success!")),
        }];

        builder.add_tool_results(results, vec![]);

        // Should have 1 message: tool result (no more context status)
        assert_eq!(builder.messages().len(), 1);
        assert_eq!(builder.messages()[0].role, "tool");
        assert_eq!(builder.messages()[0].content, Some("Success!".to_string()));
    }
```

- [ ] **Step 2: Update test_add_tool_results_with_error**

```rust
    #[test]
    fn test_add_tool_results_with_error() {
        let mut builder = ContextBuilder::new();
        let results = vec![ToolCallResponse {
            tool_call_id: "call_err".to_string(),
            result: Err("File not found".to_string()),
        }];

        builder.add_tool_results(results, vec![]);

        assert_eq!(builder.messages()[0].role, "tool");
        let content = builder.messages()[0].content.as_ref().unwrap();
        assert!(content.contains("Error:"));
        assert!(content.contains("File not found"));
    }
```

- [ ] **Step 3: Update test_add_tool_results_with_incoming_messages**

```rust
    #[test]
    fn test_add_tool_results_with_incoming_messages() {
        let mut builder = ContextBuilder::new();
        let results = vec![ToolCallResponse {
            tool_call_id: "call_1".to_string(),
            result: Ok(ToolResult::success("Done")),
        }];
        let incoming = vec![
            test_message("Hey!", "dm", "Bob"),
            test_message("Urgent!", "alerts", "System"),
        ];

        builder.add_tool_results(results, incoming);

        // Should have 2 messages: tool result + incoming messages
        assert_eq!(builder.messages().len(), 2);

        // Check incoming messages notification
        let incoming_msg = &builder.messages()[1];
        assert_eq!(incoming_msg.role, "system");
        let content = incoming_msg.content.as_ref().unwrap();
        assert!(content.contains("Messages received during tool execution"));
        assert!(content.contains("[dm] Bob: Hey!"));
        assert!(content.contains("[alerts] System: Urgent!"));
    }
```

- [ ] **Step 4: Delete test_context_status_display_in_results**

Remove the entire test (lines 489-503) as context status is no longer displayed.

- [ ] **Step 5: Update test_multiple_tool_results**

```rust
    #[test]
    fn test_multiple_tool_results() {
        let mut builder = ContextBuilder::new();
        let results = vec![
            ToolCallResponse {
                tool_call_id: "call_1".to_string(),
                result: Ok(ToolResult::success("First result")),
            },
            ToolCallResponse {
                tool_call_id: "call_2".to_string(),
                result: Ok(ToolResult::success("Second result")),
            },
            ToolCallResponse {
                tool_call_id: "call_3".to_string(),
                result: Err("Third failed".to_string()),
            },
        ];

        builder.add_tool_results(results, vec![]);

        // 3 tool results only (no context status)
        assert_eq!(builder.messages().len(), 3);

        assert_eq!(builder.messages()[0].tool_call_id, Some("call_1".to_string()));
        assert_eq!(builder.messages()[1].tool_call_id, Some("call_2".to_string()));
        assert_eq!(builder.messages()[2].tool_call_id, Some("call_3".to_string()));
        assert!(builder.messages()[2].content.as_ref().unwrap().contains("Error"));
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p river-gateway context`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/river-gateway/src/loop/context.rs
git commit -m "test(context): update tests for removed ContextStatus"
```

---

### Task 6: Fix ContextRotation lock handling

**Files:**
- Modify: `crates/river-gateway/src/tools/scheduling.rs:156-186`

- [ ] **Step 1: Update request() to use blocking write**

```rust
    /// Request a context rotation with summary
    pub fn request(&self, summary: String) {
        self.requested.store(true, Ordering::SeqCst);
        let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
        *s = Some(summary);
    }
```

- [ ] **Step 2: Update request_auto() to use blocking write**

```rust
    /// Request auto-rotation (no summary)
    pub fn request_auto(&self) {
        self.requested.store(true, Ordering::SeqCst);
        let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
        *s = None;
    }
```

- [ ] **Step 3: Update take_request() to use blocking write**

```rust
    /// Check if rotation is requested and take the summary
    /// Returns Some(Option<String>) if rotation was requested
    /// - Some(Some(summary)) = manual rotation with summary
    /// - Some(None) = auto-rotation without summary
    /// - None = no rotation requested
    pub fn take_request(&self) -> Option<Option<String>> {
        if self.requested.swap(false, Ordering::SeqCst) {
            let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
            Some(s.take())
        } else {
            None
        }
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway scheduling`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/tools/scheduling.rs
git commit -m "fix(scheduling): use blocking write in ContextRotation to prevent summary loss"
```

---

### Task 7: Update AgentLoop - remove executor context calls

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Add context_status helper method**

Add this method to `impl AgentLoop` (around line 130, after the `new()` method):

```rust
    /// Get current context status based on last known prompt tokens
    fn context_status(&self) -> ContextStatus {
        ContextStatus {
            used: self.last_prompt_tokens,
            limit: self.config.context_limit,
        }
    }
```

- [ ] **Step 2: Remove add_context call in think_phase**

In `think_phase()` (around lines 494-505), delete the context tracking block:

```rust
        // DELETE this entire block:
        // {
        //     let mut executor = self.tool_executor.write().await;
        //     executor.add_context(response.usage.total_tokens as u64);
        //     let status = executor.context_status();
        //     tracing::debug!(
        //         context_used = status.used,
        //         context_limit = status.limit,
        //         context_percent = format!("{:.1}%", status.percent()),
        //         "Context usage updated"
        //     );
        // }
```

- [ ] **Step 3: Remove reset_context call in wake_phase**

In `wake_phase()` (around lines 351-355), delete:

```rust
            // DELETE this block:
            // {
            //     let mut executor = self.tool_executor.write().await;
            //     executor.reset_context();
            // }
```

- [ ] **Step 4: Run cargo check**

Run: `cargo check -p river-gateway`
Expected: Compilation errors in act_phase and tool executor construction

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "refactor(loop): add context_status helper, remove executor tracking calls"
```

---

### Task 8: Update act_phase - simplify rotation check

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs:603-647`

- [ ] **Step 1: Remove executor context_status call**

Delete lines 603-607:

```rust
        // DELETE:
        // let context_status = {
        //     let executor = self.tool_executor.read().await;
        //     executor.context_status()
        // };
```

- [ ] **Step 2: Update add_tool_results call**

Change line 625 to not pass context_status:

```rust
        self.context.add_tool_results(results, incoming_messages);
```

- [ ] **Step 3: Simplify rotation check**

Replace lines 627-646 with simpler check:

```rust
        // Check if context rotation was requested
        if self.context_rotation.is_requested() {
            self.state = LoopState::Settling;
        } else {
            // Back to thinking
            self.state = LoopState::Thinking;
        }
```

Remove the `is_near_limit()` check since rotation is already triggered in `think_phase()`.

- [ ] **Step 4: Run cargo check**

Run: `cargo check -p river-gateway`
Expected: Should compile (may have warnings)

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "refactor(loop): simplify act_phase rotation check"
```

---

### Task 9: Add 95% hard limit gate in think_phase

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs` (think_phase, around line 420)

- [ ] **Step 1: Add hard limit check at start of think_phase**

Add this at the very beginning of `think_phase()`, before the model call:

```rust
    async fn think_phase(&mut self) {
        // Hard limit gate - force rotation if context is dangerously full
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

        tracing::debug!("Thinking...");
        // ... rest of method
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check -p river-gateway`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "feat(loop): add 95% hard limit gate to prevent context overflow"
```

---

### Task 10: Update ToolExecutor construction sites

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs` (search for ToolExecutor::new)

- [ ] **Step 1: Find and update ToolExecutor::new calls**

Search for `ToolExecutor::new` in the file and update to remove context_limit argument:

```rust
// Before:
let executor = ToolExecutor::new(registry, self.config.context_limit);

// After:
let executor = ToolExecutor::new(registry);
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check -p river-gateway`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "refactor(loop): update ToolExecutor construction for new signature"
```

---

### Task 11: Update subagent runner

**Files:**
- Modify: `crates/river-gateway/src/subagent/runner.rs`

- [ ] **Step 1: Remove add_context calls**

Find and delete all `self.tool_executor.add_context()` calls (lines 177-178 and 248-249):

```rust
// DELETE:
// self.tool_executor
//     .add_context(response.usage.total_tokens as u64);
```

- [ ] **Step 2: Remove context_warning checks**

Find and delete the context warning checks (around lines 212-215 and 269-272):

```rust
// DELETE:
// if self.tool_executor.context_warning() {
//     tracing::warn!("Subagent {} approaching context limit", self.id);
//     return Err(RiverError::tool("Context limit approaching"));
// }
```

- [ ] **Step 3: Update ToolExecutor construction**

Find where the subagent's ToolExecutor is created and update:

```rust
// Before:
let executor = ToolExecutor::new(registry, config.context_limit);

// After:
let executor = ToolExecutor::new(registry);
```

- [ ] **Step 4: Run cargo check**

Run: `cargo check -p river-gateway`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/subagent/runner.rs
git commit -m "refactor(subagent): remove context tracking calls"
```

---

### Task 12: Run full test suite and fix any remaining issues

**Files:**
- All modified files

- [ ] **Step 1: Run full test suite**

Run: `cargo test -p river-gateway`
Expected: All tests PASS

- [ ] **Step 2: Fix any remaining compilation errors**

If there are any remaining errors, fix them following the same patterns.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p river-gateway -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: resolve remaining test and clippy issues"
```

---

### Task 13: Final verification

**Files:**
- All

- [ ] **Step 1: Build release**

Run: `cargo build -p river-gateway --release`
Expected: PASS

- [ ] **Step 2: Verify no context_used references remain**

Run: `grep -r "context_used" crates/river-gateway/src/`
Expected: No matches (or only in comments)

- [ ] **Step 3: Verify no add_context references remain**

Run: `grep -r "add_context\|reset_context" crates/river-gateway/src/`
Expected: No matches (or only in comments)

- [ ] **Step 4: Final commit if needed**

```bash
git add -A
git commit -m "chore: final cleanup for context tracking rework"
```

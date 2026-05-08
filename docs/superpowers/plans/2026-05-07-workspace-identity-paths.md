# Workspace Identity Paths Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move agent identity file reads from `{workspace}/actor/` to the workspace root and require all three files to exist.

**Architecture:** Change two functions in `agent/task.rs` to read from the workspace root instead of the `actor/` subdirectory. Change both functions to return `Result<String>` and fail if any of `AGENTS.md`, `IDENTITY.md`, or `RULES.md` is missing. Update callers and tests.

**Tech Stack:** Rust, tokio

---

## File Structure

| File | Change |
|------|--------|
| `crates/river-gateway/src/agent/task.rs:537-580` | Modify `build_system_prompt` and `build_system_prompt_sync` |
| `crates/river-gateway/src/agent/task.rs:80-105` | Modify `AgentTask::new` to propagate error from `build_system_prompt_sync` |
| `crates/river-gateway/src/agent/task.rs:797-863` | Update tests |

---

### Task 1: Update `build_system_prompt_sync` and `build_system_prompt`

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs:537-580`

- [ ] **Step 1: Change `build_system_prompt_sync` to read from workspace root and require all files**

Replace lines 560-580 (`build_system_prompt_sync`) with:

```rust
    /// Build system prompt synchronously (for use in new())
    fn build_system_prompt_sync(workspace: &Path) -> anyhow::Result<String> {
        let mut parts = Vec::new();

        for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
            let path = workspace.join(filename);
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!(
                    "Required identity file missing: {:?} ({})", path, e
                ))?;
            parts.push(content);
        }

        let prefs = Preferences::load(workspace);
        let time_str = format_current_time(prefs.timezone());
        parts.push(format!("Current time: {}", time_str));

        Ok(parts.join("\n\n---\n\n"))
    }
```

- [ ] **Step 2: Change `build_system_prompt` (async) to match**

Replace lines 536-558 (`build_system_prompt`) with:

```rust
    /// Build system prompt from workspace files (async version)
    async fn build_system_prompt(&self) -> anyhow::Result<String> {
        let mut parts = Vec::new();

        for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
            let path = self.config.workspace.join(filename);
            let content = tokio::fs::read_to_string(&path).await
                .map_err(|e| anyhow::anyhow!(
                    "Required identity file missing: {:?} ({})", path, e
                ))?;
            parts.push(content);
        }

        let prefs = Preferences::load(&self.config.workspace);
        let time_str = format_current_time(prefs.timezone());
        parts.push(format!("Current time: {}", time_str));

        Ok(parts.join("\n\n---\n\n"))
    }
```

- [ ] **Step 3: Update `AgentTask::new` to propagate the error**

At line 92, change:

```rust
        let system_prompt = Self::build_system_prompt_sync(&config.workspace);
```

to:

```rust
        let system_prompt = Self::build_system_prompt_sync(&config.workspace)?;
```

This requires `AgentTask::new` to return `anyhow::Result<Self>`. Change the function signature at line 81 from:

```rust
    pub fn new(
        config: AgentTaskConfig,
        bus: EventBus,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        flash_queue: Arc<FlashQueue>,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
```

to:

```rust
    pub fn new(
        config: AgentTaskConfig,
        bus: EventBus,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        flash_queue: Arc<FlashQueue>,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> anyhow::Result<Self> {
```

And change the return at the end of `new` (around line 107) from `Self { ... }` to `Ok(Self { ... })`.

- [ ] **Step 4: Update callers of `build_system_prompt` (async)**

Search for calls to `self.build_system_prompt().await` in the same file. These are at approximately lines 189 and 280 (in the turn cycle, during compaction). Change each from:

```rust
            let system_prompt = self.build_system_prompt().await;
```

to:

```rust
            let system_prompt = self.build_system_prompt().await
                .expect("Identity files missing during compaction");
```

These are called mid-session (re-reading files during context compaction), so if the files vanished after startup, panicking is appropriate — the system is in a broken state.

- [ ] **Step 5: Update the caller of `AgentTask::new` in `server.rs`**

In `crates/river-gateway/src/server.rs` around line 374, change:

```rust
    let agent_task = AgentTask::new(
        agent_config,
        coordinator.bus().clone(),
        message_queue,
        agent_model_client,
        state.tool_executor.clone(),
        flash_queue.clone(),
        db_arc.clone(),
        snowflake_gen.clone(),
    );
```

to:

```rust
    let agent_task = AgentTask::new(
        agent_config,
        coordinator.bus().clone(),
        message_queue,
        agent_model_client,
        state.tool_executor.clone(),
        flash_queue.clone(),
        db_arc.clone(),
        snowflake_gen.clone(),
    )?;
```

- [ ] **Step 6: Run the build to check for compile errors**

```bash
cd /home/cassie/river-engine && cargo check 2>&1
```

Expected: compiles clean. If there are other callers of `AgentTask::new` that need updating, the compiler will tell you.

- [ ] **Step 7: Commit**

```bash
cd /home/cassie/river-engine && git add crates/river-gateway/src/agent/task.rs crates/river-gateway/src/server.rs
git commit -m "feat(gateway): read identity files from workspace root, require all three"
```

---

### Task 2: Update tests

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs:797-863`

- [ ] **Step 1: Update `test_build_system_prompt_default`**

This test creates no identity files and expects a fallback. Now the function should fail. Replace the test (around line 797):

```rust
    #[tokio::test]
    async fn test_build_system_prompt_missing_files() {
        let temp = TempDir::new().unwrap();
        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();

        let message_queue = Arc::new(MessageQueue::new());
        let flash_queue = Arc::new(FlashQueue::new(10));
        let tool_executor = Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new())));
        let model_client = ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        ).unwrap();

        let (db, sg) = test_db(&temp);
        let result = AgentTask::new(
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
            db,
            sg,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("AGENTS.md"), "Error should mention AGENTS.md: {}", err);
    }
```

- [ ] **Step 2: Update `test_build_system_prompt_with_identity`**

This test writes to `actor/IDENTITY.md`. Change it to write all three files at the workspace root. Replace the test (around line 829):

```rust
    #[tokio::test]
    async fn test_build_system_prompt_with_identity() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "# Agent Protocol").unwrap();
        std::fs::write(temp.path().join("IDENTITY.md"), "I am River, a helpful assistant.").unwrap();
        std::fs::write(temp.path().join("RULES.md"), "Be helpful.").unwrap();

        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();

        let message_queue = Arc::new(MessageQueue::new());
        let flash_queue = Arc::new(FlashQueue::new(10));
        let tool_executor = Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new())));
        let model_client = ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        ).unwrap();

        let (db, sg) = test_db(&temp);
        let task = AgentTask::new(
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
            db,
            sg,
        ).unwrap();

        let prompt = task.build_system_prompt().await.unwrap();
        assert!(prompt.contains("I am River"));
        assert!(prompt.contains("Agent Protocol"));
        assert!(prompt.contains("Be helpful"));
        assert!(prompt.contains("Current time:"));
    }
```

- [ ] **Step 3: Run the tests**

```bash
cd /home/cassie/river-engine && cargo test -p river-gateway -- test_build_system_prompt 2>&1
```

Expected: both tests pass.

- [ ] **Step 4: Run the full test suite**

```bash
cd /home/cassie/river-engine && cargo test 2>&1
```

Expected: all tests pass (except possibly the 8 git tests that need `git` in PATH — those are a pre-existing issue).

- [ ] **Step 5: Commit**

```bash
cd /home/cassie/river-engine && git add crates/river-gateway/src/agent/task.rs
git commit -m "test(gateway): update system prompt tests for workspace root identity files"
```

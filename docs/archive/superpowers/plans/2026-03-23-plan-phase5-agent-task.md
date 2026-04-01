# Phase 5: Agent Task

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite the agent as a peer task managed by the coordinator. The agent runs the wake/think/act/settle cycle, uses the new context assembler, emits events to the bus, and receives flash/warning events from the spectator.

**Architecture:** `AgentTask` is spawned by the coordinator. It owns the turn cycle, tool executor, and context assembler. It reads from the message queue (incoming messages) and publishes events after each turn. Flashes from the spectator are buffered and included in the next turn's context.

**Tech Stack:** tokio, existing ModelClient, river-tools

**Depends on:** Phase 4 (coordinator + event bus)

---

## File Structure

**New files:**
- `crates/river-gateway/src/agent/task.rs` — AgentTask with turn cycle
- `crates/river-gateway/src/agent/tools.rs` — Tool dispatch wrapper

**Modified files:**
- `crates/river-gateway/src/agent/mod.rs` — export task module
- `crates/river-gateway/src/coordinator/mod.rs` — spawn agent task
- `crates/river-gateway/src/server.rs` — create coordinator instead of (or alongside) old loop

---

## Task 1: Agent Task Structure

- [ ] **Step 1: Create agent/task.rs**

```rust
//! Agent task — the acting self (I)

use crate::agent::context::{ContextAssembler, ContextBudget, AssembledContext};
use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::flash::{Flash, FlashQueue, FlashTTL};
use crate::r#loop::{MessageQueue, ModelClient, ModelResponse};
use crate::tools::ToolExecutor;
use chrono::Utc;
use river_core::SnowflakeGenerator;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for the agent task
#[derive(Debug, Clone)]
pub struct AgentTaskConfig {
    pub workspace: PathBuf,
    pub embeddings_dir: PathBuf,
    pub context_budget: ContextBudget,
    pub model_timeout: std::time::Duration,
    pub max_tool_calls: usize,
    pub history_limit: usize,
}

/// The agent task — runs as a peer task in the coordinator
pub struct AgentTask {
    config: AgentTaskConfig,
    bus: EventBus,
    message_queue: Arc<MessageQueue>,
    model_client: ModelClient,
    tool_executor: Arc<RwLock<ToolExecutor>>,
    context_assembler: ContextAssembler,
    flash_queue: Arc<FlashQueue>,
    snowflake_gen: Arc<SnowflakeGenerator>,
    turn_count: u64,
    current_channel: String,
}

impl AgentTask {
    pub fn new(
        config: AgentTaskConfig,
        bus: EventBus,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        flash_queue: Arc<FlashQueue>,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        let context_assembler = ContextAssembler::new(
            config.context_budget.clone(),
            config.embeddings_dir.clone(),
        );

        Self {
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            context_assembler,
            flash_queue,
            snowflake_gen,
            turn_count: 0,
            current_channel: "default".into(),
        }
    }

    /// Main run loop — called by coordinator
    pub async fn run(mut self) {
        let mut event_rx = self.bus.subscribe();

        tracing::info!("Agent task started");

        loop {
            tokio::select! {
                // Wait for wake trigger (new messages in queue)
                _ = self.message_queue.wait_for_messages() => {
                    self.turn_cycle().await;
                }
                // Listen for coordinator events
                event = event_rx.recv() => {
                    match event {
                        Ok(CoordinatorEvent::Shutdown) => {
                            tracing::info!("Agent task: shutdown received");
                            break;
                        }
                        Ok(CoordinatorEvent::Spectator(SpectatorEvent::Flash { content, source, ttl_turns, .. })) => {
                            // Buffer flash for next turn
                            self.flash_queue.push(Flash {
                                id: format!("flash-{}", Utc::now().timestamp_millis()),
                                content,
                                source,
                                ttl: FlashTTL::Turns(ttl_turns),
                                created: Utc::now(),
                            }).await;
                        }
                        Ok(CoordinatorEvent::Spectator(SpectatorEvent::Warning { content, .. })) => {
                            tracing::warn!(warning = %content, "Spectator warning received");
                        }
                        _ => {} // Ignore own events
                    }
                }
            }
        }

        tracing::info!("Agent task stopped");
    }

    /// One turn: wake → think → act → settle
    async fn turn_cycle(&mut self) {
        self.turn_count += 1;

        // 1. Wake: tick flash queue, emit TurnStarted
        self.flash_queue.tick_turn().await;
        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: self.current_channel.clone(),
            turn_number: self.turn_count,
            timestamp: Utc::now(),
        }));

        // 2. Assemble context
        // (load system prompt, recent messages from DB, etc.)
        // This is a simplified sketch — real implementation loads from DB/inbox
        let system_prompt = self.load_system_prompt().await;
        let recent_messages = vec![]; // TODO: load from DB
        let context = self.context_assembler.assemble(
            &self.current_channel,
            &system_prompt,
            &recent_messages,
            &self.flash_queue,
            None,  // vector store
            None,  // query embedding
        ).await;

        tracing::info!(
            turn = self.turn_count,
            tokens = context.token_estimate,
            flashes = context.layer_stats.flashes_count,
            "Context assembled"
        );

        // 3. Think (model call)
        // let response = self.model_client.chat(context.messages, tools).await;

        // 4. Act (tool calls)
        // for tool_call in response.tool_calls { ... }

        // 5. Settle: emit TurnComplete
        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
            channel: self.current_channel.clone(),
            turn_number: self.turn_count,
            transcript_summary: format!("Turn {} completed", self.turn_count),
            tool_calls: vec![],
            timestamp: Utc::now(),
        }));
    }

    async fn load_system_prompt(&self) -> String {
        let identity_path = self.config.workspace.join("IDENTITY.md");
        tokio::fs::read_to_string(&identity_path).await
            .unwrap_or_else(|_| "You are a helpful assistant.".into())
    }
}
```

**Note:** This is the skeleton. The full implementation migrates logic from `loop/mod.rs` — the wake/think/act/settle cycle, tool execution, message persistence, context rotation. That migration is the bulk of this phase.

- [ ] **Step 2: Create agent/tools.rs**

```rust
//! Tool dispatch wrapper for the agent task

use river_tools::{ToolExecutor, ToolCall, ToolCallResponse};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Execute a batch of tool calls
pub async fn execute_tools(
    executor: &Arc<RwLock<ToolExecutor>>,
    calls: Vec<ToolCall>,
) -> Vec<ToolCallResponse> {
    let mut executor = executor.write().await;
    executor.execute_all(&calls)
}
```

- [ ] **Step 3: Update agent/mod.rs**

```rust
//! Agent (I) — the acting self

pub mod context;
pub mod task;
pub mod tools;

pub use context::{ContextAssembler, ContextBudget, AssembledContext, LayerStats};
pub use task::{AgentTask, AgentTaskConfig};
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p river-gateway
```

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(gateway): add agent task skeleton with turn cycle"
```

---

## Task 2: Migrate Turn Cycle from loop/mod.rs

This is the core migration. The existing `AgentLoop` in `loop/mod.rs` (~1051 lines) has the full wake/think/act/settle cycle. We need to port it to `AgentTask`.

- [ ] **Step 1: Identify code blocks to migrate**

From `loop/mod.rs`:
- `sleep_phase()` → becomes `run()` select loop (waiting for messages or events)
- `wake_phase()` → becomes first part of `turn_cycle()` (load inbox, system prompt, context)
- `think_phase()` → model call with assembled context
- `act_phase()` → tool execution loop
- `settle_phase()` → state persistence, git commit, context rotation check

- [ ] **Step 2: Port wake logic**

Copy inbox reading logic from `wake_phase()` into `turn_cycle()`:
- Read inbox files from `WakeTrigger::Inbox(paths)`
- Load recent messages from DB
- Build system prompt from identity files + preferences

- [ ] **Step 3: Port think logic**

Copy model call from `think_phase()`:
- Call `model_client.chat()` with assembled context and tool schemas
- Handle response (content, tool calls, usage)
- Update metrics

- [ ] **Step 4: Port act logic**

Copy tool execution loop from `act_phase()`:
- Execute each tool call
- Collect results
- If model wants more tool calls, loop back to think

- [ ] **Step 5: Port settle logic**

Copy from `settle_phase()`:
- Persist messages to DB
- Git commit if needed
- Check context rotation threshold
- Write context file

- [ ] **Step 6: Wire into coordinator**

Update `server.rs` to create coordinator and spawn agent task:

```rust
let mut coordinator = Coordinator::new();
let flash_queue = Arc::new(FlashQueue::new(20));

let agent_task = AgentTask::new(
    agent_config,
    coordinator.bus().clone(),
    message_queue,
    model_client,
    tool_executor,
    flash_queue,
    snowflake_gen,
);

coordinator.spawn_task("agent", |_| agent_task.run());
```

- [ ] **Step 7: Keep old loop as fallback**

Don't delete `loop/mod.rs` yet. Add a feature flag or config option:
```rust
if config.use_coordinator {
    // New coordinator path
} else {
    // Old AgentLoop path
}
```

- [ ] **Step 8: Run tests**

```bash
cargo test
```

- [ ] **Step 9: Commit**

```bash
git add -A && git commit -m "feat(gateway): migrate agent turn cycle to coordinator task"
```

---

## Task 3: Event Emission

- [ ] **Step 1: Emit TurnStarted at wake**

At the start of each turn cycle, publish `AgentEvent::TurnStarted`.

- [ ] **Step 2: Emit TurnComplete at settle**

After settling, publish `AgentEvent::TurnComplete` with transcript summary and tool call names.

- [ ] **Step 3: Emit NoteWritten when agent writes to embeddings/**

When tool execution results in a file write to `workspace/embeddings/`, emit `AgentEvent::NoteWritten`.

- [ ] **Step 4: Emit ContextPressure when context is high**

After context assembly, if usage exceeds 80%, emit `AgentEvent::ContextPressure`.

- [ ] **Step 5: Write tests for event emission**

Spawn agent task, send a message, verify events appear on the bus.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(gateway): agent task emits lifecycle events"
```

---

## Task 4: Flash Integration

- [ ] **Step 1: Receive spectator flashes**

Already handled in the `run()` select loop — spectator Flash events are pushed to the flash queue.

- [ ] **Step 2: Tick flash queue per turn**

Already called at start of `turn_cycle()`.

- [ ] **Step 3: Verify flashes appear in context**

Write test: push a flash, run a turn, verify flash content appears in assembled context.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): agent integrates flash queue into context"
```

---

## Summary

Phase 5 is the heaviest phase — it rewrites the agent loop as a coordinator peer task:
1. **AgentTask skeleton** — turn cycle structure
2. **Migration** — port ~1000 lines from loop/mod.rs
3. **Event emission** — lifecycle events for spectator
4. **Flash integration** — spectator memories in context

Total: 4 tasks, ~20 steps. The old loop remains as fallback during transition.

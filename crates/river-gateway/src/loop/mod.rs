//! Agent loop module - the heart of the agent
//!
//! **DEPRECATED**: Use coordinator + agent task instead. See `agent/task.rs`.
//! This module is kept for backwards compatibility but will be removed in a future release.

#![deprecated(note = "Use coordinator + agent task instead. See agent/task.rs")]

pub mod state;
pub mod queue;
pub mod context;
pub mod model;
pub mod persistence;

pub use state::{LoopEvent, LoopState, WakeTrigger, ToolCallRequest, FunctionCall};
pub use queue::MessageQueue;
pub use context::{ChatMessage, ContextBuilder};
pub use model::{ModelClient, ModelResponse, Usage};
pub use persistence::ContextFile;

use crate::db::{Database, Message, MessageRole};
use crate::git::{GitOps, GitCommitResult};
use crate::metrics::{AgentMetrics, LoopStateLabel};
use crate::policy::{HealthPolicy, ModelErrorAction, compute_action_hash, ContextAction};
use crate::session::PRIMARY_SESSION_ID;
use crate::tools::{ContextRotation, HeartbeatScheduler, ToolExecutor, ToolCall};
use river_core::{RiverError, RiverResult, Snowflake, SnowflakeGenerator, SnowflakeType, ContextStatus};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, RwLock};
use chrono::Utc;

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
    /// Number of recent messages to load for conversation history
    pub history_message_limit: usize,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            default_heartbeat_minutes: 45,
            context_limit: 65536,
            model_timeout: Duration::from_secs(120),
            max_tool_calls_per_generation: 50,
            history_message_limit: 50, // Load last 50 messages for continuity
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
    snowflake_gen: Arc<SnowflakeGenerator>,
    heartbeat_scheduler: Arc<HeartbeatScheduler>,
    context_rotation: Arc<ContextRotation>,
    config: LoopConfig,
    shutdown_requested: bool,
    git: GitOps,
    /// System notifications to surface on next wake (git conflicts, etc.)
    pending_notifications: Vec<String>,
    /// Whether context needs to be rebuilt from scratch (first wake or after rotation)
    needs_context_reset: bool,
    /// Current context ID (snowflake)
    context_id: Option<Snowflake>,
    /// Context file for persistence
    context_file: Option<ContextFile>,
    /// Last known prompt token count
    last_prompt_tokens: u64,
    /// Shared metrics for observability
    metrics: Arc<RwLock<AgentMetrics>>,
    /// Self-healing health policy
    policy: Arc<RwLock<HealthPolicy>>,
    /// Total tool calls this turn
    turn_total_calls: u32,
    /// Failed tool calls this turn
    turn_failed_calls: u32,
}

impl AgentLoop {
    pub fn new(
        event_rx: mpsc::Receiver<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
        heartbeat_scheduler: Arc<HeartbeatScheduler>,
        context_rotation: Arc<ContextRotation>,
        config: LoopConfig,
        metrics: Arc<RwLock<AgentMetrics>>,
        policy: Arc<RwLock<HealthPolicy>>,
    ) -> Self {
        let git = GitOps::new(&config.workspace);
        Self {
            state: LoopState::Sleeping,
            event_rx,
            message_queue,
            model_client,
            context: ContextBuilder::new(),
            tool_executor,
            db,
            snowflake_gen,
            heartbeat_scheduler,
            context_rotation,
            shutdown_requested: false,
            git,
            config,
            pending_notifications: Vec::new(),
            needs_context_reset: true, // First wake needs full context build
            context_id: None,
            context_file: None,
            last_prompt_tokens: 0,
            metrics,
            policy,
            turn_total_calls: 0,
            turn_failed_calls: 0,
        }
    }

    /// Get a reference to the heartbeat scheduler for tools
    pub fn heartbeat_scheduler(&self) -> Arc<HeartbeatScheduler> {
        self.heartbeat_scheduler.clone()
    }

    /// Get a reference to the context rotation state for tools
    pub fn context_rotation(&self) -> Arc<ContextRotation> {
        self.context_rotation.clone()
    }

    /// Get current context status based on last known prompt tokens
    fn context_status(&self) -> ContextStatus {
        ContextStatus {
            used: self.last_prompt_tokens,
            limit: self.config.context_limit,
        }
    }

    /// Update shared metrics with current loop state
    async fn update_metrics_state(&self, new_state: LoopStateLabel) {
        let mut m = self.metrics.write().await;
        m.loop_state = new_state;
        match new_state {
            LoopStateLabel::Waking => {
                m.last_wake = Some(Utc::now());
            }
            LoopStateLabel::Settling => {
                m.last_settle = Some(Utc::now());
                m.turns_since_restart += 1;
            }
            _ => {}
        }
    }

    /// Update context metrics
    async fn update_metrics_context(&self) {
        let mut m = self.metrics.write().await;
        m.context_tokens = self.last_prompt_tokens;
        m.context_id = self.context_id.map(|id| id.to_string());
    }

    /// Initialize context persistence on startup
    fn initialize_context(&mut self) -> RiverResult<()> {
        let db = self.db.lock().map_err(|_| RiverError::database("Lock poisoned"))?;
        let latest = db.get_latest_context()?;
        drop(db);

        match latest {
            Some(ctx) if ctx.is_active() => {
                if ContextFile::exists(&self.config.workspace) {
                    tracing::info!(context_id = %ctx.id, "Resuming active context from file");
                    self.context_id = Some(ctx.id);
                    self.context_file = Some(ContextFile::open(&self.config.workspace)?);
                } else {
                    tracing::warn!(context_id = %ctx.id, "Active context but file missing - creating empty");
                    self.context_id = Some(ctx.id);
                    self.context_file = Some(ContextFile::create(&self.config.workspace)?);
                }
            }
            _ => {
                self.create_fresh_context()?;
            }
        }

        if self.context_id.is_none() && ContextFile::exists(&self.config.workspace) {
            tracing::warn!("Deleting orphan context file");
            ContextFile::delete(&self.config.workspace)?;
        }

        Ok(())
    }

    /// Create a fresh context with an empty file
    fn create_fresh_context(&mut self) -> RiverResult<()> {
        let id = self.snowflake_gen.next_id(SnowflakeType::Context);

        let db = self.db.lock().map_err(|_| RiverError::database("Lock poisoned"))?;
        db.insert_context(id)?;
        drop(db);

        self.context_id = Some(id);
        self.context_file = Some(ContextFile::create(&self.config.workspace)?);

        tracing::info!(context_id = %id, "Created fresh context");
        Ok(())
    }

    /// Create a new context with a summary as the initial system message
    fn create_context_with_summary(&mut self, summary: &str) -> RiverResult<()> {
        let id = self.snowflake_gen.next_id(SnowflakeType::Context);

        let db = self.db.lock().map_err(|_| RiverError::database("Lock poisoned"))?;
        db.insert_context(id)?;
        drop(db);

        self.context_id = Some(id);
        self.context_file = Some(ContextFile::create_with_summary(&self.config.workspace, summary)?);

        tracing::info!(context_id = %id, "Created context with summary");
        Ok(())
    }

    /// Archive the current context to the database
    fn archive_current_context(&mut self, summary: Option<&str>) -> RiverResult<()> {
        let context_id = self.context_id.ok_or_else(|| RiverError::session("No active context"))?;
        let context_file = self.context_file.as_ref().ok_or_else(|| RiverError::session("No context file"))?;

        let blob = context_file.read_raw()?;
        let archived_at = self.snowflake_gen.next_id(SnowflakeType::Context);

        let db = self.db.lock().map_err(|_| RiverError::database("Lock poisoned"))?;
        db.archive_context(context_id, archived_at, self.last_prompt_tokens as i64, summary, &blob)?;
        drop(db);

        tracing::info!(
            context_id = %context_id,
            token_count = self.last_prompt_tokens,
            has_summary = summary.is_some(),
            "Archived context"
        );

        Ok(())
    }

    /// Run the continuous loop
    pub async fn run(&mut self) {
        tracing::info!(
            workspace = %self.config.workspace.display(),
            context_limit = self.config.context_limit,
            model_timeout_secs = self.config.model_timeout.as_secs(),
            "Agent loop started"
        );

        // Initialize context persistence
        if let Err(e) = self.initialize_context() {
            tracing::error!(error = %e, "Failed to initialize context - continuing without persistence");
        }

        loop {
            tracing::debug!(state = ?self.state, "Loop iteration");
            match &self.state {
                LoopState::Sleeping => {
                    tracing::trace!("Entering sleep phase");
                    self.sleep_phase().await;
                }
                LoopState::Waking { .. } => {
                    tracing::trace!("Entering wake phase");
                    self.wake_phase().await;
                }
                LoopState::Thinking => {
                    tracing::trace!("Entering think phase");
                    self.think_phase().await;
                }
                LoopState::Acting { pending } => {
                    tracing::trace!(pending_tool_calls = pending.len(), "Entering act phase");
                    self.act_phase().await;
                }
                LoopState::Settling => {
                    tracing::trace!("Entering settle phase");
                    self.settle_phase().await;
                }
            }

            if self.shutdown_requested && self.state.is_sleeping() {
                tracing::info!("Shutdown requested and loop is sleeping, exiting");
                break;
            }
        }

        tracing::info!("Agent loop stopped");
    }

    async fn sleep_phase(&mut self) {
        let heartbeat_delay = self.heartbeat_scheduler.take_delay();

        tokio::select! {
            event = self.event_rx.recv() => {
                match event {
                    Some(LoopEvent::InboxUpdate(paths)) => {
                        tracing::info!("Wake: inbox update with {} files", paths.len());
                        self.update_metrics_state(LoopStateLabel::Waking).await;
                        self.state = LoopState::Waking {
                            trigger: WakeTrigger::Inbox(paths)
                        };
                    }
                    Some(LoopEvent::Heartbeat) => {
                        tracing::info!("Wake: heartbeat");
                        self.update_metrics_state(LoopStateLabel::Waking).await;
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
                self.update_metrics_state(LoopStateLabel::Waking).await;
                self.state = LoopState::Waking {
                    trigger: WakeTrigger::Heartbeat
                };
            }
        }
    }

    async fn wake_phase(&mut self) {
        // Extract the trigger, replacing state with Sleeping temporarily
        let trigger = match std::mem::replace(&mut self.state, LoopState::Sleeping) {
            LoopState::Waking { trigger } => trigger,
            _ => {
                tracing::error!("Invalid state in wake_phase");
                return;
            }
        };

        // Check if ATTENTION.md was cleared and handle any human response
        {
            let mut policy = self.policy.write().await;
            if let Some(human_response) = policy.check_attention_cleared() {
                tracing::info!(response_len = human_response.len(), "Human responded to ATTENTION.md");
                self.pending_notifications.push(format!(
                    "HUMAN RESPONSE (from ATTENTION.md):\n{}",
                    human_response
                ));
            }
        }

        // Drain any messages that arrived before we woke
        let queued_messages = self.message_queue.drain();
        if !queued_messages.is_empty() {
            tracing::info!("Processing {} queued messages", queued_messages.len());
        }

        // Update policy with turn start state
        let is_heartbeat = matches!(trigger, WakeTrigger::Heartbeat);
        {
            let mut policy = self.policy.write().await;
            policy.set_pending_messages(queued_messages.len() as u32);
            policy.set_turn_start_tokens(self.last_prompt_tokens);
            policy.set_heartbeat_turn(is_heartbeat);
        }

        // Reset turn counters
        self.turn_total_calls = 0;
        self.turn_failed_calls = 0;

        // Only rebuild context from scratch on first wake or after rotation
        if self.needs_context_reset {
            tracing::info!("Building fresh context (first wake or post-rotation)");
            self.context.clear();
            self.context.assemble(
                &self.config.workspace,
                trigger,
                queued_messages.clone(),
            ).await;

            // Load context from file if exists (for resuming after restart)
            if let Some(ref file) = self.context_file {
                match file.load() {
                    Ok(messages) => {
                        let count = messages.len();
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

            // Persist any queued messages that arrived before we woke
            for msg in queued_messages {
                let chat_msg = ChatMessage::user(format!(
                    "[{}] {}: {}",
                    msg.channel, msg.author.name, msg.content
                ));
                // Already added via assemble(), just persist to file
                if let Some(ref file) = self.context_file {
                    if let Err(e) = file.append(&chat_msg) {
                        tracing::error!(error = %e, "Failed to append queued message to context file");
                    }
                }
            }

            self.needs_context_reset = false;
        } else {
            // Accumulating context - just add the new trigger and messages
            tracing::debug!("Adding to existing context (accumulating)");

            // Add queued messages first
            for msg in queued_messages {
                let chat_msg = ChatMessage::user(format!(
                    "[{}] {}: {}",
                    msg.channel, msg.author.name, msg.content
                ));
                self.context.add_message(chat_msg.clone());

                // Persist to context file
                if let Some(ref file) = self.context_file {
                    if let Err(e) = file.append(&chat_msg) {
                        tracing::error!(error = %e, "Failed to append queued message to context file");
                    }
                }
            }

            // Add wake trigger
            match &trigger {
                WakeTrigger::Inbox(paths) => {
                    // Notify agent of inbox files with new messages - agent decides how to process
                    let file_list: Vec<String> = paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect();
                    let chat_msg = ChatMessage::user(format!(
                        "New messages in inbox files:\n{}",
                        file_list.join("\n")
                    ));
                    self.context.add_message(chat_msg);
                    tracing::info!(files = ?file_list, "Notified agent of inbox updates");
                }
                WakeTrigger::Heartbeat => {
                    // Heartbeat messages are NOT persisted - they're transient user prompts
                    self.context.add_message(ChatMessage::user(":heartbeat:"));
                }
            }
        }

        // Inject 80% context warning if needed
        let context_percent = (self.last_prompt_tokens as f64 / self.config.context_limit as f64) * 100.0;
        if context_percent >= 80.0 && context_percent < 90.0 {
            tracing::warn!(
                event = "context.warning",
                usage_percent = format!("{:.1}", context_percent),
                threshold = 80,
                "Context usage high"
            );
            self.context.add_message(ChatMessage::system(format!(
                "WARNING: Context at {:.1}%. Consider summarizing and calling rotate_context soon.",
                context_percent
            )));
        }

        // Add any pending system notifications (git conflicts, etc.)
        if !self.pending_notifications.is_empty() {
            let notifications = std::mem::take(&mut self.pending_notifications);
            let notification_text = format!(
                "SYSTEM NOTIFICATIONS:\n{}",
                notifications.iter()
                    .map(|n| format!("- {}", n))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            self.context.add_message(ChatMessage::system(notification_text));
            tracing::info!("Surfaced {} system notification(s) to agent", notifications.len());
        }

        // Load tool schemas (in case they changed)
        let executor = self.tool_executor.read().await;
        self.context.set_tools(executor.schemas());

        self.update_metrics_state(LoopStateLabel::Thinking).await;
        self.state = LoopState::Thinking;
    }

    async fn think_phase(&mut self) {
        // Check error backoff before model call
        let backoff = {
            let policy = self.policy.read().await;
            policy.error_backoff()
        };
        if !backoff.is_zero() {
            tracing::warn!(
                backoff_secs = backoff.as_secs(),
                "Error backoff active - waiting before model call"
            );
            tokio::time::sleep(backoff).await;
        }

        // Increment model_calls counter
        {
            let mut m = self.metrics.write().await;
            m.model_calls += 1;
        }

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

        let message_count = self.context.messages().len();
        let tool_count = self.context.tools().len();
        tracing::info!(
            event = "loop.think",
            message_count = message_count,
            tool_count = tool_count,
            "Calling model"
        );

        let response = match self.model_client.complete(
            self.context.messages(),
            self.context.tools(),
        ).await {
            Ok(resp) => resp,
            Err(RiverError::ModelApi { status, message }) => {
                tracing::error!(
                    status = status,
                    message = %message,
                    "Model API error - consulting policy"
                );
                let action = {
                    let mut policy = self.policy.write().await;
                    policy.on_model_error(status)
                };
                match action {
                    ModelErrorAction::RetryAfter(duration) => {
                        tracing::info!(
                            retry_after_secs = duration.as_secs(),
                            "Rate limited - waiting before retry"
                        );
                        tokio::time::sleep(duration).await;
                        // Stay in Thinking state to retry
                        return;
                    }
                    ModelErrorAction::RetryWithBackoff(duration) => {
                        tracing::warn!(
                            backoff_secs = duration.as_secs(),
                            "Server error - will retry with backoff"
                        );
                        // Backoff is applied at start of next think_phase
                        self.state = LoopState::Settling;
                        return;
                    }
                    ModelErrorAction::NoRetry => {
                        tracing::error!("Client error - not retrying");
                        self.state = LoopState::Settling;
                        return;
                    }
                    ModelErrorAction::Escalated => {
                        tracing::error!("Auth error (401/403) - escalated to NeedsAttention");
                        self.state = LoopState::Settling;
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Model call failed - transitioning to Settling"
                );
                self.state = LoopState::Settling;
                return;
            }
        };

        tracing::info!(
            event = "loop.response",
            tokens_total = response.usage.total_tokens,
            tokens_prompt = response.usage.prompt_tokens,
            tokens_completion = response.usage.completion_tokens,
            tool_calls = response.tool_calls.len(),
            has_content = response.content.is_some(),
            "Model response received"
        );

        // Track token count for persistence
        self.last_prompt_tokens = response.usage.prompt_tokens as u64;

        // Update context metrics
        self.update_metrics_context().await;

        // Check for 90% auto-rotation
        let context_percent = (self.last_prompt_tokens as f64 / self.config.context_limit as f64) * 100.0;
        if context_percent >= 90.0 {
            tracing::warn!(percent = format!("{:.1}", context_percent), "Context at 90%+ - triggering auto-rotation");
            self.context_rotation.request_auto();
        }

        // Add assistant message to context
        self.context.add_assistant_response(
            response.content.clone(),
            if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            },
        );

        // Append assistant message to context file
        if let Some(ref file) = self.context_file {
            let msg = ChatMessage::assistant(
                response.content.clone(),
                if response.tool_calls.is_empty() { None } else { Some(response.tool_calls.clone()) },
            );
            if let Err(e) = file.append(&msg) {
                tracing::error!(error = %e, "Failed to append assistant message to context file");
            }
        }


        if response.tool_calls.is_empty() {
            // No tool calls - cycle complete
            if let Some(content) = &response.content {
                tracing::info!(
                    content_len = content.len(),
                    content_preview = %content.chars().take(300).collect::<String>(),
                    "Assistant response (no tool calls) - transitioning to Settling"
                );
            } else {
                tracing::info!("No content and no tool calls - transitioning to Settling");
            }
            self.update_metrics_state(LoopStateLabel::Settling).await;
            self.state = LoopState::Settling;
        } else {
            // Has tool calls - check for repeated action (stuck detection)
            // Convert ToolCallRequest to ToolCall for hashing
            let tool_calls_for_hash: Vec<ToolCall> = response.tool_calls.iter().map(|tc| {
                ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Null),
                }
            }).collect();
            let action_hash = compute_action_hash(&tool_calls_for_hash);
            {
                let mut policy = self.policy.write().await;
                policy.check_repeated_action(action_hash);
            }

            // Has tool calls - execute them
            tracing::info!(
                tool_call_count = response.tool_calls.len(),
                tools = ?response.tool_calls.iter().map(|t| &t.function.name).collect::<Vec<_>>(),
                "Transitioning to Acting phase"
            );
            self.update_metrics_state(LoopStateLabel::Acting).await;
            self.state = LoopState::Acting {
                pending: response.tool_calls,
            };
        }
    }

    async fn act_phase(&mut self) {
        // Extract pending tool calls from state
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

        // Convert to executor format and execute
        let mut results = Vec::new();
        {
            let mut executor = self.tool_executor.write().await;
            for (i, tc) in tool_calls.iter().enumerate() {
                // Check tool backoff before execution
                let tool_backoff = {
                    let policy = self.policy.read().await;
                    policy.tool_backoff(&tc.function.name)
                };
                if !tool_backoff.is_zero() {
                    tracing::warn!(
                        tool = %tc.function.name,
                        backoff_secs = tool_backoff.as_secs(),
                        "Tool backoff active - waiting before execution"
                    );
                    tokio::time::sleep(tool_backoff).await;
                }

                tracing::info!(
                    index = i,
                    tool = %tc.function.name,
                    call_id = %tc.id,
                    args_raw = %tc.function.arguments,
                    "Processing tool call"
                );

                let arguments = match serde_json::from_str(&tc.function.arguments) {
                    Ok(args) => {
                        tracing::debug!(
                            tool = %tc.function.name,
                            args_parsed = %serde_json::to_string(&args).unwrap_or_default(),
                            "Arguments parsed successfully"
                        );
                        args
                    }
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
                let start_time = std::time::Instant::now();
                let result = executor.execute(&call);
                let duration = start_time.elapsed();
                let success = result.result.is_ok();

                // Update turn counters
                self.turn_total_calls += 1;
                if !success {
                    self.turn_failed_calls += 1;
                }

                // Update policy with tool result
                {
                    let mut policy = self.policy.write().await;
                    policy.on_tool_result(&tc.function.name, success, duration);
                }

                tracing::info!(
                    event = "loop.tool",
                    tool_name = %tc.function.name,
                    call_id = %tc.id,
                    success = success,
                    duration_ms = duration.as_millis(),
                    "Tool execution complete"
                );
                results.push(result);
            }
        }

        // Drain any messages that arrived during tool execution
        let incoming_messages = self.message_queue.drain();
        if !incoming_messages.is_empty() {
            tracing::info!("{} messages arrived during tool execution", incoming_messages.len());
        }

        // Append tool results to context file
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

        // Add tool results and incoming messages to context
        self.context.add_tool_results(results, incoming_messages);

        // Check if context rotation was requested
        if self.context_rotation.is_requested() {
            self.state = LoopState::Settling;
        } else {
            // Back to thinking
            self.state = LoopState::Thinking;
        }
    }

    /// Persist conversation messages to database
    fn persist_messages(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let db = match self.db.lock() {
            Ok(db) => db,
            Err(e) => {
                tracing::error!("Failed to acquire database lock: {}", e);
                return;
            }
        };

        let mut persisted = 0;
        for chat_msg in self.context.messages() {
            // Skip system messages - they're context assembly, not conversation
            if chat_msg.role == "system" {
                continue;
            }

            // Convert role string to MessageRole enum
            let role = match chat_msg.role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "tool" => MessageRole::Tool,
                _ => continue, // Skip unknown roles
            };

            // Serialize tool_calls to JSON if present
            let tool_calls_json = chat_msg.tool_calls.as_ref().map(|tc| {
                serde_json::to_string(tc).unwrap_or_default()
            });

            let msg = Message {
                id: self.snowflake_gen.next_id(SnowflakeType::Message),
                session_id: PRIMARY_SESSION_ID.to_string(),
                role,
                content: chat_msg.content.clone(),
                tool_calls: tool_calls_json,
                tool_call_id: chat_msg.tool_call_id.clone(),
                name: chat_msg.name.clone(),
                created_at: now,
                metadata: None,
            };

            if let Err(e) = db.insert_message(&msg) {
                tracing::warn!("Failed to persist message: {}", e);
            } else {
                persisted += 1;
            }
        }

        if persisted > 0 {
            tracing::debug!("Persisted {} messages to database", persisted);
        }
    }

    async fn settle_phase(&mut self) {
        self.update_metrics_state(LoopStateLabel::Settling).await;
        tracing::info!(event = "loop.settle", "Turn complete, settling");

        // Record turn tokens and notify policy of turn completion
        {
            let mut policy = self.policy.write().await;
            // Record token progress for stuck detection
            let turn_start_tokens = policy.context_tokens_at_turn_start;
            policy.record_turn_tokens(turn_start_tokens, self.last_prompt_tokens);
            // Notify policy of turn completion with call counts
            policy.on_turn_complete(self.turn_total_calls, self.turn_failed_calls);
            tracing::debug!(
                total_calls = self.turn_total_calls,
                failed_calls = self.turn_failed_calls,
                "Turn stats recorded"
            );
        }

        // Check for proactive context rotation
        let context_percent = (self.last_prompt_tokens as f64 / self.config.context_limit as f64) * 100.0;
        let context_action = {
            let policy = self.policy.read().await;
            policy.on_context_warning(context_percent)
        };
        if context_action == ContextAction::RotateNow && !self.context_rotation.is_requested() {
            tracing::info!(
                context_percent = format!("{:.1}", context_percent),
                "Policy recommends proactive context rotation"
            );
            self.context_rotation.request_auto();
        }

        // Handle context rotation if requested
        if let Some(summary_opt) = self.context_rotation.take_request() {
            tracing::info!(
                event = "context.rotate",
                has_summary = summary_opt.is_some(),
                old_tokens = self.last_prompt_tokens,
                "Processing context rotation"
            );

            if let Err(e) = self.archive_current_context(summary_opt.as_deref()) {
                tracing::error!(error = %e, "Failed to archive context");
            } else {
                let result = if let Some(ref s) = summary_opt {
                    self.create_context_with_summary(s)
                } else {
                    tracing::warn!("Auto-rotation with no summary - continuity lost");
                    self.create_fresh_context()
                };

                if let Err(e) = result {
                    tracing::error!(error = %e, "Failed to create new context");
                } else {
                    // Increment rotation counter
                    {
                        let mut m = self.metrics.write().await;
                        m.rotations_since_restart += 1;
                    }
                }

                self.needs_context_reset = true;
                self.last_prompt_tokens = 0;
            }
        }

        // Persist conversation messages to database
        self.persist_messages();

        // Git commit if workspace changed
        if self.git.is_git_repo() {
            match self.git.commit_if_changed() {
                GitCommitResult::NoChanges => {
                    tracing::debug!("No workspace changes to commit");
                }
                GitCommitResult::Committed { files, commit_hash } => {
                    tracing::info!(
                        "Committed {} file(s) as {} ({})",
                        files.len(),
                        commit_hash,
                        files.join(", ")
                    );
                }
                GitCommitResult::Conflicts { conflicting_files } => {
                    // Conflicts are reported but don't stop the loop
                    tracing::warn!(
                        "Git conflicts detected in {} file(s): {}. Agent should resolve manually.",
                        conflicting_files.len(),
                        conflicting_files.join(", ")
                    );
                    // Surface to agent on next wake
                    self.pending_notifications.push(format!(
                        "Git conflict detected: The following files have merge conflicts that need resolution: {}",
                        conflicting_files.join(", ")
                    ));
                }
                GitCommitResult::Error(e) => {
                    tracing::warn!("Git commit failed: {}", e);
                }
            }
        }

        // Messages now go to inbox files, so we just go to sleep.
        // Any new messages will trigger InboxUpdate events.
        self.update_metrics_state(LoopStateLabel::Sleeping).await;
        tracing::debug!(event = "loop.sleep", "Entering sleep");
        self.state = LoopState::Sleeping;
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

    #[test]
    fn test_loop_config_workspace_default() {
        let config = LoopConfig::default();
        assert_eq!(config.workspace, PathBuf::from("."));
    }

    #[test]
    fn test_loop_config_timeout_default() {
        let config = LoopConfig::default();
        assert_eq!(config.model_timeout, Duration::from_secs(120));
    }

    #[test]
    fn test_loop_config_custom() {
        let config = LoopConfig {
            workspace: PathBuf::from("/home/agent/workspace"),
            default_heartbeat_minutes: 30,
            context_limit: 128000,
            model_timeout: Duration::from_secs(300),
            max_tool_calls_per_generation: 100,
            history_message_limit: 100,
        };
        assert_eq!(config.workspace, PathBuf::from("/home/agent/workspace"));
        assert_eq!(config.default_heartbeat_minutes, 30);
        assert_eq!(config.context_limit, 128000);
        assert_eq!(config.model_timeout, Duration::from_secs(300));
        assert_eq!(config.max_tool_calls_per_generation, 100);
        assert_eq!(config.history_message_limit, 100);
    }

    #[test]
    fn test_loop_config_clone() {
        let config = LoopConfig::default();
        let cloned = config.clone();
        assert_eq!(config.context_limit, cloned.context_limit);
        assert_eq!(config.workspace, cloned.workspace);
    }

    #[test]
    fn test_loop_config_debug() {
        let config = LoopConfig::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("LoopConfig"));
        assert!(debug.contains("workspace"));
        assert!(debug.contains("context_limit"));
    }
}

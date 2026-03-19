//! Agent loop module - the heart of the agent

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
use crate::session::PRIMARY_SESSION_ID;
use crate::tools::{ContextRotation, HeartbeatScheduler, ToolExecutor, ToolCall};
use river_core::{SnowflakeGenerator, SnowflakeType};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
}

impl AgentLoop {
    pub fn new(
        event_rx: mpsc::Receiver<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
        config: LoopConfig,
    ) -> Self {
        let git = GitOps::new(&config.workspace);
        let heartbeat_scheduler = Arc::new(HeartbeatScheduler::new(config.default_heartbeat_minutes));
        let context_rotation = Arc::new(ContextRotation::new());
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

    /// Run the continuous loop
    pub async fn run(&mut self) {
        tracing::info!(
            workspace = %self.config.workspace.display(),
            context_limit = self.config.context_limit,
            model_timeout_secs = self.config.model_timeout.as_secs(),
            "Agent loop started"
        );

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
        // Extract the trigger, replacing state with Sleeping temporarily
        let trigger = match std::mem::replace(&mut self.state, LoopState::Sleeping) {
            LoopState::Waking { trigger } => trigger,
            _ => {
                tracing::error!("Invalid state in wake_phase");
                return;
            }
        };

        // Drain any messages that arrived before we woke
        let queued_messages = self.message_queue.drain();
        if !queued_messages.is_empty() {
            tracing::info!("Processing {} queued messages", queued_messages.len());
        }

        // Only rebuild context from scratch on first wake or after rotation
        if self.needs_context_reset {
            tracing::info!("Building fresh context (first wake or post-rotation)");
            self.context.clear();
            self.context.assemble(
                &self.config.workspace,
                trigger,
                queued_messages,
            ).await;

            // Load conversation history from database for continuity
            self.load_conversation_history();

            // Reset the executor's context tracking
            {
                let mut executor = self.tool_executor.write().await;
                executor.reset_context();
            }

            self.needs_context_reset = false;
        } else {
            // Accumulating context - just add the new trigger and messages
            tracing::debug!("Adding to existing context (accumulating)");

            // Add queued messages first
            for msg in queued_messages {
                self.context.add_message(ChatMessage::user(format!(
                    "[{}] {}: {}",
                    msg.channel, msg.author.name, msg.content
                )));
            }

            // Add wake trigger
            match &trigger {
                WakeTrigger::Message(msg) => {
                    self.context.add_message(ChatMessage::user(format!(
                        "[{}] {}: {}",
                        msg.channel, msg.author.name, msg.content
                    )));
                }
                WakeTrigger::Heartbeat => {
                    self.context.add_message(ChatMessage::system(
                        "Heartbeat wake. No new messages. Check on your tasks and state.".to_string()
                    ));
                }
            }
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

        self.state = LoopState::Thinking;
    }

    async fn think_phase(&mut self) {
        let message_count = self.context.messages().len();
        let tool_count = self.context.tools().len();
        tracing::info!(
            message_count = message_count,
            tool_count = tool_count,
            "Think phase: calling model"
        );

        let response = match self.model_client.complete(
            self.context.messages(),
            self.context.tools(),
        ).await {
            Ok(resp) => resp,
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
            tokens_total = response.usage.total_tokens,
            tokens_prompt = response.usage.prompt_tokens,
            tokens_completion = response.usage.completion_tokens,
            tool_calls = response.tool_calls.len(),
            has_content = response.content.is_some(),
            "Model response received"
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
            let status = executor.context_status();
            tracing::debug!(
                context_used = status.used,
                context_limit = status.limit,
                context_percent = format!("{:.1}%", status.percent()),
                "Context usage updated"
            );
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
            self.state = LoopState::Settling;
        } else {
            // Has tool calls - execute them
            tracing::info!(
                tool_call_count = response.tool_calls.len(),
                tools = ?response.tool_calls.iter().map(|t| &t.function.name).collect::<Vec<_>>(),
                "Transitioning to Acting phase"
            );
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

        // Add tool results and incoming messages to context
        self.context.add_tool_results(results, incoming_messages, context_status.clone());

        // Check if context rotation was requested manually
        if let Some(summary_opt) = self.context_rotation.take_request() {
            match &summary_opt {
                Some(summary) => {
                    tracing::info!("Context rotation requested with summary: {}", summary);
                }
                None => {
                    tracing::info!("Context rotation requested (auto-rotation, no summary)");
                }
            }
            // Mark for context reset on next wake
            self.needs_context_reset = true;
            self.state = LoopState::Settling;
        } else if context_status.is_near_limit() {
            // Automatic rotation at 90% per spec Section 3.7
            // Penalty: Agent must recover state from workspace files and memory search
            tracing::warn!(
                "AUTOMATIC CONTEXT ROTATION: {:.1}% of limit reached ({}k/{}k tokens). \
                Session will reset. Agent must recover from workspace/memory.",
                context_status.percent(),
                context_status.used / 1000,
                context_status.limit / 1000
            );
            // Mark for context reset on next wake
            self.needs_context_reset = true;
            self.state = LoopState::Settling;
        } else {
            // Back to thinking
            self.state = LoopState::Thinking;
        }
    }

    /// Load recent conversation history from database into context
    fn load_conversation_history(&mut self) {
        if self.config.history_message_limit == 0 {
            return;
        }

        let messages = {
            let db = match self.db.lock() {
                Ok(db) => db,
                Err(e) => {
                    tracing::error!("Failed to acquire database lock for history: {}", e);
                    return;
                }
            };

            match db.get_session_messages(PRIMARY_SESSION_ID, self.config.history_message_limit) {
                Ok(msgs) => msgs,
                Err(e) => {
                    tracing::warn!("Failed to load conversation history: {}", e);
                    return;
                }
            }
        };

        if messages.is_empty() {
            tracing::debug!("No conversation history to load");
            return;
        }

        tracing::info!("Loading {} messages from conversation history", messages.len());

        // Add a marker for history section
        self.context.add_message(ChatMessage::system(
            "--- Previous conversation history ---".to_string()
        ));

        // Convert database messages to chat messages
        for msg in messages {
            let chat_msg = match msg.role {
                MessageRole::User => {
                    ChatMessage::user(msg.content.unwrap_or_default())
                }
                MessageRole::Assistant => {
                    // Reconstruct tool_calls if present
                    let tool_calls = msg.tool_calls.as_ref().and_then(|tc_json| {
                        serde_json::from_str(tc_json).ok()
                    });
                    ChatMessage::assistant(msg.content, tool_calls)
                }
                MessageRole::Tool => {
                    ChatMessage::tool(
                        msg.tool_call_id.unwrap_or_default(),
                        msg.content.unwrap_or_default()
                    )
                }
                MessageRole::System => {
                    // Skip system messages from history - they're context-specific
                    continue;
                }
            };
            self.context.add_message(chat_msg);
        }

        // Add end marker
        self.context.add_message(ChatMessage::system(
            "--- End of conversation history ---".to_string()
        ));
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
        tracing::debug!("Settling...");

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

        // Check if messages arrived during settle
        // Note: drain() is atomic with the queue, so we don't need a separate is_empty() check
        let messages = self.message_queue.drain();
        if let Some(msg) = messages.into_iter().next() {
            tracing::info!("Message arrived during settle, immediate wake");
            self.state = LoopState::Waking {
                trigger: WakeTrigger::Message(msg),
            };
        } else {
            self.state = LoopState::Sleeping;
        }
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

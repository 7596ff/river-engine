//! Agent loop module - the heart of the agent

pub mod state;
pub mod queue;
pub mod context;
pub mod model;

pub use state::{LoopEvent, LoopState, WakeTrigger};
pub use queue::MessageQueue;
pub use context::{ChatMessage, ContextBuilder, ToolCallRequest, FunctionCall};
pub use model::{ModelClient, ModelResponse, Usage};

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
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            default_heartbeat_minutes: 45,
            context_limit: 65536,
            model_timeout: Duration::from_secs(120),
            max_tool_calls_per_generation: 50,
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
    pending_tool_calls: Vec<ToolCallRequest>,
    shutdown_requested: bool,
    git: GitOps,
    /// System notifications to surface on next wake (git conflicts, etc.)
    pending_notifications: Vec<String>,
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
            pending_tool_calls: Vec::new(),
            shutdown_requested: false,
            git,
            config,
            pending_notifications: Vec::new(),
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
        tracing::info!("Agent loop started");

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
                LoopState::Acting => {
                    self.act_phase().await;
                }
                LoopState::Settling => {
                    self.settle_phase().await;
                }
            }

            if self.shutdown_requested && self.state.is_sleeping() {
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

        // Assemble context
        self.context.clear();
        self.context.assemble(
            &self.config.workspace,
            trigger,
            queued_messages,
        ).await;

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

        // Load tool schemas
        let executor = self.tool_executor.read().await;
        self.context.set_tools(executor.schemas());

        self.state = LoopState::Thinking;
    }

    async fn think_phase(&mut self) {
        tracing::debug!("Calling model...");

        let response = match self.model_client.complete(
            self.context.messages(),
            self.context.tools(),
        ).await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::error!("Model call failed: {}", e);
                self.state = LoopState::Settling;
                return;
            }
        };

        tracing::debug!(
            "Model response: {} tokens, {} tool calls",
            response.usage.total_tokens,
            response.tool_calls.len()
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
        }

        if response.tool_calls.is_empty() {
            // No tool calls - cycle complete
            if let Some(content) = &response.content {
                tracing::debug!("Assistant said: {}", content);
            }
            self.state = LoopState::Settling;
        } else {
            // Has tool calls - execute them
            self.pending_tool_calls = response.tool_calls;
            self.state = LoopState::Acting;
        }
    }

    async fn act_phase(&mut self) {
        let tool_calls = std::mem::take(&mut self.pending_tool_calls);
        tracing::debug!("Executing {} tool calls", tool_calls.len());

        // Convert to executor format and execute
        let mut results = Vec::new();
        {
            let mut executor = self.tool_executor.write().await;
            for tc in &tool_calls {
                let arguments = match serde_json::from_str(&tc.function.arguments) {
                    Ok(args) => args,
                    Err(e) => {
                        tracing::warn!(
                            "Invalid JSON arguments for tool {}: {} - using empty object",
                            tc.function.name, e
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
                tracing::debug!("Tool {}: {:?}", tc.function.name, result.result.is_ok());
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
        if let Some(reason) = self.context_rotation.take_request() {
            if reason.is_empty() {
                tracing::info!("Context rotation requested (no reason given)");
            } else {
                tracing::info!("Context rotation requested: {}", reason);
            }
            // Go to settling, which will clear context on next wake
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
}

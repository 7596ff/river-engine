//! Agent task — the acting self (I)
//!
//! Runs as a peer task in the coordinator, managing the wake/think/act/settle
//! turn cycle. Receives events from the coordinator bus and emits lifecycle
//! events for the spectator to observe.

use crate::agent::channel::ChannelContext;
use crate::agent::context::{ContextAssembler, ContextBudget};
use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::flash::{Flash, FlashQueue, FlashTTL};
use crate::preferences::{Preferences, format_current_time};
use crate::r#loop::{MessageQueue, ModelClient};
use crate::r#loop::context::ChatMessage;
use crate::r#loop::state::ToolCallRequest;
use crate::tools::{ToolExecutor, ToolCall, ToolSchema};
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Configuration for the agent task
#[derive(Debug, Clone)]
pub struct AgentTaskConfig {
    /// Workspace path for loading identity and context files
    pub workspace: PathBuf,
    /// Directory containing embeddings/notes
    pub embeddings_dir: PathBuf,
    /// Token budget allocation for context layers
    pub context_budget: ContextBudget,
    /// Timeout for model calls
    pub model_timeout: Duration,
    /// Maximum tool calls per turn (safety limit)
    pub max_tool_calls: usize,
    /// Number of recent messages to load for hot context
    pub history_limit: usize,
    /// Heartbeat interval (how often to wake if no messages)
    pub heartbeat_interval: Duration,
    /// Context limit in tokens (for rotation checks)
    pub context_limit: u64,
}

impl Default for AgentTaskConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            embeddings_dir: PathBuf::from("embeddings"),
            context_budget: ContextBudget::default(),
            model_timeout: Duration::from_secs(120),
            max_tool_calls: 50,
            history_limit: 50,
            heartbeat_interval: Duration::from_secs(45 * 60),
            context_limit: 128_000,
        }
    }
}

/// Turn statistics for tracking
#[derive(Debug, Default)]
pub struct TurnStats {
    pub tool_calls: Vec<String>,
    pub total_tool_calls: u32,
    pub failed_tool_calls: u32,
    pub prompt_tokens: u64,
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
    turn_count: u64,
    channel_context: Option<ChannelContext>,
    /// Conversation messages for context (accumulated across turns)
    conversation: Vec<ChatMessage>,
    /// Last known prompt token count
    last_prompt_tokens: u64,
}

impl AgentTask {
    pub fn new(
        config: AgentTaskConfig,
        bus: EventBus,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        flash_queue: Arc<FlashQueue>,
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
            turn_count: 0,
            channel_context: None,
            conversation: Vec::new(),
            last_prompt_tokens: 0,
        }
    }

    /// Main run loop — called by coordinator via spawn_task
    pub async fn run(mut self) {
        let mut event_rx = self.bus.subscribe();

        tracing::info!("Agent task started");

        loop {
            tokio::select! {
                // Wait for messages or heartbeat timeout
                _ = tokio::time::sleep(self.config.heartbeat_interval) => {
                    // Heartbeat wake - always run a turn (agent can decide what to do)
                    tracing::info!("Heartbeat wake");
                    self.turn_cycle(true).await;
                }
                // Check for new messages periodically
                _ = self.wait_for_messages() => {
                    self.turn_cycle(false).await;
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

    /// Wait until there are messages in the queue
    async fn wait_for_messages(&self) {
        loop {
            if !self.message_queue.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// One turn: wake → think → act → settle
    async fn turn_cycle(&mut self, is_heartbeat: bool) {
        self.turn_count += 1;
        let turn_start = Utc::now();
        let mut stats = TurnStats::default();

        // ========== WAKE ==========
        self.flash_queue.tick_turn().await;
        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: self.channel_context
                .as_ref()
                .map(|c| c.display_name().to_string())
                .unwrap_or_else(|| "unset".to_string()),
            turn_number: self.turn_count,
            timestamp: turn_start,
        }));

        tracing::info!(
            turn = self.turn_count,
            channel = %self.channel_context
                .as_ref()
                .map(|c| c.display_name())
                .unwrap_or("unset"),
            is_heartbeat = is_heartbeat,
            "Turn started"
        );

        // Drain incoming messages
        let incoming = self.message_queue.drain();
        for msg in &incoming {
            let chat_msg = ChatMessage::user(format!(
                "[{}] {}: {}",
                msg.channel, msg.author.name, msg.content
            ));
            self.conversation.push(chat_msg);
        }

        // Add heartbeat trigger if applicable
        if is_heartbeat && incoming.is_empty() {
            self.conversation.push(ChatMessage::user(":heartbeat:"));
        }

        // ========== ASSEMBLE CONTEXT ==========
        let system_prompt = self.build_system_prompt().await;
        let channel_name = self.channel_context
            .as_ref()
            .map(|c| c.display_name().to_string())
            .unwrap_or_else(|| "default".to_string());
        let context = self.context_assembler.assemble(
            &channel_name,
            &system_prompt,
            &self.conversation,
            &self.flash_queue,
            None,  // vector store - TODO: integrate
            None,  // query embedding - TODO: integrate
        ).await;

        tracing::info!(
            turn = self.turn_count,
            tokens = context.token_estimate,
            flashes = context.layer_stats.flashes_count,
            hot_messages = context.layer_stats.hot_messages,
            "Context assembled"
        );

        // Check for context pressure
        let context_percent = (context.token_estimate as f64 / self.config.context_limit as f64) * 100.0;
        if context_percent >= 80.0 {
            self.bus.publish(CoordinatorEvent::Agent(AgentEvent::ContextPressure {
                usage_percent: context_percent,
                timestamp: Utc::now(),
            }));
            tracing::warn!(
                usage_percent = format!("{:.1}", context_percent),
                "Context pressure high"
            );
        }

        // Get tool schemas
        let tools: Vec<ToolSchema> = {
            let executor = self.tool_executor.read().await;
            executor.schemas()
        };

        // ========== THINK + ACT LOOP ==========
        let mut messages = context.messages;
        let mut iteration = 0;
        let max_iterations = self.config.max_tool_calls;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                tracing::warn!(
                    iterations = iteration,
                    max = max_iterations,
                    "Max tool call iterations reached, breaking"
                );
                break;
            }

            // Call model
            let response = match self.model_client.complete(&messages, &tools).await {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::error!(error = %e, "Model call failed");
                    break;
                }
            };

            stats.prompt_tokens = response.usage.prompt_tokens as u64;
            self.last_prompt_tokens = stats.prompt_tokens;

            tracing::info!(
                iteration = iteration,
                prompt_tokens = response.usage.prompt_tokens,
                completion_tokens = response.usage.completion_tokens,
                tool_calls = response.tool_calls.len(),
                has_content = response.content.is_some(),
                "Model response"
            );

            // Add assistant response to conversation
            let assistant_msg = ChatMessage::assistant(
                response.content.clone(),
                if response.tool_calls.is_empty() { None } else { Some(response.tool_calls.clone()) },
            );
            messages.push(assistant_msg.clone());
            self.conversation.push(assistant_msg);

            // If no tool calls, we're done
            if response.tool_calls.is_empty() {
                if let Some(ref content) = response.content {
                    tracing::info!(
                        content_len = content.len(),
                        "Assistant response (no tool calls)"
                    );
                }
                break;
            }

            // ========== ACT: Execute tool calls ==========
            let tool_results = self.execute_tool_calls(&response.tool_calls, &mut stats).await;

            // Add tool results to conversation
            for result in &tool_results {
                let tool_msg = ChatMessage::tool(&result.0, &result.1);
                messages.push(tool_msg.clone());
                self.conversation.push(tool_msg);
            }

            // Check for messages that arrived during tool execution
            let mid_turn_messages = self.message_queue.drain();
            if !mid_turn_messages.is_empty() {
                let mut content = String::from("Messages received during tool execution:\n");
                for msg in mid_turn_messages {
                    content.push_str(&format!(
                        "- [{}] {}: {}\n",
                        msg.channel, msg.author.name, msg.content
                    ));
                }
                let system_msg = ChatMessage::system(content);
                messages.push(system_msg.clone());
                self.conversation.push(system_msg);
            }
        }

        // ========== SETTLE ==========
        let transcript_summary = format!(
            "Turn {} completed: {} messages, {} tool calls ({} failed)",
            self.turn_count,
            incoming.len(),
            stats.total_tool_calls,
            stats.failed_tool_calls
        );

        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
            channel: self.channel_context
                .as_ref()
                .map(|c| c.display_name().to_string())
                .unwrap_or_else(|| "unset".to_string()),
            turn_number: self.turn_count,
            transcript_summary: transcript_summary.clone(),
            tool_calls: stats.tool_calls,
            timestamp: Utc::now(),
        }));

        tracing::info!(
            turn = self.turn_count,
            summary = %transcript_summary,
            prompt_tokens = stats.prompt_tokens,
            "Turn complete"
        );

        // Trim conversation if too long
        self.trim_conversation();
    }

    /// Execute a batch of tool calls
    async fn execute_tool_calls(
        &self,
        tool_calls: &[ToolCallRequest],
        stats: &mut TurnStats,
    ) -> Vec<(String, String)> {
        let mut results = Vec::new();

        for tc in tool_calls {
            let start = std::time::Instant::now();

            // Parse arguments
            let arguments: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

            let call = ToolCall {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments,
            };

            // Execute
            let response = {
                let mut executor = self.tool_executor.write().await;
                executor.execute(&call)
            };

            let duration = start.elapsed();
            let success = response.result.is_ok();

            stats.total_tool_calls += 1;
            stats.tool_calls.push(tc.function.name.clone());
            if !success {
                stats.failed_tool_calls += 1;
            }

            let output = match response.result {
                Ok(r) => {
                    // Check if tool wrote to embeddings/
                    if let Some(ref path) = r.output_file {
                        if path.contains("embeddings/") || path.contains("embeddings\\") {
                            self.bus.publish(CoordinatorEvent::Agent(AgentEvent::NoteWritten {
                                path: path.clone(),
                                timestamp: Utc::now(),
                            }));
                        }
                    }
                    r.output
                }
                Err(e) => format!("Error: {}", e),
            };

            tracing::info!(
                tool = %tc.function.name,
                call_id = %tc.id,
                success = success,
                duration_ms = duration.as_millis(),
                "Tool executed"
            );

            results.push((tc.id.clone(), output));
        }

        results
    }

    /// Build system prompt from workspace files
    async fn build_system_prompt(&self) -> String {
        let mut parts = Vec::new();

        // Load identity files
        for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
            let path = self.config.workspace.join(filename);
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                parts.push(content);
            }
        }

        // Load continuity state
        let state_path = self.config.workspace.join("thinking/current-state.md");
        if let Ok(state) = tokio::fs::read_to_string(&state_path).await {
            parts.push(format!("Continuing session. Last cycle you were:\n{}", state));
        }

        // Add current time with timezone from preferences
        let prefs = Preferences::load(&self.config.workspace);
        let time_str = format_current_time(prefs.timezone());
        parts.push(format!("Current time: {}", time_str));

        if parts.is_empty() {
            "You are an AI assistant.".to_string()
        } else {
            parts.join("\n\n---\n\n")
        }
    }

    /// Trim conversation to stay within history limit
    fn trim_conversation(&mut self) {
        let max_messages = self.config.history_limit * 2; // Some buffer
        if self.conversation.len() > max_messages {
            let trim_count = self.conversation.len() - max_messages;
            self.conversation.drain(0..trim_count);
            tracing::debug!(
                trimmed = trim_count,
                remaining = self.conversation.len(),
                "Trimmed old conversation messages"
            );
        }
    }

    /// Switch to a different channel
    pub fn set_channel_context(&mut self, context: ChannelContext) {
        let old = self.channel_context
            .as_ref()
            .map(|c| c.display_name().to_string())
            .unwrap_or_else(|| "unset".to_string());
        let new = context.display_name().to_string();

        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched {
            from: old.clone(),
            to: new.clone(),
            timestamp: Utc::now(),
        }));

        tracing::info!(from = %old, to = %new, "Channel switched");
        self.channel_context = Some(context);
    }

    /// Get current channel context
    pub fn channel_context(&self) -> Option<&ChannelContext> {
        self.channel_context.as_ref()
    }

    /// Get current turn count
    pub fn turn_count(&self) -> u64 {
        self.turn_count
    }

    /// Get last known prompt token count
    pub fn last_prompt_tokens(&self) -> u64 {
        self.last_prompt_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::channel::ChannelContext;
    use crate::coordinator::Coordinator;
    use crate::tools::ToolRegistry;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn test_config(workspace: &TempDir) -> AgentTaskConfig {
        AgentTaskConfig {
            workspace: workspace.path().to_path_buf(),
            embeddings_dir: workspace.path().join("embeddings"),
            heartbeat_interval: Duration::from_millis(100),
            ..Default::default()
        }
    }

    #[test]
    fn test_agent_task_config_default() {
        let config = AgentTaskConfig::default();
        assert_eq!(config.max_tool_calls, 50);
        assert_eq!(config.history_limit, 50);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(45 * 60));
        assert_eq!(config.context_limit, 128_000);
    }

    #[tokio::test]
    async fn test_agent_task_emits_events() {
        let temp = TempDir::new().unwrap();
        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();

        // Subscribe to events before creating task
        let mut event_rx = bus.subscribe();

        let message_queue = Arc::new(MessageQueue::new());
        let flash_queue = Arc::new(FlashQueue::new(10));
        let tool_executor = Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new())));

        // Create model client (won't actually call API in test)
        let model_client = ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        ).unwrap();

        let task = AgentTask::new(
            config,
            bus.clone(),
            message_queue.clone(),
            model_client,
            tool_executor,
            flash_queue,
        );

        // Note: turn_cycle would fail without a real model, so we just test the event emission
        // by checking the bus directly
        task.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: "test".into(),
            turn_number: 1,
            timestamp: Utc::now(),
        }));

        let event1 = event_rx.try_recv();
        assert!(matches!(event1, Ok(CoordinatorEvent::Agent(AgentEvent::TurnStarted { turn_number: 1, .. }))));
    }

    #[tokio::test]
    async fn test_agent_task_channel_switch() {
        let temp = TempDir::new().unwrap();
        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();
        let mut event_rx = bus.subscribe();

        let message_queue = Arc::new(MessageQueue::new());
        let flash_queue = Arc::new(FlashQueue::new(10));
        let tool_executor = Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new())));
        let model_client = ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        ).unwrap();

        let mut task = AgentTask::new(
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
        );

        assert!(task.channel_context().is_none());

        let ctx = ChannelContext {
            path: PathBuf::from("conversations/discord/general.txt"),
            adapter: "discord".to_string(),
            channel_id: "123".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: None,
        };
        task.set_channel_context(ctx);

        assert!(task.channel_context().is_some());
        assert_eq!(task.channel_context().unwrap().display_name(), "general");

        let event = event_rx.try_recv();
        assert!(matches!(event, Ok(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched { .. }))));
    }

    #[tokio::test]
    async fn test_build_system_prompt_default() {
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

        let task = AgentTask::new(
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
        );

        let prompt = task.build_system_prompt().await;
        // With no identity files, should have at least current time
        assert!(prompt.contains("Current time:") || prompt.contains("You are an AI assistant"));
    }

    #[tokio::test]
    async fn test_build_system_prompt_with_identity() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("IDENTITY.md"), "I am River, a helpful assistant.").unwrap();

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

        let task = AgentTask::new(
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
        );

        let prompt = task.build_system_prompt().await;
        assert!(prompt.contains("I am River"));
        assert!(prompt.contains("Current time:"));
    }

    #[test]
    fn test_turn_stats_default() {
        let stats = TurnStats::default();
        assert_eq!(stats.total_tool_calls, 0);
        assert_eq!(stats.failed_tool_calls, 0);
        assert!(stats.tool_calls.is_empty());
    }

    #[test]
    fn test_trim_conversation() {
        // Test that trim_conversation properly removes old messages
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

        let mut task = AgentTask::new(
            AgentTaskConfig {
                history_limit: 5,
                ..Default::default()
            },
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
        );

        // Add more messages than the limit
        for i in 0..20 {
            task.conversation.push(ChatMessage::user(format!("Message {}", i)));
        }

        assert_eq!(task.conversation.len(), 20);
        task.trim_conversation();
        // history_limit * 2 = 10
        assert_eq!(task.conversation.len(), 10);
        // Should have kept the most recent messages
        assert!(task.conversation[0].content.as_ref().unwrap().contains("Message 10"));
    }
}

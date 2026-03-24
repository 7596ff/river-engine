//! Agent task — the acting self (I)
//!
//! Runs as a peer task in the coordinator, managing the wake/think/act/settle
//! turn cycle. Receives events from the coordinator bus and emits lifecycle
//! events for the spectator to observe.

use crate::agent::context::{ContextAssembler, ContextBudget};
use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::flash::{Flash, FlashQueue, FlashTTL};
use crate::r#loop::{MessageQueue, ModelClient};
use crate::r#loop::context::ChatMessage;
use crate::tools::ToolExecutor;
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
        }
    }
}

/// The agent task — runs as a peer task in the coordinator
pub struct AgentTask {
    config: AgentTaskConfig,
    bus: EventBus,
    message_queue: Arc<MessageQueue>,
    #[allow(dead_code)] // Used in full implementation for model calls
    model_client: ModelClient,
    tool_executor: Arc<RwLock<ToolExecutor>>,
    context_assembler: ContextAssembler,
    flash_queue: Arc<FlashQueue>,
    turn_count: u64,
    current_channel: String,
    /// Recent messages for hot context (accumulated across turns)
    recent_messages: Vec<ChatMessage>,
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
            current_channel: "default".into(),
            recent_messages: Vec::new(),
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
                    // Heartbeat wake - check if there's work to do
                    if !self.message_queue.is_empty() {
                        self.turn_cycle().await;
                    } else {
                        tracing::debug!("Heartbeat: no pending messages");
                    }
                }
                // Check for new messages periodically
                _ = self.wait_for_messages() => {
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

    /// Wait until there are messages in the queue
    async fn wait_for_messages(&self) {
        // Poll the queue periodically until we have messages
        loop {
            if !self.message_queue.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// One turn: wake → think → act → settle
    async fn turn_cycle(&mut self) {
        self.turn_count += 1;
        let turn_start = Utc::now();

        // 1. WAKE: tick flash queue, emit TurnStarted
        self.flash_queue.tick_turn().await;
        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: self.current_channel.clone(),
            turn_number: self.turn_count,
            timestamp: turn_start,
        }));

        tracing::info!(
            turn = self.turn_count,
            channel = %self.current_channel,
            "Turn started"
        );

        // Drain incoming messages
        let incoming = self.message_queue.drain();
        for msg in &incoming {
            let chat_msg = ChatMessage::user(format!(
                "[{}] {}: {}",
                msg.channel, msg.author.name, msg.content
            ));
            self.recent_messages.push(chat_msg);
        }

        // 2. ASSEMBLE: build context from layers
        let system_prompt = self.load_system_prompt().await;
        let context = self.context_assembler.assemble(
            &self.current_channel,
            &system_prompt,
            &self.recent_messages,
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
        let context_percent = (context.token_estimate as f64 / self.config.context_budget.total as f64) * 100.0;
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

        // 3. THINK: call model
        // Note: Full implementation would:
        // - Get tool schemas from executor
        // - Call model_client.complete()
        // - Handle response content and tool calls
        // For skeleton, we just log
        let _tools = {
            let executor = self.tool_executor.read().await;
            executor.schemas()
        };

        // TODO: Actual model call and tool execution loop
        // let response = self.model_client.complete(&context.messages, &tools).await;
        // ... handle tool calls in a loop ...

        // 4. ACT: execute tools
        // Tool execution would happen here in a loop until model stops requesting tools

        // 5. SETTLE: emit TurnComplete
        let tool_calls: Vec<String> = vec![]; // Would be populated from actual execution
        let transcript_summary = format!(
            "Turn {} completed ({} messages processed)",
            self.turn_count,
            incoming.len()
        );

        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
            channel: self.current_channel.clone(),
            turn_number: self.turn_count,
            transcript_summary: transcript_summary.clone(),
            tool_calls,
            timestamp: Utc::now(),
        }));

        tracing::info!(
            turn = self.turn_count,
            summary = %transcript_summary,
            "Turn complete"
        );
    }

    /// Load system prompt from identity files
    async fn load_system_prompt(&self) -> String {
        let identity_path = self.config.workspace.join("IDENTITY.md");
        tokio::fs::read_to_string(&identity_path).await
            .unwrap_or_else(|_| "You are a helpful assistant.".into())
    }

    /// Switch to a different channel
    pub fn switch_channel(&mut self, channel: String) {
        let old = std::mem::replace(&mut self.current_channel, channel.clone());
        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched {
            from: old.clone(),
            to: channel,
            timestamp: Utc::now(),
        }));
        tracing::info!(from = %old, to = %self.current_channel, "Channel switched");
    }

    /// Get current turn count
    pub fn turn_count(&self) -> u64 {
        self.turn_count
    }

    /// Get current channel
    pub fn current_channel(&self) -> &str {
        &self.current_channel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::Coordinator;
    use crate::r#loop::model::ModelClient;
    use crate::tools::ToolRegistry;
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

        let mut task = AgentTask::new(
            config,
            bus.clone(),
            message_queue.clone(),
            model_client,
            tool_executor,
            flash_queue,
        );

        // Run a single turn
        task.turn_cycle().await;

        // Should receive TurnStarted and TurnComplete
        let event1 = event_rx.try_recv();
        assert!(matches!(event1, Ok(CoordinatorEvent::Agent(AgentEvent::TurnStarted { turn_number: 1, .. }))));

        let event2 = event_rx.try_recv();
        assert!(matches!(event2, Ok(CoordinatorEvent::Agent(AgentEvent::TurnComplete { turn_number: 1, .. }))));
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

        assert_eq!(task.current_channel(), "default");
        task.switch_channel("general".into());
        assert_eq!(task.current_channel(), "general");

        let event = event_rx.try_recv();
        assert!(matches!(event, Ok(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched { .. }))));
    }

    #[test]
    fn test_agent_task_turn_count() {
        // Just verify the struct can be instantiated and has correct initial state
        let config = AgentTaskConfig::default();
        assert_eq!(config.max_tool_calls, 50);
    }
}

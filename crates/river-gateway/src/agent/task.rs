//! Agent task — the acting self (I)
//!
//! Runs as a peer task in the coordinator, managing the wake/think/act/settle
//! turn cycle. Context is built from the home channel — an append-only JSONL log.

use crate::agent::home_context::{self, HomeContextConfig};
use crate::channels::entry::{HeartbeatEntry, HomeChannelEntry, MessageEntry, ToolEntry};
use crate::channels::writer::HomeChannelWriter;
use crate::coordinator::{AgentEvent, CoordinatorEvent, EventBus, SpectatorEvent};
use crate::flash::{Flash, FlashQueue, FlashTTL};
use crate::model::{ChatMessage, ModelClient, ToolCallRequest};
use crate::preferences::{format_current_time, Preferences};
use crate::queue::MessageQueue;
use crate::tools::{ToolCall, ToolExecutor, ToolSchema};
use chrono::Utc;
use river_core::{SnowflakeGenerator, SnowflakeType};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Configuration for the agent task
#[derive(Debug, Clone)]
pub struct AgentTaskConfig {
    /// Workspace path for loading identity and context files
    pub workspace: PathBuf,
    /// Timeout for model calls
    pub model_timeout: Duration,
    /// Maximum tool calls per turn (safety limit)
    pub max_tool_calls: usize,
    /// Heartbeat interval (how often to wake if no messages)
    pub heartbeat_interval: Duration,
    /// Home context configuration (tail limit, etc.)
    pub home_context_config: HomeContextConfig,
}

impl Default for AgentTaskConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            model_timeout: Duration::from_secs(120),
            max_tool_calls: 50,
            heartbeat_interval: Duration::from_secs(45 * 60),
            home_context_config: HomeContextConfig::default(),
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

/// Result of a tool execution, preserving tool name for logging
pub struct ToolExecResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub result: String,
}

/// The agent task — runs as a peer task in the coordinator
pub struct AgentTask {
    config: AgentTaskConfig,
    bus: EventBus,
    message_queue: Arc<MessageQueue>,
    model_client: ModelClient,
    tool_executor: Arc<RwLock<ToolExecutor>>,
    flash_queue: Arc<FlashQueue>,
    snowflake_gen: Arc<SnowflakeGenerator>,
    turn_count: u64,
    /// Home channel writer
    home_channel_writer: HomeChannelWriter,
    /// Path to home channel JSONL
    home_channel_path: PathBuf,
    /// Agent name
    agent_name: String,
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
        home_channel_writer: HomeChannelWriter,
        home_channel_path: PathBuf,
        agent_name: String,
    ) -> anyhow::Result<Self> {
        // Verify identity files exist
        Self::build_system_prompt_sync(&config.workspace)?;

        Ok(Self {
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
            snowflake_gen,
            turn_count: 0,
            home_channel_writer,
            home_channel_path,
            agent_name,
        })
    }

    /// Main run loop — called by coordinator via spawn_task
    pub async fn run(mut self) {
        let mut event_rx = self.bus.subscribe();

        tracing::info!("Agent task started");

        loop {
            tokio::select! {
                // Wait for messages or heartbeat timeout
                _ = tokio::time::sleep(self.config.heartbeat_interval) => {
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
        let mut stats = TurnStats::default();

        // ========== WAKE ==========
        self.flash_queue.tick_turn().await;
        let channel_name = "home".to_string();

        self.bus
            .publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
                channel: channel_name.clone(),
                turn_number: self.turn_count,
                timestamp: Utc::now(),
            }));

        // Drain notifications (so the queue is clear for mid-turn checks)
        let _notifications = self.message_queue.drain();

        // Write heartbeat to home channel if applicable
        if is_heartbeat {
            let hb = HeartbeatEntry::new(
                self.snowflake_gen.next_id(SnowflakeType::Message),
                Utc::now().to_rfc3339(),
            );
            self.home_channel_writer
                .write(HomeChannelEntry::Heartbeat(hb))
                .await;
        }

        tracing::info!(
            turn = self.turn_count,
            is_heartbeat = is_heartbeat,
            "Turn started"
        );

        // ========== BUILD CONTEXT FROM HOME CHANNEL ==========
        let system_prompt = match self.build_system_prompt().await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "Failed to build system prompt");
                return;
            }
        };

        // Load moves from filesystem
        let moves = self.load_moves().await;

        let home_messages = match home_context::build_context(
            &self.home_channel_path,
            &moves,
            &self.config.home_context_config,
        )
        .await
        {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "Failed to build context from home channel");
                return;
            }
        };

        let mut messages = vec![ChatMessage::system(system_prompt)];
        messages.extend(home_messages);

        // Get tool schemas
        let tools: Vec<ToolSchema> = {
            let executor = self.tool_executor.read().await;
            executor.schemas()
        };

        // ========== THINK + ACT LOOP ==========
        let mut iteration = 0;
        tracing::debug!(max_iterations = self.config.max_tool_calls, "Starting think+act loop");

        loop {
            iteration += 1;
            tracing::debug!(iteration, "Loop iteration start");
            if iteration > self.config.max_tool_calls {
                tracing::warn!(
                    iterations = iteration,
                    max = self.config.max_tool_calls,
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

            tracing::info!(
                iteration = iteration,
                prompt_tokens = response.usage.prompt_tokens,
                completion_tokens = response.usage.completion_tokens,
                tool_calls = response.tool_calls.len(),
                has_content = response.content.is_some(),
                "Model response"
            );

            // Add assistant response to local conversation and write to home channel
            let assistant_msg = ChatMessage::assistant(
                response.content.clone(),
                if response.tool_calls.is_empty() {
                    None
                } else {
                    Some(response.tool_calls.clone())
                },
            );
            messages.push(assistant_msg);

            if let Some(ref content) = response.content {
                let entry = MessageEntry::agent(
                    self.snowflake_gen.next_id(SnowflakeType::Message),
                    content.clone(),
                    "home".to_string(),
                    None,
                );
                self.home_channel_writer
                    .write(HomeChannelEntry::Message(entry))
                    .await;
            }

            // If no tool calls, we're done
            if response.tool_calls.is_empty() {
                tracing::debug!(iteration, "No tool calls — breaking loop");
                if let Some(ref content) = response.content {
                    tracing::info!(
                        content_len = content.len(),
                        "Assistant response (no tool calls)"
                    );
                }
                break;
            }
            tracing::debug!(iteration, tool_count = response.tool_calls.len(), "Has tool calls — executing");

            // Write tool calls to home channel
            for tc in &response.tool_calls {
                let entry = ToolEntry::call(
                    self.snowflake_gen.next_id(SnowflakeType::Message),
                    tc.function.name.clone(),
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null),
                    tc.id.clone(),
                );
                self.home_channel_writer
                    .write(HomeChannelEntry::Tool(entry))
                    .await;
            }

            // ========== ACT: Execute tool calls ==========
            let tool_results = self
                .execute_tool_calls(&response.tool_calls, &mut stats)
                .await;

            // Add tool results to local conversation and write to home channel
            for result in &tool_results {
                let tool_msg = ChatMessage::tool(&result.tool_call_id, &result.result);
                messages.push(tool_msg);

                let snowflake = self.snowflake_gen.next_id(SnowflakeType::Message);
                let entry = if result.result.len() > 4096 {
                    let results_dir = self
                        .config
                        .workspace
                        .join("channels")
                        .join("home")
                        .join(&self.agent_name)
                        .join("tool-results");
                    tokio::fs::create_dir_all(&results_dir).await.ok();
                    let file_path = results_dir.join(format!("{}.txt", snowflake));
                    tokio::fs::write(&file_path, &result.result).await.ok();
                    ToolEntry::result_file(
                        snowflake,
                        result.tool_name.clone(),
                        file_path.to_string_lossy().to_string(),
                        result.tool_call_id.clone(),
                    )
                } else {
                    ToolEntry::result(
                        snowflake,
                        result.tool_name.clone(),
                        result.result.clone(),
                        result.tool_call_id.clone(),
                    )
                };
                self.home_channel_writer
                    .write(HomeChannelEntry::Tool(entry))
                    .await;
            }

            tracing::debug!(iteration, results = tool_results.len(), "Tool execution complete, checking for mid-turn messages");

            // Check for messages that arrived during tool execution (final batch check)
            let mid_turn_notifications = self.message_queue.drain();
            if !mid_turn_notifications.is_empty() {
                // Messages are already in the home channel via handle_incoming.
                // Re-read context to pick them up, or inject a system note.
                let system_msg = ChatMessage::system(
                    "[New messages arrived during tool execution — they are in the home channel]"
                        .to_string(),
                );
                messages.push(system_msg);
            }
            tracing::debug!(iteration, message_count = messages.len(), "Continuing loop — will call model again");
        }

        // ========== SETTLE ==========
        let transcript_summary = format!(
            "Turn {} completed: {} tool calls ({} failed)",
            self.turn_count, stats.total_tool_calls, stats.failed_tool_calls
        );

        self.bus
            .publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
                channel: channel_name,
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
    }

    /// Execute a batch of tool calls
    async fn execute_tool_calls(
        &self,
        tool_calls: &[ToolCallRequest],
        stats: &mut TurnStats,
    ) -> Vec<ToolExecResult> {
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
                            self.bus
                                .publish(CoordinatorEvent::Agent(AgentEvent::NoteWritten {
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

            results.push(ToolExecResult {
                tool_call_id: tc.id.clone(),
                tool_name: tc.function.name.clone(),
                result: output,
            });
        }

        results
    }

    /// Build system prompt from workspace files (async version)
    async fn build_system_prompt(&self) -> anyhow::Result<String> {
        let mut parts = Vec::new();

        for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
            let path = self.config.workspace.join(filename);
            let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
                anyhow::anyhow!("Required identity file missing: {:?} ({})", path, e)
            })?;
            parts.push(content);
        }

        let prefs = Preferences::load(&self.config.workspace);
        let time_str = format_current_time(prefs.timezone());
        parts.push(format!("Current time: {}", time_str));

        Ok(parts.join("\n\n---\n\n"))
    }

    /// Build system prompt synchronously (for use in new())
    fn build_system_prompt_sync(workspace: &Path) -> anyhow::Result<String> {
        let mut parts = Vec::new();

        for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
            let path = workspace.join(filename);
            let content = std::fs::read_to_string(&path).map_err(|e| {
                anyhow::anyhow!("Required identity file missing: {:?} ({})", path, e)
            })?;
            parts.push(content);
        }

        let prefs = Preferences::load(workspace);
        let time_str = format_current_time(prefs.timezone());
        parts.push(format!("Current time: {}", time_str));

        Ok(parts.join("\n\n---\n\n"))
    }

    /// Load move summaries from moves.jsonl, returning the last N
    async fn load_moves(&self) -> Vec<String> {
        let moves_path = self
            .config
            .workspace
            .join("channels")
            .join("home")
            .join(&self.agent_name)
            .join("moves.jsonl");

        crate::spectator::moves::read_moves_tail(&moves_path, 10)
            .await
            .into_iter()
            .map(|m| m.summary)
            .collect()
    }

    /// Get current turn count
    pub fn turn_count(&self) -> u64 {
        self.turn_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::writer::HomeChannelWriter;
    use crate::coordinator::Coordinator;
    use crate::tools::ToolRegistry;
    use tempfile::TempDir;

    fn test_sg() -> Arc<SnowflakeGenerator> {
        let birth = river_core::AgentBirth::new(2026, 4, 29, 12, 0, 0).unwrap();
        Arc::new(SnowflakeGenerator::new(birth))
    }

    fn test_config(workspace: &TempDir) -> AgentTaskConfig {
        AgentTaskConfig {
            workspace: workspace.path().to_path_buf(),
            heartbeat_interval: Duration::from_millis(100),
            ..Default::default()
        }
    }

    /// Write required identity files to a temp workspace
    fn write_identity_files(workspace: &TempDir) {
        std::fs::write(workspace.path().join("AGENTS.md"), "# Agent Protocol").unwrap();
        std::fs::write(workspace.path().join("IDENTITY.md"), "I am a test agent.").unwrap();
        std::fs::write(workspace.path().join("RULES.md"), "Be helpful.").unwrap();
    }

    fn test_task(temp: &TempDir) -> AgentTask {
        write_identity_files(temp);
        let config = test_config(temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();
        let sg = test_sg();

        let home_path = temp.path().join("channels/home/test-agent.jsonl");
        let writer = HomeChannelWriter::spawn(home_path.clone());

        AgentTask::new(
            config,
            bus,
            Arc::new(MessageQueue::new()),
            ModelClient::new(
                "http://localhost:8080".to_string(),
                "test-model".to_string(),
                Duration::from_secs(30),
                None,
            )
            .unwrap(),
            Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new()))),
            Arc::new(FlashQueue::new(10)),
            sg,
            writer,
            home_path,
            "test-agent".to_string(),
        )
        .unwrap()
    }

    #[test]
    fn test_agent_task_config_default() {
        let config = AgentTaskConfig::default();
        assert_eq!(config.max_tool_calls, 50);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(45 * 60));
        assert_eq!(config.home_context_config.max_tail_entries, 200);
    }

    #[tokio::test]
    async fn test_agent_task_emits_events() {
        let temp = TempDir::new().unwrap();
        let task = test_task(&temp);
        let mut event_rx = task.bus.subscribe();

        task.bus
            .publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
                channel: "test".into(),
                turn_number: 1,
                timestamp: Utc::now(),
            }));

        let event1 = event_rx.try_recv();
        assert!(matches!(
            event1,
            Ok(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
                turn_number: 1,
                ..
            }))
        ));
    }

    #[tokio::test]
    async fn test_build_system_prompt_missing_files() {
        let temp = TempDir::new().unwrap();
        let config = test_config(&temp);
        let coord = Coordinator::new();
        let sg = test_sg();
        let home_path = temp.path().join("channels/home/test.jsonl");
        let writer = HomeChannelWriter::spawn(home_path.clone());

        let result = AgentTask::new(
            config,
            coord.bus().clone(),
            Arc::new(MessageQueue::new()),
            ModelClient::new(
                "http://localhost:8080".into(),
                "test".into(),
                Duration::from_secs(30),
                None,
            )
            .unwrap(),
            Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new()))),
            Arc::new(FlashQueue::new(10)),
            sg,
            writer,
            home_path,
            "test".to_string(),
        );

        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("AGENTS.md"),
            "Error should mention AGENTS.md: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_build_system_prompt_with_identity() {
        let temp = TempDir::new().unwrap();
        let task = test_task(&temp);

        let prompt = task.build_system_prompt().await.unwrap();
        assert!(prompt.contains("test agent"));
        assert!(prompt.contains("Agent Protocol"));
        assert!(prompt.contains("Be helpful"));
        assert!(prompt.contains("Current time:"));
    }

    #[test]
    fn test_turn_stats_default() {
        let stats = TurnStats::default();
        assert_eq!(stats.total_tool_calls, 0);
        assert_eq!(stats.failed_tool_calls, 0);
        assert!(stats.tool_calls.is_empty());
    }
}

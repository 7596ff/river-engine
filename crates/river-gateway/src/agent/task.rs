//! Agent task — the acting self (I)
//!
//! Runs as a peer task in the coordinator, managing the wake/think/act/settle
//! turn cycle. Uses a persistent context that accumulates messages and compacts
//! via spectator cursor coordination.

use crate::agent::channel::ChannelContext;
use crate::agent::context::{PersistentContext, ContextConfig, ContextMessage};
use crate::channels::entry::{HomeChannelEntry, MessageEntry, ToolEntry};
use crate::channels::writer::HomeChannelWriter;
use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::flash::{Flash, FlashQueue, FlashTTL};
use crate::preferences::{Preferences, format_current_time};
use crate::model::{ChatMessage, ModelClient, ToolCallRequest};
use crate::queue::MessageQueue;
use crate::session::PRIMARY_SESSION_ID;
use crate::tools::{ToolExecutor, ToolCall, ToolSchema};
use chrono::Utc;
use river_core::{SnowflakeGenerator, SnowflakeType};
use river_db::{Database, Message, MessageRole};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Configuration for the agent task
#[derive(Debug, Clone)]
pub struct AgentTaskConfig {
    /// Workspace path for loading identity and context files
    pub workspace: PathBuf,
    /// Context configuration
    pub context_config: ContextConfig,
    /// Timeout for model calls
    pub model_timeout: Duration,
    /// Maximum tool calls per turn (safety limit)
    pub max_tool_calls: usize,
    /// Heartbeat interval (how often to wake if no messages)
    pub heartbeat_interval: Duration,
}

impl Default for AgentTaskConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            context_config: ContextConfig::default(),
            model_timeout: Duration::from_secs(120),
            max_tool_calls: 50,
            heartbeat_interval: Duration::from_secs(45 * 60),
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
    db: Arc<Mutex<Database>>,
    snowflake_gen: Arc<SnowflakeGenerator>,
    turn_count: u64,
    channel_context: Option<ChannelContext>,
    /// Pending channel switch (applied at start of next turn)
    pending_channel_switch: Option<ChannelContext>,
    /// The persistent context object
    context: PersistentContext,
    /// Last estimated prompt tokens (for calibration)
    last_estimated_prompt_tokens: usize,
    /// Home channel writer (if configured)
    home_channel_writer: Option<HomeChannelWriter>,
    /// Agent name (for home channel paths)
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
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
        home_channel_writer: Option<HomeChannelWriter>,
        agent_name: String,
    ) -> anyhow::Result<Self> {
        // Build system prompt synchronously
        let system_prompt = Self::build_system_prompt_sync(&config.workspace)?;

        let channel = "default";

        let context = {
            let db_guard = db.lock().expect("DB lock poisoned");
            PersistentContext::build(
                config.context_config.clone(),
                system_prompt,
                &db_guard,
                PRIMARY_SESSION_ID,
                channel,
            )
        };

        Ok(Self {
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
            db,
            snowflake_gen,
            turn_count: 0,
            channel_context: None,
            pending_channel_switch: None,
            context,
            last_estimated_prompt_tokens: 0,
            home_channel_writer,
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

        // ========== CHECK PENDING CHANNEL SWITCH ==========
        if let Some(new_channel) = self.pending_channel_switch.take() {
            let channel_name = new_channel.display_name().to_string();
            self.channel_context = Some(new_channel);

            let system_prompt = self.build_system_prompt().await
                .expect("Identity files missing during channel switch");
            let db_guard = self.db.lock().expect("DB lock poisoned");
            self.context = PersistentContext::build(
                self.config.context_config.clone(),
                system_prompt,
                &db_guard,
                PRIMARY_SESSION_ID,
                &channel_name,
            );
            drop(db_guard);

            tracing::info!(channel = %channel_name, "Channel switch applied");
        }

        // ========== WAKE ==========
        self.flash_queue.tick_turn().await;
        let channel_name = self.channel_context
            .as_ref()
            .map(|c| c.display_name().to_string())
            .unwrap_or_else(|| "default".to_string());

        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: channel_name.clone(),
            turn_number: self.turn_count,
            timestamp: Utc::now(),
        }));

        tracing::info!(
            turn = self.turn_count,
            channel = %channel_name,
            is_heartbeat = is_heartbeat,
            "Turn started"
        );

        // Drain notifications and read channel logs
        let notifications = self.message_queue.drain();
        let channels_dir = self.config.workspace.join("channels");

        // Deduplicate channels (multiple notifications for same channel).
        // This set grows if mid-turn notifications arrive for new channels.
        let mut seen_channels = std::collections::HashSet::new();
        for notification in &notifications {
            seen_channels.insert(notification.channel.clone());
        }

        // Read new messages from each channel
        let mut all_new_messages = Vec::new();
        for channel_key in &seen_channels {
            // Parse adapter and channel_id from the key
            let parts: Vec<&str> = channel_key.splitn(2, '_').collect();
            if parts.len() != 2 {
                tracing::warn!(channel = %channel_key, "Invalid channel key format");
                continue;
            }
            let log = crate::channels::ChannelLog::open(&channels_dir, parts[0], parts[1]);
            match log.read_since_cursor(50).await {
                Ok(entries) => {
                    for entry in entries {
                        if let crate::channels::ChannelEntry::Message(msg) = entry {
                            if !msg.is_agent() {
                                all_new_messages.push((channel_key.clone(), msg));
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(channel = %channel_key, error = %e, "Failed to read channel log");
                }
            }
        }

        // Auto-set channel context from first incoming message if not yet set
        if self.channel_context.is_none() {
            if let Some((channel_key, _)) = all_new_messages.first() {
                let parts: Vec<&str> = channel_key.splitn(2, '_').collect();
                if parts.len() == 2 {
                    let ctx = ChannelContext {
                        path: PathBuf::from(format!("channels/{}_{}.jsonl", parts[0], parts[1])),
                        adapter: parts[0].to_string(),
                        channel_id: parts[1].to_string(),
                        channel_name: None,
                        guild_id: None,
                    };
                    tracing::info!(
                        adapter = %ctx.adapter,
                        channel = %ctx.channel_id,
                        "Auto-set channel context from first incoming message"
                    );
                    self.channel_context = Some(ctx);
                }
            }
        }

        // Add messages to context
        for (channel, msg) in &all_new_messages {
            let author = msg.author.as_deref().unwrap_or("unknown");
            let chat_msg = ChatMessage::user(format!(
                "[{}] {}: {}",
                channel, author, msg.content
            ));
            self.context.append(ContextMessage::new(chat_msg, self.turn_count));
        }

        // Add heartbeat trigger if applicable
        if is_heartbeat && all_new_messages.is_empty() {
            self.context.append(ContextMessage::new(
                ChatMessage::user(":heartbeat:"),
                self.turn_count,
            ));
        }

        // ========== CHECK COMPACTION ==========
        if self.context.needs_compaction() {
            let system_prompt = self.build_system_prompt().await
                .expect("Identity files missing during compaction");
            let db_guard = self.db.lock().expect("DB lock poisoned");
            self.context.compact(
                system_prompt,
                &db_guard,
                PRIMARY_SESSION_ID,
                &channel_name,
                self.turn_count,
            );
            drop(db_guard);
            tracing::info!(
                turn = self.turn_count,
                tokens = self.context.estimate_total_tokens(),
                "Context compacted"
            );
        }

        // Check context pressure
        let total_tokens = self.context.estimate_total_tokens();
        let context_percent = (total_tokens as f64 / self.config.context_config.limit as f64) * 100.0;
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
        let mut messages = self.context.to_messages();
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > self.config.max_tool_calls {
                tracing::warn!(
                    iterations = iteration,
                    max = self.config.max_tool_calls,
                    "Max tool call iterations reached, breaking"
                );
                break;
            }

            // Track estimated tokens before model call (for calibration)
            self.last_estimated_prompt_tokens = self.context.estimate_total_tokens();

            // Call model
            let response = match self.model_client.complete(&messages, &tools).await {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::error!(error = %e, "Model call failed");
                    break;
                }
            };

            // Calibrate token estimation
            self.context.update_calibration(
                response.usage.prompt_tokens as u64,
                self.last_estimated_prompt_tokens,
            );

            stats.prompt_tokens = response.usage.prompt_tokens as u64;

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
            self.context.append(ContextMessage::new(assistant_msg, self.turn_count));

            // Write assistant text to home channel
            if let Some(ref writer) = self.home_channel_writer {
                if let Some(ref content) = response.content {
                    let entry = MessageEntry::agent(
                        self.snowflake_gen.next_id(SnowflakeType::Message).to_string(),
                        content.clone(), "home".to_string(), None,
                    );
                    writer.write(HomeChannelEntry::Message(entry)).await;
                }
            }

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

            // Write tool calls to home channel
            if let Some(ref writer) = self.home_channel_writer {
                for tc in &response.tool_calls {
                    let entry = ToolEntry::call(
                        self.snowflake_gen.next_id(SnowflakeType::Message).to_string(),
                        tc.function.name.clone(),
                        serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null),
                        tc.id.clone(),
                    );
                    writer.write(HomeChannelEntry::Tool(entry)).await;
                }
            }

            // ========== ACT: Execute tool calls ==========
            let tool_results = self.execute_tool_calls(&response.tool_calls, &mut stats).await;

            // Add tool results to conversation and write to home channel
            for result in &tool_results {
                let tool_msg = ChatMessage::tool(&result.tool_call_id, &result.result);
                messages.push(tool_msg.clone());
                self.context.append(ContextMessage::new(tool_msg, self.turn_count));

                // Write tool result to home channel
                if let Some(ref writer) = self.home_channel_writer {
                    let snowflake = self.snowflake_gen.next_id(SnowflakeType::Message).to_string();
                    let entry = if result.result.len() > 4096 {
                        let results_dir = self.config.workspace.join("channels").join("home")
                            .join(&self.agent_name).join("tool-results");
                        tokio::fs::create_dir_all(&results_dir).await.ok();
                        let file_path = results_dir.join(format!("{}.txt", snowflake));
                        tokio::fs::write(&file_path, &result.result).await.ok();
                        ToolEntry::result_file(
                            snowflake, result.tool_name.clone(),
                            file_path.to_string_lossy().to_string(), result.tool_call_id.clone(),
                        )
                    } else {
                        ToolEntry::result(
                            snowflake, result.tool_name.clone(),
                            result.result.clone(), result.tool_call_id.clone(),
                        )
                    };
                    writer.write(HomeChannelEntry::Tool(entry)).await;
                }
            }

            // Check for notifications that arrived during tool execution
            let mid_turn_notifications = self.message_queue.drain();
            if !mid_turn_notifications.is_empty() {
                // Deduplicate channels
                let mut mid_channels = std::collections::HashSet::new();
                for n in &mid_turn_notifications {
                    mid_channels.insert(n.channel.clone());
                }
                // Read new messages from each notified channel
                let mut mid_content = String::from("Messages received during tool execution:\n");
                let mut has_mid_messages = false;
                for ch in &mid_channels {
                    let parts: Vec<&str> = ch.splitn(2, '_').collect();
                    if parts.len() != 2 { continue; }
                    let log = crate::channels::ChannelLog::open(&channels_dir, parts[0], parts[1]);
                    if let Ok(entries) = log.read_since_cursor(50).await {
                        for entry in entries {
                            if let crate::channels::ChannelEntry::Message(msg) = entry {
                                if !msg.is_agent() {
                                    let author = msg.author.as_deref().unwrap_or("unknown");
                                    mid_content.push_str(&format!(
                                        "- [{}] {}: {}\n",
                                        ch, author, msg.content
                                    ));
                                    has_mid_messages = true;
                                }
                            }
                        }
                    }
                    // Track these channels for cursor writing at settle
                    seen_channels.insert(ch.clone());
                }
                if has_mid_messages {
                    let system_msg = ChatMessage::system(mid_content);
                    messages.push(system_msg.clone());
                    self.context.append(ContextMessage::new(system_msg, self.turn_count));
                }
            }
        }

        // ========== SETTLE ==========
        // Write cursor entries for channels we read
        for channel_key in &seen_channels {
            let parts: Vec<&str> = channel_key.splitn(2, '_').collect();
            if parts.len() == 2 {
                let log = crate::channels::ChannelLog::open(&channels_dir, parts[0], parts[1]);
                let cursor_id = self.snowflake_gen.next_id(river_core::SnowflakeType::Message).to_string();
                let cursor = crate::channels::CursorEntry::new(cursor_id);
                if let Err(e) = log.append_entry(&cursor).await {
                    tracing::warn!(channel = %channel_key, error = %e, "Failed to write cursor");
                }
            }
        }

        self.persist_turn_messages();

        let transcript_summary = format!(
            "Turn {} completed: {} messages, {} tool calls ({} failed)",
            self.turn_count,
            all_new_messages.len(),
            stats.total_tool_calls,
            stats.failed_tool_calls
        );

        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
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

    /// Persist conversation messages from the current turn to the database.
    /// Must be called before emitting TurnComplete (ordering guarantee).
    fn persist_turn_messages(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let db = match self.db.lock() {
            Ok(db) => db,
            Err(e) => {
                tracing::error!(error = %e, "DB lock poisoned in persist_turn_messages");
                return;
            }
        };

        // Persist messages from the persistent context
        // TODO: Track a persistence cursor to avoid re-persisting old messages.
        // For now this persists all non-system messages, which is the same
        // behavior as the previous implementation.
        let messages = self.context.to_messages();
        let mut persisted = 0;
        for chat_msg in &messages {
            if chat_msg.role == "system" {
                continue;
            }

            let role = match chat_msg.role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "tool" => MessageRole::Tool,
                _ => continue,
            };

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
                turn_number: self.turn_count,
            };

            if let Err(e) = db.insert_message(&msg) {
                tracing::warn!(error = %e, "Failed to persist message");
            } else {
                persisted += 1;
            }
        }

        if persisted > 0 {
            tracing::debug!(persisted = persisted, turn = self.turn_count, "Messages persisted");
        }
    }

    /// Switch to a different channel (takes effect at next turn start)
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

        tracing::info!(from = %old, to = %new, "Channel switch pending (applied at next turn)");
        self.pending_channel_switch = Some(context);
    }

    /// Get current channel context
    pub fn channel_context(&self) -> Option<&ChannelContext> {
        self.channel_context.as_ref()
    }

    /// Get current turn count
    pub fn turn_count(&self) -> u64 {
        self.turn_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::channel::ChannelContext;
    use crate::coordinator::Coordinator;
    use crate::tools::ToolRegistry;
    use river_db::init_db;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn test_db(temp: &TempDir) -> (Arc<Mutex<Database>>, Arc<SnowflakeGenerator>) {
        let db = init_db(&temp.path().join("test.db")).unwrap();
        let db_arc = Arc::new(Mutex::new(db));
        let birth = river_core::AgentBirth::new(2026, 4, 29, 12, 0, 0).unwrap();
        let sg = Arc::new(SnowflakeGenerator::new(birth));
        (db_arc, sg)
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

    #[test]
    fn test_agent_task_config_default() {
        let config = AgentTaskConfig::default();
        assert_eq!(config.max_tool_calls, 50);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(45 * 60));
        assert_eq!(config.context_config.limit, 128_000);
    }

    #[tokio::test]
    async fn test_agent_task_emits_events() {
        let temp = TempDir::new().unwrap();
        write_identity_files(&temp);
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

        let (db, sg) = test_db(&temp);
        let task = AgentTask::new(
            config,
            bus.clone(),
            message_queue.clone(),
            model_client,
            tool_executor,
            flash_queue,
            db,
            sg,
            None,
            "test-agent".to_string(),
        ).unwrap();

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
        write_identity_files(&temp);
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

        let (db, sg) = test_db(&temp);
        let mut task = AgentTask::new(
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
            db,
            sg,
            None,
            "test-agent".to_string(),
        ).unwrap();

        assert!(task.channel_context().is_none());

        let ctx = ChannelContext {
            path: PathBuf::from("conversations/discord/general.txt"),
            adapter: "discord".to_string(),
            channel_id: "123".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: None,
        };
        task.set_channel_context(ctx);

        // Channel switch is now deferred — channel_context is still None
        assert!(task.channel_context().is_none());
        assert!(task.pending_channel_switch.is_some());

        let event = event_rx.try_recv();
        assert!(matches!(event, Ok(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched { .. }))));
    }

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
            None,
            "test-agent".to_string(),
        );

        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("AGENTS.md"), "Error should mention AGENTS.md: {}", err);
    }

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
            None,
            "test-agent".to_string(),
        ).unwrap();

        let prompt = task.build_system_prompt().await.unwrap();
        assert!(prompt.contains("I am River"));
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

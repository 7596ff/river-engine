//! Spectator — event-driven sweep observer
//!
//! Listens for TurnComplete events, reads the home channel, and produces
//! narrative move summaries via LLM. One sweep, one move.
//! Moves stored in channels/home/{agent}/moves.jsonl.

pub mod format;
pub mod moves;
pub mod prompt;

use crate::channels::log::ChannelLog;
use crate::channels::writer::HomeChannelWriter;
use crate::channels::entry::HomeChannelEntry;
use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::model::ModelClient;
use chrono::Utc;
use river_core::{SnowflakeGenerator, SnowflakeType};
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration for the spectator task
#[derive(Debug, Clone)]
pub struct SpectatorConfig {
    /// Directory containing spectator prompt files
    pub spectator_dir: PathBuf,
    /// Path to the home channel JSONL
    pub home_channel_path: PathBuf,
    /// Path to moves.jsonl
    pub moves_path: PathBuf,
    /// Minimum time between sweeps (ignored during catch-up)
    pub sweep_interval: std::time::Duration,
    /// Max tokens for entries in a single sweep
    pub sweep_token_budget: usize,
    /// Number of recent moves to include as LLM context
    pub moves_tail: usize,
}

impl Default for SpectatorConfig {
    fn default() -> Self {
        Self {
            spectator_dir: PathBuf::from("spectator"),
            home_channel_path: PathBuf::from("channels/home/agent.jsonl"),
            moves_path: PathBuf::from("channels/home/agent/moves.jsonl"),
            sweep_interval: std::time::Duration::from_secs(300),
            sweep_token_budget: 16384,
            moves_tail: 10,
        }
    }
}

/// The spectator task
pub struct SpectatorTask {
    config: SpectatorConfig,
    bus: EventBus,
    model_client: ModelClient,
    home_channel_writer: HomeChannelWriter,
    snowflake_gen: Arc<SnowflakeGenerator>,
    /// Cached identity (system prompt)
    identity: String,
    /// Cached sweep prompt template
    on_sweep: Option<String>,
    /// Cached pressure prompt template
    on_pressure: Option<String>,
    /// Timestamp of last successful sweep
    last_sweep: Option<std::time::Instant>,
}

impl SpectatorTask {
    pub fn new(
        config: SpectatorConfig,
        bus: EventBus,
        model_client: ModelClient,
        home_channel_writer: HomeChannelWriter,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        Self {
            config,
            bus,
            model_client,
            home_channel_writer,
            snowflake_gen,
            identity: String::new(),
            on_sweep: None,
            on_pressure: None,
            last_sweep: None,
        }
    }

    /// Main run loop
    pub async fn run(mut self) {
        // Load identity — required
        let identity_path = self.config.spectator_dir.join("identity.md");
        self.identity = match prompt::load_prompt(&identity_path) {
            Some(id) => {
                tracing::info!("Spectator identity loaded from {:?}", identity_path);
                id
            }
            None => {
                tracing::error!("Spectator identity.md not found at {:?} — cannot start", identity_path);
                return;
            }
        };

        // Load prompts
        self.on_sweep = prompt::load_prompt(
            &self.config.spectator_dir.join("on-sweep.md"),
        );
        self.on_pressure = prompt::load_prompt(
            &self.config.spectator_dir.join("on-pressure.md"),
        );

        tracing::info!(
            sweep = self.on_sweep.is_some(),
            pressure = self.on_pressure.is_some(),
            "Spectator handlers loaded"
        );

        let mut event_rx = self.bus.subscribe();
        tracing::info!("Spectator task started");

        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(AgentEvent::TurnComplete { .. })) => {
                    self.maybe_sweep().await;
                }
                Ok(CoordinatorEvent::Agent(AgentEvent::ContextPressure { usage_percent, .. })) => {
                    self.handle_pressure(usage_percent).await;
                }
                Ok(CoordinatorEvent::Shutdown) => {
                    tracing::info!("Spectator: shutdown received");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "Event receive error");
                }
            }
        }

        tracing::info!("Spectator task stopped");
    }

    /// Check the time gate and sweep if enough time has passed
    async fn maybe_sweep(&mut self) {
        if self.on_sweep.is_none() {
            return;
        }

        // Check time gate (skip during catch-up — last_sweep is None on first run)
        if let Some(last) = self.last_sweep {
            if last.elapsed() < self.config.sweep_interval {
                return;
            }
        }

        // Run sweep loop (may iterate for catch-up)
        loop {
            let more = self.sweep().await;
            if !more {
                break;
            }
            tracing::info!("Catch-up sweep: more entries to process");
        }
    }

    /// Execute one sweep. Returns true if there are more entries to process (catch-up needed).
    async fn sweep(&mut self) -> bool {
        let template = match &self.on_sweep {
            Some(t) => t.clone(),
            None => return false,
        };

        // Read cursor
        let cursor = moves::read_cursor(&self.config.moves_path).await;

        // Read entries since cursor
        let log = ChannelLog::from_path(self.config.home_channel_path.clone());
        let entries = match log.read_home_since_opt(cursor).await {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(error = %e, "Failed to read home channel for sweep");
                return false;
            }
        };

        if entries.is_empty() {
            tracing::debug!("Sweep: no new entries");
            self.last_sweep = Some(std::time::Instant::now());
            return false;
        }

        // Format with token budget
        let (transcript, last_idx) = format::format_entries_budgeted(
            &entries, self.config.sweep_token_budget,
        );

        if transcript.is_empty() {
            // All entries were filtered (heartbeats/cursors/spectator messages)
            // Write a no-activity move to advance the cursor past them
            let first_id = entries[0].id();
            let last_id = entries.last().unwrap().id();
            if let Err(e) = moves::append_move(&self.config.moves_path, first_id, last_id, "[no activity]").await {
                tracing::error!(error = %e, "Failed to write no-activity move");
            }
            self.last_sweep = Some(std::time::Instant::now());
            return false;
        }

        let first_id = entries[0].id();
        let last_id = entries[last_idx].id();

        // Check if there are more non-filtered entries beyond what we included
        let remaining = &entries[last_idx + 1..];
        let has_more = remaining.iter().any(|e| format::format_entry(e).is_some());

        // Read recent moves for continuity
        let recent_moves = moves::read_moves_tail(&self.config.moves_path, self.config.moves_tail).await;
        let moves_text = if recent_moves.is_empty() {
            "No previous moves.".to_string()
        } else {
            recent_moves.iter().map(|m| m.summary.as_str()).collect::<Vec<_>>().join("\n\n")
        };

        // Build prompt
        let user_prompt = prompt::substitute(&template, &[
            ("recent_moves", &moves_text),
            ("entries", &transcript),
        ]);

        // Call LLM
        let summary = match self.call_model(&user_prompt).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(error = %e, "Sweep LLM call failed");
                return false;
            }
        };

        // Write move
        if let Err(e) = moves::append_move(&self.config.moves_path, first_id, last_id, &summary).await {
            tracing::error!(error = %e, "Failed to write move");
            return false;
        }

        // Write observability message to home channel
        let obs_msg = crate::channels::entry::MessageEntry::system_msg(
            self.snowflake_gen.next_id(SnowflakeType::Message),
            format!("[spectator] move written covering entries {}-{}", first_id, last_id),
        );
        self.home_channel_writer.write(HomeChannelEntry::Message(obs_msg)).await;

        // Clean up tool result files
        HomeChannelWriter::cleanup_tool_results(
            &self.config.home_channel_path, first_id, last_id,
        ).await;

        // Emit event
        self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated {
            channel: "home".to_string(),
            timestamp: Utc::now(),
        }));

        self.last_sweep = Some(std::time::Instant::now());

        tracing::info!(
            start = %first_id,
            end = %last_id,
            summary_len = summary.len(),
            has_more = has_more,
            "Sweep complete — move written"
        );

        has_more
    }

    async fn handle_pressure(&self, usage_percent: f64) {
        let template = match &self.on_pressure {
            Some(t) => t,
            None => return,
        };

        let user_prompt = prompt::substitute(template, &[
            ("usage_percent", &format!("{:.1}", usage_percent)),
        ]);

        if let Ok(warning) = self.call_model(&user_prompt).await {
            self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::Warning {
                content: warning,
                timestamp: Utc::now(),
            }));
        }
    }

    /// Call the model with the spectator's identity as system prompt.
    async fn call_model(&self, user_prompt: &str) -> Result<String, String> {
        use crate::model::ChatMessage;

        let messages = vec![
            ChatMessage::system(self.identity.clone()),
            ChatMessage::user(user_prompt.to_string()),
        ];

        let response = self
            .model_client
            .complete(&messages, &[])
            .await
            .map_err(|e| format!("Model error: {}", e))?;

        response
            .content
            .ok_or_else(|| "Model returned no content".to_string())
    }
}

//! Spectator — prompt-driven observing self
//!
//! The spectator is a thin event dispatcher. On each event it loads
//! a prompt file, assembles context, calls the LLM, and handles
//! the structured output.

pub mod format;
pub mod handlers;
pub mod prompt;

use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::r#loop::ModelClient;
use crate::session::PRIMARY_SESSION_ID;
use chrono::Utc;
use river_core::SnowflakeGenerator;
use river_db::Database;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Compression threshold: consider creating a moment when moves exceed this
const COMPRESSION_MOVES_THRESHOLD: usize = 50;

/// Configuration for the spectator task
#[derive(Debug, Clone)]
pub struct SpectatorConfig {
    /// Directory containing spectator prompt files
    pub spectator_dir: PathBuf,
    /// Directory for writing moment files
    pub moments_dir: PathBuf,
    /// Model timeout
    pub model_timeout: std::time::Duration,
}

/// The spectator task
pub struct SpectatorTask {
    config: SpectatorConfig,
    bus: EventBus,
    model_client: ModelClient,
    db: Arc<Mutex<Database>>,
    snowflake_gen: Arc<SnowflakeGenerator>,
    /// Cached identity (system prompt)
    identity: String,
    /// Cached prompt templates (None = handler disabled)
    on_turn_complete: Option<String>,
    on_compress: Option<String>,
    on_pressure: Option<String>,
}

impl SpectatorTask {
    pub fn new(
        config: SpectatorConfig,
        bus: EventBus,
        model_client: ModelClient,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        Self {
            config,
            bus,
            model_client,
            db,
            snowflake_gen,
            identity: String::new(),
            on_turn_complete: None,
            on_compress: None,
            on_pressure: None,
        }
    }

    /// Main run loop
    pub async fn run(mut self) {
        // Load identity — required, fail if missing
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

        // Load optional prompts
        self.on_turn_complete = prompt::load_prompt(
            &self.config.spectator_dir.join("on-turn-complete.md"),
        );
        self.on_compress = prompt::load_prompt(
            &self.config.spectator_dir.join("on-compress.md"),
        );
        self.on_pressure = prompt::load_prompt(
            &self.config.spectator_dir.join("on-pressure.md"),
        );

        tracing::info!(
            turn_complete = self.on_turn_complete.is_some(),
            compress = self.on_compress.is_some(),
            pressure = self.on_pressure.is_some(),
            "Spectator handlers loaded"
        );

        let mut event_rx = self.bus.subscribe();

        tracing::info!("Spectator task started");

        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
                    channel,
                    turn_number,
                    tool_calls,
                    ..
                })) => {
                    self.handle_turn_complete(&channel, turn_number, &tool_calls).await;
                }
                Ok(CoordinatorEvent::Agent(AgentEvent::ContextPressure {
                    usage_percent,
                    ..
                })) => {
                    self.handle_pressure(usage_percent).await;
                }
                Ok(CoordinatorEvent::Shutdown) => {
                    tracing::info!("Spectator: shutdown received");
                    break;
                }
                Ok(_) => {
                    // Ignore other events
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Event receive error");
                }
            }
        }

        tracing::info!("Spectator task stopped");
    }

    async fn handle_turn_complete(&self, channel: &str, turn_number: u64, tool_calls: &[String]) {
        let template = match &self.on_turn_complete {
            Some(t) => t,
            None => return,
        };

        // Lock-query-drop: get messages for this turn
        let messages = {
            let db = match self.db.lock() {
                Ok(db) => db,
                Err(e) => {
                    tracing::error!(error = %e, "DB lock poisoned");
                    return;
                }
            };
            match db.get_turn_messages(PRIMARY_SESSION_ID, turn_number) {
                Ok(msgs) => msgs,
                Err(e) => {
                    tracing::error!(turn = turn_number, error = %e, "Failed to query turn messages");
                    return;
                }
            }
        }; // MutexGuard dropped here

        if messages.is_empty() {
            tracing::error!(turn = turn_number, "No messages found for turn — skipping");
            return;
        }

        // Format transcript and substitute into prompt
        let transcript = format::format_transcript(&messages);
        let user_prompt = prompt::substitute(template, &[
            ("transcript", &transcript),
            ("turn_number", &turn_number.to_string()),
        ]);

        // Call LLM
        let summary = match self.call_model(&user_prompt).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(turn = turn_number, error = %e, "Model call failed, using fallback");
                format::fallback_summary(&messages)
            }
        };

        // Lock-query-drop: insert move
        {
            let db = match self.db.lock() {
                Ok(db) => db,
                Err(e) => {
                    tracing::error!(error = %e, "DB lock poisoned");
                    return;
                }
            };
            let m = river_db::Move {
                id: self.snowflake_gen.next_id(river_core::SnowflakeType::Embedding),
                channel: channel.to_string(),
                turn_number,
                summary: summary.clone(),
                tool_calls: Some(serde_json::to_string(tool_calls).unwrap_or_default()),
                created_at: Utc::now().timestamp(),
            };
            if let Err(e) = db.insert_move(&m) {
                tracing::error!(error = %e, "Failed to insert move");
                return;
            }
        }; // MutexGuard dropped here

        // Emit event
        self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated {
            channel: channel.to_string(),
            timestamp: Utc::now(),
        }));

        tracing::debug!(turn = turn_number, channel = %channel, "Move recorded");

        // Check compression threshold
        if self.on_compress.is_some() {
            let count = {
                let db = self.db.lock().unwrap();
                db.count_moves(channel).unwrap_or(0)
            };
            if count > COMPRESSION_MOVES_THRESHOLD {
                self.handle_compress(channel).await;
            }
        }
    }

    async fn handle_compress(&self, channel: &str) {
        let template = match &self.on_compress {
            Some(t) => t,
            None => return,
        };

        // Lock-query-drop: get all moves
        let moves = {
            let db = self.db.lock().unwrap();
            match db.get_moves(channel, 10_000) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to query moves for compression");
                    return;
                }
            }
        };

        let moves_text = format::format_moves(&moves);
        let user_prompt = prompt::substitute(template, &[
            ("moves", &moves_text),
            ("channel", channel),
        ]);

        // Call LLM
        let response_text = match self.call_model(&user_prompt).await {
            Ok(text) => text,
            Err(e) => {
                tracing::error!(error = %e, "Model call failed for compression");
                return;
            }
        };

        // Parse — strict, no fallback
        let moment = match handlers::parse_moment_response(&response_text) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "Failed to parse moment response");
                return;
            }
        };

        // Write moment file
        let timestamp = Utc::now();
        let sanitized_channel = channel.replace(['/', '\\', ' '], "-");
        let filename = format!("{}-{}.md", sanitized_channel, timestamp.format("%Y%m%d%H%M%S"));
        let moment_path = self.config.moments_dir.join(&filename);

        let content = format!(
            "---\nchannel: {}\nturns: {}-{}\ncreated: {}\nauthor: spectator\ntype: moment\n---\n\n{}",
            channel,
            moment.start_turn,
            moment.end_turn,
            timestamp.to_rfc3339(),
            moment.narrative,
        );

        if let Err(e) = tokio::fs::create_dir_all(&self.config.moments_dir).await {
            tracing::error!(error = %e, "Failed to create moments directory");
            return;
        }

        if let Err(e) = tokio::fs::write(&moment_path, &content).await {
            tracing::error!(error = %e, "Failed to write moment file");
            return;
        }

        self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MomentCreated {
            summary: moment.narrative,
            timestamp,
        }));

        tracing::info!(
            channel = %channel,
            turns = format!("{}-{}", moment.start_turn, moment.end_turn),
            path = %moment_path.display(),
            "Moment created"
        );
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
        use crate::r#loop::context::ChatMessage;

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

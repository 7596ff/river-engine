//! Spectator — prompt-driven observing self
//!
//! The spectator is a thin event dispatcher. On each event it loads
//! a prompt file, assembles context, calls the LLM, and handles
//! the structured output.
//!
//! Moves are stored as files at channels/home/{agent}/moves/{start}-{end}.md

pub mod handlers;
pub mod prompt;

use crate::channels::writer::HomeChannelWriter;
use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::model::ModelClient;
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration for the spectator task
#[derive(Debug, Clone)]
pub struct SpectatorConfig {
    /// Directory containing spectator prompt files
    pub spectator_dir: PathBuf,
    /// Directory for writing moment files
    pub moments_dir: PathBuf,
    /// Directory for writing move files (channels/home/{agent}/moves/)
    pub moves_dir: PathBuf,
    /// Path to the home channel JSONL (for tool result cleanup)
    pub home_channel_path: PathBuf,
    /// Model timeout
    pub model_timeout: std::time::Duration,
}

/// The spectator task
pub struct SpectatorTask {
    config: SpectatorConfig,
    bus: EventBus,
    model_client: ModelClient,
    /// Cached identity (system prompt)
    identity: String,
    /// Cached prompt templates (None = handler disabled)
    on_turn_complete: Option<String>,
    on_pressure: Option<String>,
}

impl SpectatorTask {
    pub fn new(
        config: SpectatorConfig,
        bus: EventBus,
        model_client: ModelClient,
    ) -> Self {
        Self {
            config,
            bus,
            model_client,
            identity: String::new(),
            on_turn_complete: None,
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
        self.on_pressure = prompt::load_prompt(
            &self.config.spectator_dir.join("on-pressure.md"),
        );

        tracing::info!(
            turn_complete = self.on_turn_complete.is_some(),
            pressure = self.on_pressure.is_some(),
            "Spectator handlers loaded"
        );

        let mut event_rx = self.bus.subscribe();

        tracing::info!("Spectator task started");

        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
                    turn_number,
                    transcript_summary,
                    tool_calls,
                    first_snowflake,
                    last_snowflake,
                    ..
                })) => {
                    self.handle_turn_complete(
                        turn_number, &transcript_summary, &tool_calls,
                        first_snowflake.as_deref(), last_snowflake.as_deref(),
                    ).await;
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

    async fn handle_turn_complete(
        &self,
        turn_number: u64,
        transcript_summary: &str,
        tool_calls: &[String],
        first_snowflake: Option<&str>,
        last_snowflake: Option<&str>,
    ) {
        let template = match &self.on_turn_complete {
            Some(t) => t,
            None => return,
        };

        // Use transcript summary as the prompt context
        let user_prompt = prompt::substitute(template, &[
            ("transcript", transcript_summary),
            ("turn_number", &turn_number.to_string()),
        ]);

        // Call LLM for move summary
        let summary = match self.call_model(&user_prompt).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(turn = turn_number, error = %e, "Model call failed, using transcript summary");
                transcript_summary.to_string()
            }
        };

        // Write move file with snowflake range
        let (start, end) = match (first_snowflake, last_snowflake) {
            (Some(s), Some(e)) => (s.to_string(), e.to_string()),
            _ => {
                tracing::warn!(turn = turn_number, "No snowflake range for turn, skipping move");
                return;
            }
        };

        let filename = format!("{}-{}.md", start, end);
        let move_path = self.config.moves_dir.join(&filename);

        let content = format!(
            "Turn {}: {}\n\nTools: {}",
            turn_number,
            summary,
            if tool_calls.is_empty() { "none".to_string() } else { tool_calls.join(", ") },
        );

        if let Err(e) = tokio::fs::create_dir_all(&self.config.moves_dir).await {
            tracing::error!(error = %e, "Failed to create moves directory");
            return;
        }

        if let Err(e) = tokio::fs::write(&move_path, &content).await {
            tracing::error!(error = %e, "Failed to write move file");
            return;
        }

        // Emit event
        self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated {
            channel: "home".to_string(),
            timestamp: Utc::now(),
        }));

        // Clean up tool result files in the covered snowflake range
        HomeChannelWriter::cleanup_tool_results(
            &self.config.home_channel_path, &start, &end,
        ).await;

        tracing::debug!(
            turn = turn_number,
            path = %move_path.display(),
            "Move recorded to file"
        );
    }

    // NOTE: Compression (moments from moves) is deferred until the spectator
    // learns to read the home channel directly. The file-based move storage
    // in load_moves() provides the data; compression just needs an LLM call
    // over accumulated move files.

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

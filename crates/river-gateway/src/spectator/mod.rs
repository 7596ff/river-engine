//! Spectator task — the observing self (You)
//!
//! The spectator watches agent turn transcripts, compresses conversations
//! into moves and moments, curates memories by pushing flashes, and writes
//! room notes as witness observations.

pub mod compress;
pub mod curate;
pub mod room;

use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::embeddings::VectorStore;
use crate::flash::FlashQueue;
use crate::r#loop::ModelClient;
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use compress::Compressor;
pub use curate::Curator;
pub use room::RoomWriter;

/// Configuration for the spectator task
#[derive(Debug, Clone)]
pub struct SpectatorConfig {
    /// Workspace path
    pub workspace: PathBuf,
    /// Directory containing embeddings/notes
    pub embeddings_dir: PathBuf,
    /// Model URL for spectator (may differ from agent's model)
    pub model_url: String,
    /// Model name (e.g., "llama-3-8b" or "claude-sonnet")
    pub model_name: String,
    /// Identity file path
    pub identity_path: PathBuf,
    /// Rules file path
    pub rules_path: PathBuf,
    /// Model timeout
    pub model_timeout: Duration,
}

impl SpectatorConfig {
    /// Create config with default paths based on workspace
    pub fn from_workspace(workspace: PathBuf, model_url: String, model_name: String) -> Self {
        Self {
            embeddings_dir: workspace.join("embeddings"),
            identity_path: workspace.join("spectator/IDENTITY.md"),
            rules_path: workspace.join("spectator/RULES.md"),
            workspace,
            model_url,
            model_name,
            model_timeout: Duration::from_secs(60),
        }
    }
}

/// The spectator task — observes, compresses, curates
pub struct SpectatorTask {
    config: SpectatorConfig,
    bus: EventBus,
    #[allow(dead_code)] // Used when model calls are enabled
    model_client: ModelClient,
    vector_store: Option<Arc<VectorStore>>,
    #[allow(dead_code)] // Referenced by curator
    flash_queue: Arc<FlashQueue>,
    compressor: Compressor,
    curator: Curator,
    room_writer: RoomWriter,
    /// Cached identity text
    identity: Option<String>,
}

impl SpectatorTask {
    pub fn new(
        config: SpectatorConfig,
        bus: EventBus,
        model_client: ModelClient,
        vector_store: Option<Arc<VectorStore>>,
        flash_queue: Arc<FlashQueue>,
    ) -> Self {
        let compressor = Compressor::new(config.embeddings_dir.clone());
        let curator = Curator::new(flash_queue.clone());
        let room_writer = RoomWriter::new(config.embeddings_dir.join("room-notes"));

        Self {
            config,
            bus,
            model_client,
            vector_store,
            flash_queue,
            compressor,
            curator,
            room_writer,
            identity: None,
        }
    }

    /// Main run loop
    pub async fn run(mut self) {
        let mut event_rx = self.bus.subscribe();

        // Load identity once at startup
        self.identity = Some(self.load_identity().await);

        tracing::info!("Spectator task started");

        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(event)) => {
                    let identity = self.identity.clone().unwrap_or_default();
                    self.observe(event, &identity).await;
                }
                Ok(CoordinatorEvent::Shutdown) => {
                    tracing::info!("Spectator task: shutdown received");
                    break;
                }
                Ok(CoordinatorEvent::Spectator(_)) => {
                    // Ignore own events
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Event receive error");
                }
            }
        }

        tracing::info!("Spectator task stopped");
    }

    /// Process an agent event
    async fn observe(&mut self, event: AgentEvent, identity: &str) {
        match event {
            AgentEvent::TurnComplete {
                channel,
                turn_number,
                transcript_summary,
                tool_calls,
                ..
            } => {
                tracing::debug!(turn = turn_number, channel = %channel, "Spectator observing turn");

                // Job 1: Compress — update moves for this channel
                if let Err(e) = self.compressor.update_moves(
                    &channel,
                    turn_number,
                    &transcript_summary,
                    &tool_calls,
                    &self.model_client,
                    identity,
                ).await {
                    tracing::error!(error = %e, "Failed to update moves");
                }

                // Job 2: Curate — search for relevant memories and push flashes
                if let Some(ref store) = self.vector_store {
                    if let Err(e) = self.curator.curate(
                        &transcript_summary,
                        store,
                        &self.bus,
                    ).await {
                        tracing::error!(error = %e, "Failed to curate");
                    }
                }

                // Job 3: Room notes — write witness observation
                if let Err(e) = self.room_writer.write_observation(
                    turn_number,
                    &transcript_summary,
                    &self.model_client,
                    identity,
                ).await {
                    tracing::error!(error = %e, "Failed to write room note");
                }

                // Emit MovesUpdated
                self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated {
                    channel: channel.clone(),
                    timestamp: Utc::now(),
                }));
            }

            AgentEvent::NoteWritten { path, .. } => {
                tracing::debug!(path = %path, "Spectator: agent wrote note");
                // Could trigger re-indexing or review
            }

            AgentEvent::ContextPressure { usage_percent, .. } => {
                if usage_percent > 85.0 {
                    self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::Warning {
                        content: format!(
                            "Context at {:.0}% — consider rotation",
                            usage_percent
                        ),
                        timestamp: Utc::now(),
                    }));
                    tracing::warn!(
                        usage_percent = format!("{:.0}", usage_percent),
                        "Spectator warning: high context pressure"
                    );
                }
            }

            AgentEvent::TurnStarted { .. } => {
                // Could use for timing analysis
            }

            AgentEvent::ChannelSwitched { from, to, .. } => {
                tracing::debug!(from = %from, to = %to, "Spectator: channel switched");
            }
        }
    }

    /// Load spectator identity and rules
    async fn load_identity(&self) -> String {
        let identity = tokio::fs::read_to_string(&self.config.identity_path).await
            .unwrap_or_else(|_| {
                tracing::warn!(path = %self.config.identity_path.display(), "Identity file not found");
                "You observe. You do not act.".to_string()
            });

        let rules = tokio::fs::read_to_string(&self.config.rules_path).await
            .unwrap_or_else(|_| {
                tracing::warn!(path = %self.config.rules_path.display(), "Rules file not found");
                String::new()
            });

        if rules.is_empty() {
            identity
        } else {
            format!("{}\n\n---\n\n{}", identity, rules)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::Coordinator;
    use tempfile::TempDir;

    fn test_config(temp: &TempDir) -> SpectatorConfig {
        SpectatorConfig::from_workspace(
            temp.path().to_path_buf(),
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
        )
    }

    fn test_model_client() -> ModelClient {
        ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        ).unwrap()
    }

    #[test]
    fn test_spectator_config_from_workspace() {
        let config = SpectatorConfig::from_workspace(
            PathBuf::from("/workspace"),
            "http://model:8080".to_string(),
            "llama".to_string(),
        );

        assert_eq!(config.workspace, PathBuf::from("/workspace"));
        assert_eq!(config.embeddings_dir, PathBuf::from("/workspace/embeddings"));
        assert_eq!(config.identity_path, PathBuf::from("/workspace/spectator/IDENTITY.md"));
        assert_eq!(config.rules_path, PathBuf::from("/workspace/spectator/RULES.md"));
    }

    #[tokio::test]
    async fn test_spectator_observes_turn_complete() {
        let temp = TempDir::new().unwrap();
        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();
        let mut rx = bus.subscribe();

        let flash_queue = Arc::new(FlashQueue::new(10));
        let model = test_model_client();

        let mut spectator = SpectatorTask::new(
            config,
            bus.clone(),
            model,
            None,
            flash_queue,
        );

        // Simulate loading identity
        spectator.identity = Some("Test identity".to_string());

        // Observe a TurnComplete event
        let event = AgentEvent::TurnComplete {
            channel: "general".to_string(),
            turn_number: 1,
            transcript_summary: "User asked a question".to_string(),
            tool_calls: vec![],
            timestamp: Utc::now(),
        };

        spectator.observe(event, "Test identity").await;

        // Should emit MovesUpdated
        let response = rx.try_recv();
        assert!(matches!(
            response,
            Ok(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated { .. }))
        ));

        // Check moves file was created
        let moves_path = temp.path().join("embeddings/moves/general.md");
        assert!(moves_path.exists());
    }

    #[tokio::test]
    async fn test_spectator_emits_warning_on_high_pressure() {
        let temp = TempDir::new().unwrap();
        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();
        let mut rx = bus.subscribe();

        let flash_queue = Arc::new(FlashQueue::new(10));
        let model = test_model_client();

        let mut spectator = SpectatorTask::new(
            config,
            bus.clone(),
            model,
            None,
            flash_queue,
        );

        spectator.identity = Some("Test".to_string());

        // Simulate high context pressure
        let event = AgentEvent::ContextPressure {
            usage_percent: 90.0,
            timestamp: Utc::now(),
        };

        spectator.observe(event, "Test").await;

        // Should emit Warning
        let response = rx.try_recv();
        assert!(matches!(
            response,
            Ok(CoordinatorEvent::Spectator(SpectatorEvent::Warning { .. }))
        ));
    }

    #[tokio::test]
    async fn test_load_identity_with_files() {
        let temp = TempDir::new().unwrap();

        // Create identity files
        let spectator_dir = temp.path().join("spectator");
        tokio::fs::create_dir_all(&spectator_dir).await.unwrap();
        tokio::fs::write(spectator_dir.join("IDENTITY.md"), "I observe").await.unwrap();
        tokio::fs::write(spectator_dir.join("RULES.md"), "Never act").await.unwrap();

        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();
        let flash_queue = Arc::new(FlashQueue::new(10));
        let model = test_model_client();

        let spectator = SpectatorTask::new(config, bus, model, None, flash_queue);
        let identity = spectator.load_identity().await;

        assert!(identity.contains("I observe"));
        assert!(identity.contains("Never act"));
    }

    #[tokio::test]
    async fn test_load_identity_missing_files() {
        let temp = TempDir::new().unwrap();
        let config = test_config(&temp);
        let coord = Coordinator::new();
        let bus = coord.bus().clone();
        let flash_queue = Arc::new(FlashQueue::new(10));
        let model = test_model_client();

        let spectator = SpectatorTask::new(config, bus, model, None, flash_queue);
        let identity = spectator.load_identity().await;

        // Should use default
        assert!(identity.contains("observe"));
    }
}

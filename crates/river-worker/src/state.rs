//! Worker state.

use crate::config::WorkerConfig;
use river_adapter::{Baton, Channel, Ground, Side};
use river_context::Flash;
use river_protocol::{ModelConfig, Registry, WorkerRegistrationResponse};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Notification about new messages.
#[derive(Debug, Clone)]
pub struct Notification {
    pub channel: Channel,
    pub count: usize,
}

/// Worker state.
#[derive(Debug)]
pub struct WorkerState {
    // Identity
    pub dyad: String,
    pub side: Side,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub ground: Ground,
    pub workspace: PathBuf,

    // Communication
    pub current_channel: Channel,
    pub watch_list: HashSet<String>, // Channel keys: "adapter:id"

    // Registry
    pub registry: Registry,

    // Model
    pub model_config: ModelConfig,
    pub token_count: usize,
    pub context_limit: usize,

    // Loop control
    pub sleeping: bool,
    pub sleep_until: Option<Instant>,
    pub pending_notifications: Vec<Notification>,
    pub pending_flashes: Vec<Flash>,

    // Role switching
    pub switch_pending: bool,

    // Initial context (loaded from files at startup)
    pub role_content: Option<String>,
    pub identity_content: Option<String>,
    pub initial_message: Option<String>,
}

impl WorkerState {
    /// Create initial state from config and registration.
    pub fn new(config: &WorkerConfig, registration: WorkerRegistrationResponse) -> Self {
        Self {
            dyad: config.dyad.clone(),
            side: config.side.clone(),
            baton: registration.baton,
            partner_endpoint: registration.partner_endpoint,
            ground: registration.ground.clone(),
            workspace: PathBuf::from(&registration.workspace),
            current_channel: registration.ground.channel.clone(),
            watch_list: HashSet::new(),
            registry: Registry::default(),
            model_config: registration.model,
            token_count: 0,
            context_limit: 0, // Will be set from model config
            sleeping: registration.start_sleeping,
            sleep_until: None,
            pending_notifications: Vec::new(),
            pending_flashes: Vec::new(),
            switch_pending: false,
            role_content: None,
            identity_content: None,
            initial_message: None,
        }
    }

    /// Get channel key for watch list.
    pub fn channel_key(channel: &Channel) -> String {
        format!("{}:{}", channel.adapter, channel.id)
    }

    /// Check if a channel is in the watch list.
    pub fn is_watched(&self, channel: &Channel) -> bool {
        self.watch_list.contains(&Self::channel_key(channel))
    }

    /// Add channel to watch list.
    pub fn watch(&mut self, channel: &Channel) {
        self.watch_list.insert(Self::channel_key(channel));
    }

    /// Remove channel from watch list.
    pub fn unwatch(&mut self, channel: &Channel) {
        self.watch_list.remove(&Self::channel_key(channel));
    }

    /// Get partner side.
    pub fn partner_side(&self) -> Side {
        self.side.opposite()
    }
}

/// Thread-safe state wrapper.
pub type SharedState = Arc<RwLock<WorkerState>>;

pub fn new_shared_state(config: &WorkerConfig, registration: WorkerRegistrationResponse) -> SharedState {
    let state = WorkerState::new(config, registration);
    Arc::new(RwLock::new(state))
}

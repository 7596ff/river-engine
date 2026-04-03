//! Worker state.

use crate::config::{ModelConfig, RegistrationResponse, WorkerConfig};
use river_adapter::{Baton, Channel, Ground, Side};
use river_context::Flash;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Process entry from registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProcessEntry {
    Worker {
        endpoint: String,
        dyad: String,
        side: Side,
        baton: Baton,
        model: String,
        ground: Ground,
    },
    Adapter {
        endpoint: String,
        #[serde(rename = "type")]
        adapter_type: String,
        dyad: String,
        features: Vec<u16>,
    },
    EmbedService {
        endpoint: String,
        name: String,
    },
}

/// Registry of all processes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    pub processes: Vec<ProcessEntry>,
}

impl Registry {
    /// Find embed service endpoint.
    pub fn embed_endpoint(&self) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::EmbedService { endpoint, .. } => Some(endpoint.as_str()),
            _ => None,
        })
    }

    /// Find adapter endpoint by type.
    pub fn adapter_endpoint(&self, adapter_type: &str) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::Adapter {
                endpoint,
                adapter_type: t,
                ..
            } if t == adapter_type => Some(endpoint.as_str()),
            _ => None,
        })
    }

    /// Find worker endpoint by dyad and side.
    pub fn worker_endpoint(&self, dyad: &str, side: &Side) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::Worker {
                endpoint,
                dyad: d,
                side: s,
                ..
            } if d == dyad && s == side => Some(endpoint.as_str()),
            _ => None,
        })
    }
}

/// Notification about new messages.
#[derive(Debug, Clone)]
pub struct Notification {
    pub channel: Channel,
    pub count: usize,
    pub since_id: Option<String>,
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
    pub fn new(config: &WorkerConfig, registration: RegistrationResponse) -> Self {
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
        match self.side {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Thread-safe state wrapper.
pub type SharedState = Arc<RwLock<WorkerState>>;

pub fn new_shared_state(config: &WorkerConfig, registration: RegistrationResponse) -> SharedState {
    let state = WorkerState::new(config, registration);
    Arc::new(RwLock::new(state))
}

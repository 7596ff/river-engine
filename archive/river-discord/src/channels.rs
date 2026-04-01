//! Channel state management

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Persisted channel state format
#[derive(Debug, Serialize, Deserialize)]
struct PersistedState {
    version: u32,
    channels: Vec<u64>,
}

/// Thread-safe channel state
pub struct ChannelState {
    channels: RwLock<HashSet<u64>>,
    state_file: Option<std::path::PathBuf>,
}

impl ChannelState {
    /// Create new channel state with initial channels
    pub fn new(initial_channels: Vec<u64>, state_file: Option<std::path::PathBuf>) -> Arc<Self> {
        let channels: HashSet<u64> = initial_channels.into_iter().collect();
        Arc::new(Self {
            channels: RwLock::new(channels),
            state_file,
        })
    }

    /// Load state from file, falling back to initial channels
    pub async fn load(
        initial_channels: Vec<u64>,
        state_file: Option<std::path::PathBuf>,
    ) -> Arc<Self> {
        let channels = if let Some(ref path) = state_file {
            Self::load_from_file(path).unwrap_or_else(|| initial_channels.into_iter().collect())
        } else {
            initial_channels.into_iter().collect()
        };

        Arc::new(Self {
            channels: RwLock::new(channels),
            state_file,
        })
    }

    fn load_from_file(path: &Path) -> Option<HashSet<u64>> {
        let content = std::fs::read_to_string(path).ok()?;
        let state: PersistedState = serde_json::from_str(&content).ok()?;
        if state.version != 1 {
            tracing::warn!("Unknown state file version, ignoring");
            return None;
        }
        Some(state.channels.into_iter().collect())
    }

    /// Check if a channel is being listened to
    pub async fn contains(&self, channel_id: u64) -> bool {
        self.channels.read().await.contains(&channel_id)
    }

    /// Add a channel to the listen set
    pub async fn add(&self, channel_id: u64) -> bool {
        let mut channels = self.channels.write().await;
        let added = channels.insert(channel_id);
        if added {
            drop(channels);
            self.persist().await;
        }
        added
    }

    /// Remove a channel from the listen set
    pub async fn remove(&self, channel_id: u64) -> bool {
        let mut channels = self.channels.write().await;
        let removed = channels.remove(&channel_id);
        if removed {
            drop(channels);
            self.persist().await;
        }
        removed
    }

    /// Get all channel IDs
    pub async fn list(&self) -> Vec<u64> {
        self.channels.read().await.iter().copied().collect()
    }

    /// Get channel count
    pub async fn count(&self) -> usize {
        self.channels.read().await.len()
    }

    /// Persist state to file (atomic write)
    async fn persist(&self) {
        let Some(ref path) = self.state_file else {
            return;
        };

        let channels: Vec<u64> = self.channels.read().await.iter().copied().collect();
        let state = PersistedState {
            version: 1,
            channels,
        };

        let content = match serde_json::to_string_pretty(&state) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to serialize state: {}", e);
                return;
            }
        };

        // Atomic write: write to temp file then rename
        let temp_path = path.with_extension("tmp");
        if let Err(e) = std::fs::write(&temp_path, &content) {
            tracing::error!("Failed to write state file: {}", e);
            return;
        }
        if let Err(e) = std::fs::rename(&temp_path, path) {
            tracing::error!("Failed to rename state file: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_channel_state_basic() {
        let state = ChannelState::new(vec![1, 2, 3], None);

        assert!(state.contains(1).await);
        assert!(state.contains(2).await);
        assert!(!state.contains(99).await);
        assert_eq!(state.count().await, 3);
    }

    #[tokio::test]
    async fn test_channel_state_add_remove() {
        let state = ChannelState::new(vec![], None);

        assert!(state.add(100).await);
        assert!(state.contains(100).await);
        assert!(!state.add(100).await); // already exists

        assert!(state.remove(100).await);
        assert!(!state.contains(100).await);
        assert!(!state.remove(100).await); // already removed
    }

    #[tokio::test]
    async fn test_channel_state_persistence() {
        let dir = tempdir().unwrap();
        let state_file = dir.path().join("channels.json");

        // Create and populate state
        {
            let state = ChannelState::new(vec![], Some(state_file.clone()));
            state.add(111).await;
            state.add(222).await;
        }

        // Load state from file
        let state = ChannelState::load(vec![], Some(state_file)).await;
        assert!(state.contains(111).await);
        assert!(state.contains(222).await);
        assert_eq!(state.count().await, 2);
    }

    #[tokio::test]
    async fn test_channel_state_list() {
        let state = ChannelState::new(vec![5, 3, 1], None);
        let mut list = state.list().await;
        list.sort();
        assert_eq!(list, vec![1, 3, 5]);
    }
}

//! Adapter registry for managing external communication adapters

use river_adapter::{Adapter, AdapterInfo, HttpAdapter, RegisterResponse};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Registry of connected adapters
pub struct AdapterRegistry {
    adapters: RwLock<HashMap<String, Arc<dyn Adapter>>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new adapter
    pub async fn register(&self, info: AdapterInfo) -> RegisterResponse {
        let name = info.name.clone();
        let adapter = Arc::new(HttpAdapter::new(info));

        let mut adapters = self.adapters.write().await;
        adapters.insert(name.clone(), adapter);

        tracing::info!(adapter = %name, "Adapter registered");

        RegisterResponse {
            accepted: true,
            error: None,
        }
    }

    /// Get an adapter by name
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Adapter>> {
        self.adapters.read().await.get(name).cloned()
    }

    /// List all registered adapter names
    pub async fn list(&self) -> Vec<String> {
        self.adapters.read().await.keys().cloned().collect()
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

//! Registry state and push mechanism.

use river_protocol::{Baton, Ground, ProcessEntry, Registry, Side};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Timeout for registry push operations.
const REGISTRY_PUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Key for identifying a worker.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct WorkerKey {
    pub dyad: String,
    pub side: Side,
}

/// Key for identifying an adapter.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct AdapterKey {
    pub dyad: String,
    pub adapter_type: String,
}

/// Internal registry state.
#[derive(Debug, Default)]
pub struct RegistryState {
    workers: HashMap<WorkerKey, ProcessEntry>,
    adapters: HashMap<AdapterKey, ProcessEntry>,
    embed_services: HashMap<String, ProcessEntry>,
}

impl RegistryState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or update a worker.
    pub fn register_worker(
        &mut self,
        dyad: String,
        side: Side,
        endpoint: String,
        baton: Baton,
        model: String,
        ground: Ground,
    ) {
        let key = WorkerKey {
            dyad: dyad.clone(),
            side: side.clone(),
        };
        let entry = ProcessEntry::Worker {
            endpoint,
            dyad,
            side,
            baton,
            model,
            ground,
        };
        self.workers.insert(key, entry);
    }

    /// Register or update an adapter.
    pub fn register_adapter(
        &mut self,
        dyad: String,
        adapter_type: String,
        endpoint: String,
        features: Vec<u16>,
    ) {
        let key = AdapterKey {
            dyad: dyad.clone(),
            adapter_type: adapter_type.clone(),
        };
        let entry = ProcessEntry::Adapter {
            endpoint,
            adapter_type,
            dyad,
            features,
        };
        self.adapters.insert(key, entry);
    }

    /// Register or update an embed service.
    pub fn register_embed(&mut self, name: String, endpoint: String) {
        let entry = ProcessEntry::EmbedService {
            endpoint,
            name: name.clone(),
        };
        self.embed_services.insert(name, entry);
    }

    /// Update a worker's baton.
    pub fn update_worker_baton(&mut self, dyad: &str, side: &Side, new_baton: Baton) -> bool {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        if let Some(ProcessEntry::Worker { baton, .. }) = self.workers.get_mut(&key) {
            *baton = new_baton;
            true
        } else {
            false
        }
    }

    /// Update a worker's model.
    pub fn update_worker_model(&mut self, dyad: &str, side: &Side, new_model: String) -> bool {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        if let Some(ProcessEntry::Worker { model, .. }) = self.workers.get_mut(&key) {
            *model = new_model;
            true
        } else {
            false
        }
    }

    /// Remove a worker from registry.
    pub fn remove_worker(&mut self, dyad: &str, side: &Side) {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        self.workers.remove(&key);
    }

    /// Remove an adapter from registry.
    pub fn remove_adapter(&mut self, dyad: &str, adapter_type: &str) {
        let key = AdapterKey {
            dyad: dyad.to_string(),
            adapter_type: adapter_type.to_string(),
        };
        self.adapters.remove(&key);
    }

    /// Remove an embed service from registry.
    pub fn remove_embed(&mut self, name: &str) {
        self.embed_services.remove(name);
    }

    /// Get worker endpoint.
    pub fn get_worker_endpoint(&self, dyad: &str, side: &Side) -> Option<String> {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        self.workers.get(&key).map(|e| e.endpoint().to_string())
    }

    /// Get partner worker endpoint.
    pub fn get_partner_endpoint(&self, dyad: &str, side: &Side) -> Option<String> {
        let partner_side = side.opposite();
        self.get_worker_endpoint(dyad, &partner_side)
    }

    /// Get worker's current baton.
    pub fn get_worker_baton(&self, dyad: &str, side: &Side) -> Option<Baton> {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        self.workers.get(&key).and_then(|e| {
            if let ProcessEntry::Worker { baton, .. } = e {
                Some(baton.clone())
            } else {
                None
            }
        })
    }

    /// Get embed service endpoint by name.
    /// Note: Not called internally - orchestrator pushes Registry to workers who use embed_endpoint().
    #[allow(dead_code)]
    pub fn get_embed_endpoint(&self, name: &str) -> Option<String> {
        self.embed_services.get(name).map(|e| e.endpoint().to_string())
    }

    /// Build the registry snapshot for pushing.
    pub fn build_registry(&self) -> Registry {
        let mut processes = Vec::new();
        processes.extend(self.workers.values().cloned());
        processes.extend(self.adapters.values().cloned());
        processes.extend(self.embed_services.values().cloned());
        Registry { processes }
    }

    /// Get all endpoints for pushing.
    pub fn all_endpoints(&self) -> Vec<String> {
        let mut endpoints = Vec::new();
        for entry in self.workers.values() {
            endpoints.push(entry.endpoint().to_string());
        }
        for entry in self.adapters.values() {
            endpoints.push(entry.endpoint().to_string());
        }
        for entry in self.embed_services.values() {
            endpoints.push(entry.endpoint().to_string());
        }
        endpoints
    }

    /// Get worker count.
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Get adapter count.
    pub fn adapter_count(&self) -> usize {
        self.adapters.len()
    }

    /// Get embed service count.
    pub fn embed_count(&self) -> usize {
        self.embed_services.len()
    }
}

/// Push registry to all endpoints.
pub async fn push_registry(
    client: &reqwest::Client,
    registry: &Registry,
    endpoints: &[String],
) {
    for endpoint in endpoints {
        let url = format!("{}/registry", endpoint);
        let registry_clone = registry.clone();
        let client_clone = client.clone();

        // Fire and forget - don't wait for responses
        tokio::spawn(async move {
            if let Err(e) = client_clone
                .post(&url)
                .json(&registry_clone)
                .timeout(REGISTRY_PUSH_TIMEOUT)
                .send()
                .await
            {
                tracing::warn!("Failed to push registry to {}: {}", url, e);
            }
        });
    }
}

/// Thread-safe registry wrapper.
pub type SharedRegistry = Arc<RwLock<RegistryState>>;

pub fn new_shared_registry() -> SharedRegistry {
    Arc::new(RwLock::new(RegistryState::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ground() -> Ground {
        Ground {
            name: Some("Test User".into()),
            id: "user123".into(),
            adapter: "discord".into(),
            channel: "ch123".into(),
        }
    }

    #[test]
    fn test_get_worker_baton() {
        let mut state = RegistryState::new();

        // Initially no worker
        assert_eq!(state.get_worker_baton("dyad1", &Side::Left), None);

        // Register a worker as Actor
        state.register_worker(
            "dyad1".into(),
            Side::Left,
            "http://localhost:3001".into(),
            Baton::Actor,
            "gpt-4".into(),
            test_ground(),
        );

        // Should get Actor baton
        assert_eq!(state.get_worker_baton("dyad1", &Side::Left), Some(Baton::Actor));

        // Other side still None
        assert_eq!(state.get_worker_baton("dyad1", &Side::Right), None);
    }

    #[test]
    fn test_baton_swap() {
        let mut state = RegistryState::new();
        let ground = test_ground();

        // Register both workers
        state.register_worker(
            "dyad1".into(),
            Side::Left,
            "http://localhost:3001".into(),
            Baton::Actor,
            "gpt-4".into(),
            ground.clone(),
        );
        state.register_worker(
            "dyad1".into(),
            Side::Right,
            "http://localhost:3002".into(),
            Baton::Spectator,
            "gpt-4".into(),
            ground,
        );

        // Verify initial state
        assert_eq!(state.get_worker_baton("dyad1", &Side::Left), Some(Baton::Actor));
        assert_eq!(state.get_worker_baton("dyad1", &Side::Right), Some(Baton::Spectator));

        // Swap batons
        let left_baton = state.get_worker_baton("dyad1", &Side::Left).unwrap();
        let right_baton = state.get_worker_baton("dyad1", &Side::Right).unwrap();

        state.update_worker_baton("dyad1", &Side::Left, right_baton);
        state.update_worker_baton("dyad1", &Side::Right, left_baton);

        // Verify swapped state
        assert_eq!(state.get_worker_baton("dyad1", &Side::Left), Some(Baton::Spectator));
        assert_eq!(state.get_worker_baton("dyad1", &Side::Right), Some(Baton::Actor));
    }

    #[test]
    fn test_update_worker_baton_nonexistent() {
        let mut state = RegistryState::new();

        // Should return false for nonexistent worker
        assert!(!state.update_worker_baton("dyad1", &Side::Left, Baton::Actor));
    }
}

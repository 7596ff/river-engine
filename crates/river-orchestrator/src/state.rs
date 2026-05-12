//! Shared application state

use crate::agents::{AgentInfo, AgentStatus};
use crate::config::OrchestratorConfig;
use crate::discovery::LocalModel;
use crate::external::ExternalModel;
use crate::process::{ProcessConfig, ProcessManager};
use crate::resources::{DeviceId, ResourceConfig, ResourceTracker, SystemMemory};
use river_core::RiverError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Model status for API responses
#[derive(Debug, Clone)]
pub enum LocalModelStatus {
    Available,
    Loading,
    Loaded {
        endpoint: String,
        device: DeviceId,
        idle_seconds: u64,
    },
    Error(String),
}

/// Extended local model with runtime status
#[derive(Debug, Clone)]
pub struct LocalModelEntry {
    pub model: LocalModel,
    pub status: LocalModelStatus,
    pub releasable: bool,  // Can be evicted if resources needed
}

/// Shared orchestrator state
pub struct OrchestratorState {
    pub agents: RwLock<HashMap<String, AgentInfo>>,
    pub config: OrchestratorConfig,
    pub local_models: RwLock<HashMap<String, LocalModelEntry>>,
    pub external_models: Vec<ExternalModel>,
    pub resource_tracker: Arc<ResourceTracker>,
    pub process_manager: Arc<ProcessManager>,
    pub auth_token: String,
}

impl OrchestratorState {
    /// Create new orchestrator state
    pub fn new(
        config: OrchestratorConfig,
        local_models: Vec<LocalModel>,
        external_models: Vec<ExternalModel>,
        resource_config: ResourceConfig,
        process_config: ProcessConfig,
        auth_token: String,
    ) -> Self {
        let local_entries: HashMap<String, LocalModelEntry> = local_models
            .into_iter()
            .map(|m| {
                let id = m.id.clone();
                let entry = LocalModelEntry {
                    model: m,
                    status: LocalModelStatus::Available,
                    releasable: false,
                };
                (id, entry)
            })
            .collect();

        Self {
            agents: RwLock::new(HashMap::new()),
            config,
            local_models: RwLock::new(local_entries),
            external_models,
            resource_tracker: Arc::new(ResourceTracker::new(resource_config)),
            process_manager: Arc::new(ProcessManager::new(process_config)),
            auth_token,
        }
    }

    /// Get health threshold as Duration
    pub fn health_threshold(&self) -> Duration {
        Duration::from_secs(self.config.health_threshold_seconds)
    }

    /// Register or update agent heartbeat
    pub async fn heartbeat(&self, name: String, gateway_url: String) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&name) {
            agent.heartbeat();
            if agent.gateway_url != gateway_url {
                agent.update_url(gateway_url);
            }
        } else {
            agents.insert(name.clone(), AgentInfo::new(name, gateway_url));
        }
    }

    /// Get all agent statuses
    pub async fn agent_statuses(&self) -> Vec<AgentStatus> {
        let agents = self.agents.read().await;
        let threshold = self.health_threshold();
        agents
            .values()
            .map(|a| AgentStatus::from_agent(a, threshold))
            .collect()
    }

    /// Get count of registered agents
    pub async fn agent_count(&self) -> usize {
        self.agents.read().await.len()
    }

    /// Request a model to be loaded
    ///
    /// Blocks for up to `timeout_seconds` waiting for the model to become ready.
    /// Returns `Loading` if timeout expires (model continues loading in background).
    pub async fn request_model(&self, model_id: &str, timeout_seconds: u32) -> Result<ModelRequestResponse, RiverError> {
        let timeout = Duration::from_secs(timeout_seconds as u64);
        let start = Instant::now();
        let poll_interval = Duration::from_millis(500);

        // Check external models first (always ready immediately)
        for ext in &self.external_models {
            if ext.id == model_id {
                return Ok(ModelRequestResponse::Ready {
                    endpoint: ext.endpoint(),
                    device: None,
                    warning: None,
                });
            }
        }

        // Check local models
        {
            let local_models = self.local_models.read().await;
            if !local_models.contains_key(model_id) {
                return Err(RiverError::orchestrator(format!("Model not found: {}", model_id)));
            }
        }

        // Main loop: check status, spawn if needed, poll until ready or timeout
        loop {
            // Check current status
            {
                let local_models = self.local_models.read().await;
                if let Some(entry) = local_models.get(model_id) {
                    match &entry.status {
                        LocalModelStatus::Loaded { endpoint, device, .. } => {
                            return Ok(ModelRequestResponse::Ready {
                                endpoint: endpoint.clone(),
                                device: Some(*device),
                                warning: None,
                            });
                        }
                        LocalModelStatus::Loading => {
                            // Model is loading (either from us or another request)
                            // Check timeout and continue polling
                            if start.elapsed() >= timeout {
                                let remaining = 30; // Estimate 30 more seconds
                                return Ok(ModelRequestResponse::Loading {
                                    estimated_seconds: remaining,
                                });
                            }
                            // Drop lock and poll again after interval
                            drop(local_models);
                            tokio::time::sleep(poll_interval).await;
                            continue;
                        }
                        LocalModelStatus::Error(e) => {
                            return Err(RiverError::orchestrator(format!(
                                "Model failed to load: {}", e
                            )));
                        }
                        LocalModelStatus::Available => {
                            // Need to spawn - continue below
                        }
                    }
                }
            }

            // Model is Available - need to spawn it
            // Check if llama-server is available
            if !self.process_manager.is_available() {
                return Err(RiverError::orchestrator(
                    "Local model inference unavailable: llama-server not found"
                ));
            }

            // Get model info and find device
            let (model_clone, vram_needed) = {
                let local_models = self.local_models.read().await;
                let entry = local_models.get(model_id).ok_or_else(|| {
                    RiverError::orchestrator(format!("Model not found: {}", model_id))
                })?;
                (entry.model.clone(), entry.model.metadata.estimate_vram())
            };

            let device = match self.resource_tracker.find_device_for(vram_needed).await {
                Some(dev) => dev,
                None => {
                    // Try to evict releasable models to make space
                    self.evict_for_space(vram_needed).await?;
                    self.resource_tracker.find_device_for(vram_needed).await
                        .ok_or_else(|| {
                            RiverError::orchestrator(format!(
                                "Insufficient resources: model requires {} bytes, eviction failed",
                                vram_needed
                            ))
                        })?
                }
            };

            // Check for swap warning on CPU
            let warning = if matches!(device, DeviceId::Cpu) {
                let sys_mem = SystemMemory::current();
                let cpu_allocated = self.resource_tracker.cpu_allocated().await;
                if sys_mem.would_use_swap(vram_needed, cpu_allocated) {
                    let swap_gb = sys_mem.estimated_swap_usage(vram_needed, cpu_allocated) as f64
                        / 1_073_741_824.0;
                    Some(format!(
                        "Model will use ~{:.1}GB swap. Expect slow inference due to memory pressure.",
                        swap_gb
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            // Mark as loading before spawning
            {
                let mut local_models = self.local_models.write().await;
                if let Some(entry) = local_models.get_mut(model_id) {
                    entry.status = LocalModelStatus::Loading;
                }
            }

            // Spawn process (this blocks until healthy or its internal timeout)
            match self.process_manager.spawn(&model_clone, device).await {
                Ok(snapshot) => {
                    // Allocate resources
                    self.resource_tracker.allocate(model_id, device, vram_needed).await;

                    // Update status to loaded
                    let endpoint = format!("http://127.0.0.1:{}/v1/chat/completions", snapshot.port);
                    {
                        let mut local_models = self.local_models.write().await;
                        if let Some(entry) = local_models.get_mut(model_id) {
                            entry.status = LocalModelStatus::Loaded {
                                endpoint: endpoint.clone(),
                                device,
                                idle_seconds: 0,
                            };
                        }
                    }

                    return Ok(ModelRequestResponse::Ready {
                        endpoint,
                        device: Some(device),
                        warning,
                    });
                }
                Err(e) => {
                    // Check if it's a timeout error - return Loading instead
                    let error_msg = e.to_string();
                    if error_msg.contains("timeout") || error_msg.contains("Health check") {
                        // Model is still loading in background
                        return Ok(ModelRequestResponse::Loading {
                            estimated_seconds: 30, // Estimate 30 more seconds
                        });
                    }

                    // Mark as error
                    {
                        let mut local_models = self.local_models.write().await;
                        if let Some(entry) = local_models.get_mut(model_id) {
                            entry.status = LocalModelStatus::Error(error_msg.clone());
                        }
                    }

                    return Err(e);
                }
            }
        }
    }

    /// Mark a model as releasable for eviction
    pub async fn release_model(&self, model_id: &str) -> bool {
        let mut local_models = self.local_models.write().await;
        if let Some(entry) = local_models.get_mut(model_id) {
            entry.releasable = true;
            true
        } else {
            false
        }
    }

    /// Evict releasable models to free up space
    async fn evict_for_space(&self, bytes_needed: u64) -> Result<(), RiverError> {
        // Get releasable models sorted by idle time (oldest first)
        let candidates: Vec<(String, u64)> = {
            let local_models = self.local_models.read().await;
            let mut list: Vec<_> = local_models
                .iter()
                .filter(|(_, entry)| entry.releasable)
                .filter_map(|(id, entry)| {
                    if let LocalModelStatus::Loaded { .. } = &entry.status {
                        Some((id.clone(), entry.model.metadata.estimate_vram()))
                    } else {
                        None
                    }
                })
                .collect();
            // Sort by VRAM (largest first for efficient eviction)
            list.sort_by(|a, b| b.1.cmp(&a.1));
            list
        };

        let mut freed = 0u64;
        for (model_id, vram) in candidates {
            if freed >= bytes_needed {
                break;
            }
            tracing::info!("Evicting releasable model {} to free space", model_id);
            self.unload_model(&model_id).await?;
            freed += vram;
        }

        if freed >= bytes_needed {
            Ok(())
        } else {
            Err(RiverError::orchestrator(format!(
                "Could not free enough space: needed {} bytes, freed {} bytes",
                bytes_needed, freed
            )))
        }
    }

    /// Unload a model
    pub async fn unload_model(&self, model_id: &str) -> Result<(), RiverError> {
        // Get device before unloading
        let device = {
            let local_models = self.local_models.read().await;
            if let Some(entry) = local_models.get(model_id) {
                match &entry.status {
                    LocalModelStatus::Loaded { device, .. } => Some(*device),
                    _ => None,
                }
            } else {
                None
            }
        };

        // Kill process
        let _ = self.process_manager.kill(model_id).await;

        // Release resources
        if let Some(device) = device {
            self.resource_tracker.release(model_id, device).await;
        }

        // Update status
        let mut local_models = self.local_models.write().await;
        if let Some(entry) = local_models.get_mut(model_id) {
            entry.status = LocalModelStatus::Available;
            entry.releasable = false;
        }

        Ok(())
    }

    /// Check if llama-server is available
    pub fn llama_server_available(&self) -> bool {
        self.process_manager.is_available()
    }
}

/// Response from model request
#[derive(Debug)]
pub enum ModelRequestResponse {
    Ready {
        endpoint: String,
        device: Option<DeviceId>,
        warning: Option<String>,
    },
    Loading {
        estimated_seconds: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> OrchestratorState {
        OrchestratorState::new(
            OrchestratorConfig::default(),
            vec![],
            vec![],
            ResourceConfig::default(),
            ProcessConfig::default(),
            "test-token".to_string(),
        )
    }

    #[test]
    fn test_state_creation() {
        let state = test_state();
        assert_eq!(state.config.port, 5000);
    }

    #[tokio::test]
    async fn test_state_heartbeat_creates_agent() {
        let state = test_state();
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;
        assert_eq!(state.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_state_heartbeat_updates_existing() {
        let state = test_state();
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;
        state.heartbeat("test".to_string(), "http://localhost:4000".to_string()).await;

        let statuses = state.agent_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].gateway_url, "http://localhost:4000");
    }

    #[tokio::test]
    async fn test_state_agent_statuses() {
        let state = test_state();
        state.heartbeat("agent1".to_string(), "http://localhost:3000".to_string()).await;
        state.heartbeat("agent2".to_string(), "http://localhost:3001".to_string()).await;

        let statuses = state.agent_statuses().await;
        assert_eq!(statuses.len(), 2);
    }
}

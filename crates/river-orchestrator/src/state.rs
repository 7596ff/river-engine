//! Shared application state

use crate::agents::{AgentInfo, AgentStatus};
use crate::config::OrchestratorConfig;
use crate::models::ModelInfo;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;

/// Shared orchestrator state
pub struct OrchestratorState {
    pub agents: RwLock<HashMap<String, AgentInfo>>,
    pub models: Vec<ModelInfo>,
    pub config: OrchestratorConfig,
}

impl OrchestratorState {
    /// Create new orchestrator state
    pub fn new(config: OrchestratorConfig, models: Vec<ModelInfo>) -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            models,
            config,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let config = OrchestratorConfig::default();
        let state = OrchestratorState::new(config, vec![]);
        assert_eq!(state.config.port, 5000);
    }

    #[tokio::test]
    async fn test_state_heartbeat_creates_agent() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;
        assert_eq!(state.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_state_heartbeat_updates_existing() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;
        state.heartbeat("test".to_string(), "http://localhost:4000".to_string()).await;

        let statuses = state.agent_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].gateway_url, "http://localhost:4000");
    }

    #[tokio::test]
    async fn test_state_agent_statuses() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("agent1".to_string(), "http://localhost:3000".to_string()).await;
        state.heartbeat("agent2".to_string(), "http://localhost:3001".to_string()).await;

        let statuses = state.agent_statuses().await;
        assert_eq!(statuses.len(), 2);
    }
}

//! Agent registry and health tracking

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::{Duration, Instant};

/// Information about a registered agent
pub struct AgentInfo {
    pub name: String,
    pub gateway_url: String,
    pub last_heartbeat: Instant,
    pub registered_at: DateTime<Utc>,
}

impl AgentInfo {
    /// Create new agent info (registers current time as first heartbeat)
    pub fn new(name: String, gateway_url: String) -> Self {
        Self {
            name,
            gateway_url,
            last_heartbeat: Instant::now(),
            registered_at: Utc::now(),
        }
    }

    /// Update heartbeat timestamp
    pub fn heartbeat(&mut self) {
        self.last_heartbeat = Instant::now();
    }

    /// Update gateway URL
    pub fn update_url(&mut self, url: String) {
        self.gateway_url = url;
    }

    /// Check if agent is healthy (heartbeat within threshold)
    pub fn is_healthy(&self, threshold: Duration) -> bool {
        self.last_heartbeat.elapsed() < threshold
    }

    /// Get seconds since last heartbeat
    pub fn seconds_since_heartbeat(&self) -> u64 {
        self.last_heartbeat.elapsed().as_secs()
    }
}

/// Agent status for API response
#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub name: String,
    pub gateway_url: String,
    pub healthy: bool,
    pub last_heartbeat_seconds_ago: u64,
    pub registered_at: DateTime<Utc>,
}

impl AgentStatus {
    pub fn from_agent(agent: &AgentInfo, threshold: Duration) -> Self {
        Self {
            name: agent.name.clone(),
            gateway_url: agent.gateway_url.clone(),
            healthy: agent.is_healthy(threshold),
            last_heartbeat_seconds_ago: agent.seconds_since_heartbeat(),
            registered_at: agent.registered_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_is_healthy_within_threshold() {
        let agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        // Just created, should be healthy
        assert!(agent.is_healthy(Duration::from_secs(120)));
    }

    #[test]
    fn test_agent_heartbeat_updates_timestamp() {
        let mut agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        let before = agent.last_heartbeat;
        std::thread::sleep(Duration::from_millis(10));
        agent.heartbeat();
        assert!(agent.last_heartbeat > before);
    }

    #[test]
    fn test_agent_status_from_agent() {
        let agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        let status = AgentStatus::from_agent(&agent, Duration::from_secs(120));
        assert_eq!(status.name, "test");
        assert!(status.healthy);
        assert!(status.last_heartbeat_seconds_ago < 1);
    }

    #[test]
    fn test_agent_update_url() {
        let mut agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        agent.update_url("http://localhost:4000".to_string());
        assert_eq!(agent.gateway_url, "http://localhost:4000");
    }
}


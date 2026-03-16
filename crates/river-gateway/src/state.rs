//! Shared application state

use crate::db::Database;
use crate::tools::{ToolExecutor, ToolRegistry};
use river_core::{AgentBirth, SnowflakeGenerator};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

/// Shared application state
pub struct AppState {
    pub config: GatewayConfig,
    pub db: Arc<Mutex<Database>>,
    pub snowflake_gen: Arc<SnowflakeGenerator>,
    pub tool_executor: Arc<RwLock<ToolExecutor>>,
}

/// Gateway configuration (runtime)
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub model_url: String,
    pub model_name: String,
    pub context_limit: u64,
    pub heartbeat_minutes: u32,
    pub agent_birth: AgentBirth,
}

impl AppState {
    pub fn new(config: GatewayConfig, db: Database, registry: ToolRegistry) -> Self {
        let executor = ToolExecutor::new(registry, config.context_limit);

        Self {
            snowflake_gen: Arc::new(SnowflakeGenerator::new(config.agent_birth)),
            db: Arc::new(Mutex::new(db)),
            tool_executor: Arc::new(RwLock::new(executor)),
            config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::tools::ToolRegistry;

    #[test]
    fn test_state_creation() {
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth: AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap(),
        };

        let db = Database::open_in_memory().unwrap();
        let registry = ToolRegistry::new();
        let state = AppState::new(config, db, registry);

        // Verify state was created correctly
        assert_eq!(state.config.port, 3000);
        assert_eq!(state.config.context_limit, 65536);
    }
}

//! Shared application state

use crate::db::Database;
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::metrics::AgentMetrics;
use crate::r#loop::{LoopEvent, MessageQueue};
use crate::redis::{RedisClient, RedisConfig};
use crate::subagent::SubagentManager;
use crate::tools::{ToolExecutor, ToolRegistry};
use river_core::{AgentBirth, SnowflakeGenerator};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, RwLock};

/// Shared application state
pub struct AppState {
    pub config: GatewayConfig,
    pub db: Arc<Mutex<Database>>,
    pub snowflake_gen: Arc<SnowflakeGenerator>,
    pub tool_executor: Arc<RwLock<ToolExecutor>>,
    pub embedding_client: Option<Arc<EmbeddingClient>>,
    pub redis_client: Option<Arc<RedisClient>>,
    pub loop_tx: mpsc::Sender<LoopEvent>,
    pub message_queue: Arc<MessageQueue>,
    /// Bearer token for authentication (if configured)
    pub auth_token: Option<String>,
    /// Subagent manager
    pub subagent_manager: Arc<RwLock<SubagentManager>>,
    /// Shared metrics for observability
    pub metrics: Arc<RwLock<AgentMetrics>>,
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
    pub agent_name: String,
    pub embedding: Option<EmbeddingConfig>,
    pub redis: Option<RedisConfig>,
}

impl GatewayConfig {
    pub fn db_path(&self) -> std::path::PathBuf {
        self.data_dir.join("river.db")
    }
}

impl AppState {
    pub fn new(
        config: GatewayConfig,
        db: Arc<Mutex<Database>>,
        registry: ToolRegistry,
        embedding_client: Option<EmbeddingClient>,
        redis_client: Option<RedisClient>,
        loop_tx: mpsc::Sender<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        auth_token: Option<String>,
        subagent_manager: Arc<RwLock<SubagentManager>>,
        metrics: Arc<RwLock<AgentMetrics>>,
    ) -> Self {
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(config.agent_birth));
        let executor = ToolExecutor::new(registry);

        Self {
            snowflake_gen,
            db,
            tool_executor: Arc::new(RwLock::new(executor)),
            embedding_client: embedding_client.map(Arc::new),
            redis_client: redis_client.map(Arc::new),
            loop_tx,
            message_queue,
            config,
            auth_token,
            subagent_manager,
            metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::metrics::AgentMetrics;
    use crate::tools::ToolRegistry;
    use chrono::Utc;
    use river_core::SnowflakeGenerator;

    #[tokio::test]
    async fn test_state_creation() {
        let agent_birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth,
            agent_name: "test".to_string(),
            embedding: None,
            redis: None,
        };

        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let (loop_tx, _loop_rx) = mpsc::channel(256);
        let message_queue = Arc::new(MessageQueue::new());
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));
        let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)));
        let metrics = Arc::new(RwLock::new(AgentMetrics::new(
            "test".to_string(),
            Utc::now(),
            65536,
        )));
        let state = AppState::new(
            config,
            db,
            registry,
            None,
            None,
            loop_tx,
            message_queue,
            None,
            subagent_manager,
            metrics,
        );

        assert_eq!(state.config.port, 3000);
        assert_eq!(state.config.context_limit, 65536);
        assert!(state.embedding_client.is_none());
        assert!(state.redis_client.is_none());
    }
}

//! Shared application state

use crate::db::Database;
use crate::memory::{EmbeddingClient, EmbeddingConfig};
use crate::r#loop::{LoopEvent, MessageQueue};
use crate::redis::{RedisClient, RedisConfig};
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

impl AppState {
    pub fn new(
        config: GatewayConfig,
        db: Arc<Mutex<Database>>,
        registry: ToolRegistry,
        embedding_client: Option<EmbeddingClient>,
        redis_client: Option<RedisClient>,
        loop_tx: mpsc::Sender<LoopEvent>,
        message_queue: Arc<MessageQueue>,
    ) -> Self {
        let executor = ToolExecutor::new(registry, config.context_limit);

        Self {
            snowflake_gen: Arc::new(SnowflakeGenerator::new(config.agent_birth)),
            db,
            tool_executor: Arc::new(RwLock::new(executor)),
            embedding_client: embedding_client.map(Arc::new),
            redis_client: redis_client.map(Arc::new),
            loop_tx,
            message_queue,
            config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::tools::ToolRegistry;

    #[tokio::test]
    async fn test_state_creation() {
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth: AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap(),
            agent_name: "test".to_string(),
            embedding: None,
            redis: None,
        };

        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let (loop_tx, _loop_rx) = mpsc::channel(256);
        let message_queue = Arc::new(MessageQueue::new());
        let state = AppState::new(config, db, registry, None, None, loop_tx, message_queue);

        assert_eq!(state.config.port, 3000);
        assert_eq!(state.config.context_limit, 65536);
        assert!(state.embedding_client.is_none());
        assert!(state.redis_client.is_none());
    }
}

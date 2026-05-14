//! River Gateway - Agent Runtime

/// Birth record stored at {data_dir}/birth.json
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BirthRecord {
    pub id: river_core::Snowflake,
    pub name: String,
}

pub mod adapters;
pub use adapters::AdapterRegistry;
pub mod api;
pub mod channels;
pub mod config;
pub mod db;
pub mod git;
pub mod heartbeat;
pub mod memory;
pub mod model;
pub mod queue;
pub mod redis;
pub mod server;
pub mod state;
pub mod tools;
// inbox module removed — replaced by channels module
pub mod agent;
pub mod conversations;
pub mod coordinator;
pub mod embeddings;
pub mod flash;
pub mod logging;
pub mod metrics;
pub mod policy;
pub mod preferences;
pub mod spectator;
pub mod subagent;
pub mod watchdog;

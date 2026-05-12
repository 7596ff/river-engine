//! River Gateway - Agent Runtime

pub mod adapters;
pub use adapters::AdapterRegistry;
pub mod config;
pub mod db;
pub mod tools;
pub mod state;
pub mod server;
pub mod api;
pub mod model;
pub mod channels;
pub mod queue;
pub mod memory;
pub mod redis;
pub mod heartbeat;
pub mod git;
// inbox module removed — replaced by channels module
pub mod conversations;
pub mod subagent;
pub mod metrics;
pub mod logging;
pub mod policy;
pub mod preferences;
pub mod watchdog;
pub mod embeddings;
pub mod flash;
pub mod agent;
pub mod coordinator;
pub mod spectator;

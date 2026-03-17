//! River Engine Orchestrator
//!
//! Coordination service for River Engine agents.

pub mod agents;
pub mod discovery;
pub mod resources;
pub mod process;
pub mod api;
pub mod config;
pub mod models;
pub mod state;

pub use config::{ModelConfig, ModelsFile, OrchestratorConfig};
pub use state::OrchestratorState;

//! River Engine Orchestrator
//!
//! Coordination service for River Engine agents.

pub mod agents;
pub mod discovery;
pub mod resources;
pub mod process;
pub mod external;
pub mod api;
pub mod config;
pub mod config_file;
pub mod env;
pub mod state;

pub use config::OrchestratorConfig;
pub use state::OrchestratorState;

//! River Engine Orchestrator
//!
//! Coordination service for River Engine agents.

pub mod agents;
pub mod api;
pub mod cli_builder;
pub mod config;
pub mod config_file;
pub mod discovery;
pub mod env;
pub mod external;
pub mod process;
pub mod resources;
pub mod state;
pub mod supervisor;
pub mod validate;

pub use config::OrchestratorConfig;
pub use state::OrchestratorState;

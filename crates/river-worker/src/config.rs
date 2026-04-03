//! Worker configuration.

use river_adapter::{Baton, Side};
use river_protocol::WorkerRegistrationResponse;
use std::path::PathBuf;

/// Worker config from CLI args.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub orchestrator_endpoint: String,
    pub dyad: String,
    pub side: Side,
    pub port: u16,
}

// Re-export registration types for convenience
pub use river_protocol::ModelConfig;
pub type RegistrationResponse = WorkerRegistrationResponse;

impl WorkerConfig {
    pub fn workspace_path(&self, registration: &RegistrationResponse) -> PathBuf {
        PathBuf::from(&registration.workspace)
    }

    pub fn context_path(&self, registration: &RegistrationResponse) -> PathBuf {
        let workspace = self.workspace_path(registration);
        let side_str = match self.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        workspace.join(side_str).join("context.jsonl")
    }

    pub fn identity_path(&self, registration: &RegistrationResponse) -> PathBuf {
        let workspace = self.workspace_path(registration);
        let side_str = match self.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        workspace.join(side_str).join("identity.md")
    }

    pub fn role_path(&self, registration: &RegistrationResponse) -> PathBuf {
        let workspace = self.workspace_path(registration);
        let role_str = match registration.baton {
            Baton::Actor => "actor",
            Baton::Spectator => "spectator",
        };
        workspace.join("roles").join(format!("{}.md", role_str))
    }
}

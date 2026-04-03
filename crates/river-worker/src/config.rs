//! Worker configuration.

use river_adapter::{Baton, Ground, Side};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Worker config from CLI args.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub orchestrator_endpoint: String,
    pub dyad: String,
    pub side: Side,
    pub port: u16,
}

/// Registration response from orchestrator.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationResponse {
    pub accepted: bool,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub model: ModelConfig,
    pub ground: Ground,
    pub workspace: String,
    pub initial_message: Option<String>,
    pub start_sleeping: bool,
}

/// Model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,
    pub context_limit: usize,
}

/// Registration request to orchestrator.
#[derive(Debug, Serialize)]
pub struct RegistrationRequest {
    pub endpoint: String,
    pub worker: WorkerRegistration,
}

#[derive(Debug, Serialize)]
pub struct WorkerRegistration {
    pub dyad: String,
    pub side: Side,
}

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

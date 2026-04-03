//! Registration protocol types for workers and adapters.

use crate::{Baton, Ground, ModelConfig, Side};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// === Worker Registration ===

/// Worker identity for registration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkerRegistration {
    pub dyad: String,
    pub side: Side,
}

/// Worker registration request to orchestrator.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkerRegistrationRequest {
    pub endpoint: String,
    pub worker: WorkerRegistration,
}

/// Worker registration response from orchestrator.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WorkerRegistrationResponse {
    pub accepted: bool,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub model: ModelConfig,
    pub ground: Ground,
    pub workspace: String,
    pub initial_message: Option<String>,
    pub start_sleeping: bool,
}

// === Adapter Registration ===

/// Adapter identity for registration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdapterRegistration {
    #[serde(rename = "type")]
    pub adapter_type: String,
    pub dyad: String,
    pub features: Vec<u16>,
}

/// Adapter registration request to orchestrator.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdapterRegistrationRequest {
    pub endpoint: String,
    pub adapter: AdapterRegistration,
}

/// Adapter registration response from orchestrator.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AdapterRegistrationResponse {
    pub accepted: bool,
    /// Adapter-specific configuration (e.g., Discord token).
    pub config: serde_json::Value,
    pub worker_endpoint: String,
}

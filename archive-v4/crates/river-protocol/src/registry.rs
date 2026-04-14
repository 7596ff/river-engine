//! Registry types for process discovery.

use crate::{Baton, Ground, Side};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Process entry in the registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessEntry {
    Worker {
        endpoint: String,
        dyad: String,
        side: Side,
        baton: Baton,
        model: String,
        ground: Ground,
    },
    Adapter {
        endpoint: String,
        #[serde(rename = "adapter_type")]
        adapter_type: String,
        dyad: String,
        features: Vec<u16>,
    },
    EmbedService {
        endpoint: String,
        name: String,
    },
}

impl ProcessEntry {
    /// Get the endpoint for this process.
    pub fn endpoint(&self) -> &str {
        match self {
            ProcessEntry::Worker { endpoint, .. } => endpoint,
            ProcessEntry::Adapter { endpoint, .. } => endpoint,
            ProcessEntry::EmbedService { endpoint, .. } => endpoint,
        }
    }
}

/// The full registry sent to all processes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Registry {
    pub processes: Vec<ProcessEntry>,
}

impl Registry {
    /// Find embed service endpoint.
    pub fn embed_endpoint(&self) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::EmbedService { endpoint, .. } => Some(endpoint.as_str()),
            _ => None,
        })
    }

    /// Find adapter endpoint by type.
    pub fn adapter_endpoint(&self, adapter_type: &str) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::Adapter {
                endpoint,
                adapter_type: t,
                ..
            } if t == adapter_type => Some(endpoint.as_str()),
            _ => None,
        })
    }

    /// Find worker endpoint by dyad and side.
    pub fn worker_endpoint(&self, dyad: &str, side: &Side) -> Option<&str> {
        self.processes.iter().find_map(|p| match p {
            ProcessEntry::Worker {
                endpoint,
                dyad: d,
                side: s,
                ..
            } if d == dyad && s == side => Some(endpoint.as_str()),
            _ => None,
        })
    }
}

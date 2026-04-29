//! Adapter trait definition.

use async_trait::async_trait;

use crate::error::AdapterError;
use crate::feature::{FeatureId, OutboundRequest};
use crate::response::OutboundResponse;

/// Trait that adapter implementations must implement.
#[async_trait]
pub trait Adapter: Send + Sync {
    /// Adapter type name (e.g. "discord", "slack").
    fn adapter_type(&self) -> &str;

    /// Which features this adapter supports.
    fn features(&self) -> Vec<FeatureId>;

    /// Check if a specific feature is supported.
    fn supports(&self, feature: FeatureId) -> bool {
        self.features().contains(&feature)
    }

    /// Start receiving events, forward to bound Worker.
    async fn start(&self, worker_endpoint: String) -> Result<(), AdapterError>;

    /// Execute an outbound request.
    async fn execute(&self, request: OutboundRequest) -> Result<OutboundResponse, AdapterError>;

    /// Health check.
    async fn health(&self) -> Result<(), AdapterError>;
}

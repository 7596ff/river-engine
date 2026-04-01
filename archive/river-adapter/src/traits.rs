//! Adapter trait for gateway-side abstraction

use crate::{AdapterError, Feature, IncomingEvent, SendRequest, SendResponse};
use async_trait::async_trait;

#[async_trait]
pub trait Adapter: Send + Sync {
    /// Adapter name
    fn name(&self) -> &str;

    /// Check if feature is supported
    fn supports(&self, feature: &Feature) -> bool;

    /// Send a message
    async fn send(&self, request: SendRequest) -> Result<SendResponse, AdapterError>;

    /// Read channel history (if supported)
    async fn read_history(&self, channel: &str, limit: usize) -> Result<Vec<IncomingEvent>, AdapterError>;

    /// Health check
    async fn health(&self) -> Result<bool, AdapterError>;
}

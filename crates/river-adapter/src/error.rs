//! Adapter error types.

use crate::feature::FeatureId;

/// Errors that can occur in adapter operations.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("connection failed: {0}")]
    Connection(String),

    #[error("request timeout")]
    Timeout,

    #[error("feature not supported: {0:?}")]
    Unsupported(FeatureId),

    #[error("rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("platform error: {0}")]
    Platform(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

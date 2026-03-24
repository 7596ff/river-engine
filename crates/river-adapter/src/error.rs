//! Adapter error types

use crate::capabilities::Feature;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Adapter not found: {0}")]
    NotFound(String),

    #[error("Feature not supported: {0:?}")]
    FeatureNotSupported(Feature),

    #[error("Adapter error: {0}")]
    Other(String),
}

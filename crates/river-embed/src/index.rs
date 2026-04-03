//! Indexing logic.

use crate::embed::EmbedError;
use crate::store::StoreError;

#[derive(Debug)]
pub enum IndexError {
    Embed(EmbedError),
    Store(StoreError),
    EmptyContent,
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Embed(e) => write!(f, "embedding error: {}", e),
            Self::Store(e) => write!(f, "store error: {}", e),
            Self::EmptyContent => write!(f, "empty content"),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<EmbedError> for IndexError {
    fn from(e: EmbedError) -> Self {
        Self::Embed(e)
    }
}

impl From<StoreError> for IndexError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

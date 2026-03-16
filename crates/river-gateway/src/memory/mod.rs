//! Memory system for semantic search

mod embedding;
mod search;

pub use embedding::{EmbeddingClient, EmbeddingConfig};
pub use search::{MemorySearcher, SearchResult};

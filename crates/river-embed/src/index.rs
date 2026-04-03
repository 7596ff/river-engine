//! Indexing logic.

use sha2::{Digest, Sha256};

use river_snowflake::{AgentBirth, GeneratorCache, SnowflakeType};

use crate::chunk::{chunk_markdown, ChunkConfig};
use crate::embed::{EmbedClient, EmbedError};
use crate::store::{Store, StoreError};

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

/// Index result.
pub struct IndexResult {
    pub indexed: bool,
    pub chunks: usize,
}

/// Index content from a source file.
pub async fn index_content(
    store: &Store,
    embed_client: &EmbedClient,
    id_cache: &GeneratorCache,
    birth: AgentBirth,
    source_path: &str,
    content: &str,
) -> Result<IndexResult, IndexError> {
    if content.trim().is_empty() {
        return Err(IndexError::EmptyContent);
    }

    // Hash content
    let hash = hash_content(content);

    // Check if update needed
    if !store.needs_update(source_path, &hash)? {
        return Ok(IndexResult {
            indexed: false,
            chunks: 0,
        });
    }

    // Delete existing chunks
    store.delete_source(source_path)?;

    // Chunk content
    let config = ChunkConfig::default();
    let text_chunks = chunk_markdown(content, &config);

    if text_chunks.is_empty() {
        return Ok(IndexResult {
            indexed: true,
            chunks: 0,
        });
    }

    // Generate embeddings
    let texts: Vec<String> = text_chunks.iter().map(|c| c.text.clone()).collect();
    let embeddings = embed_client.embed(&texts).await?;

    // Store source
    store.upsert_source(source_path, &hash)?;

    // Store chunks with embeddings
    for (chunk, embedding) in text_chunks.iter().zip(embeddings.iter()) {
        let id = id_cache.next_id(birth, SnowflakeType::Embedding);
        store.insert_chunk(
            &id.to_string(),
            source_path,
            chunk.line_start,
            chunk.line_end,
            &chunk.text,
            embedding,
        )?;
    }

    Ok(IndexResult {
        indexed: true,
        chunks: text_chunks.len(),
    })
}

fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

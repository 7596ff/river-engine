//! Indexing logic.

use sha2::{Digest, Sha256};

use river_snowflake::SnowflakeType;

use crate::chunk::{chunk_markdown, ChunkConfig};
use crate::embed::EmbedError;
use crate::http::AppState;
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

/// Index content into the vector store.
pub async fn index_content(
    state: &AppState,
    source: &str,
    content: &str,
    birth: river_snowflake::AgentBirth,
) -> Result<(bool, usize), IndexError> {
    if content.trim().is_empty() {
        return Err(IndexError::EmptyContent);
    }

    // Hash content
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    // Check if update needed
    let needs_update = {
        let store = state.store.lock().await;
        store.needs_update(source, &hash)?
    };

    if !needs_update {
        return Ok((false, 0));
    }

    // Delete existing chunks
    {
        let store = state.store.lock().await;
        store.delete_source(source)?;
    }

    // Chunk content
    let config = ChunkConfig::default();
    let text_chunks = chunk_markdown(content, &config);

    if text_chunks.is_empty() {
        return Ok((true, 0));
    }

    // Generate embeddings (async)
    let texts: Vec<String> = text_chunks.iter().map(|c| c.text.clone()).collect();
    let embeddings = state.embed_client.embed(&texts).await?;

    // Store source and chunks
    {
        let store = state.store.lock().await;
        store.upsert_source(source, &hash)?;

        for (chunk, embedding) in text_chunks.iter().zip(embeddings.iter()) {
            let id = state.id_cache.next_id(birth, SnowflakeType::Embedding).unwrap();
            store.insert_chunk(
                &id.to_string(),
                source,
                chunk.line_start,
                chunk.line_end,
                &chunk.text,
                embedding,
            )?;
        }
    }

    Ok((true, text_chunks.len()))
}

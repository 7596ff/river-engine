//! Curation: flash selection via semantic search
//!
//! The curator searches for relevant memories based on the current
//! conversation context and surfaces them as flashes for the agent.

use crate::coordinator::{EventBus, CoordinatorEvent, SpectatorEvent};
use crate::embeddings::VectorStore;
use crate::flash::FlashQueue;
use chrono::Utc;
use std::sync::Arc;

/// Curator selects relevant memories and pushes them as flashes
pub struct Curator {
    flash_queue: Arc<FlashQueue>,
    /// Minimum similarity threshold for flash selection
    similarity_threshold: f32,
    /// Maximum flashes to push per turn
    max_flashes_per_turn: usize,
}

impl Curator {
    pub fn new(flash_queue: Arc<FlashQueue>) -> Self {
        Self {
            flash_queue,
            similarity_threshold: 0.6,
            max_flashes_per_turn: 3,
        }
    }

    /// Create curator with custom thresholds
    pub fn with_config(
        flash_queue: Arc<FlashQueue>,
        similarity_threshold: f32,
        max_flashes_per_turn: usize,
    ) -> Self {
        Self {
            flash_queue,
            similarity_threshold,
            max_flashes_per_turn,
        }
    }

    /// Get current flash queue length
    pub async fn queue_length(&self) -> usize {
        self.flash_queue.len().await
    }

    /// Search for relevant memories and push as flashes
    ///
    /// This requires an embedding of the transcript to search against
    /// the vector store. Returns the number of flashes pushed.
    pub async fn curate_with_embedding(
        &self,
        embedding: &[f32],
        vector_store: &VectorStore,
        bus: &EventBus,
    ) -> Result<usize, String> {
        let results = vector_store.search(embedding, self.max_flashes_per_turn * 2)
            .map_err(|e| format!("Vector search failed: {}", e))?;

        let mut pushed = 0;
        for result in results {
            if result.similarity < self.similarity_threshold {
                break;
            }
            if pushed >= self.max_flashes_per_turn {
                break;
            }

            // Publish flash event
            bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::Flash {
                content: result.content.clone(),
                source: result.source_path.clone(),
                ttl_turns: 5,
                timestamp: Utc::now(),
            }));

            tracing::debug!(
                source = %result.source_path,
                similarity = result.similarity,
                "Flash published"
            );

            pushed += 1;
        }

        if pushed > 0 {
            tracing::info!(
                flashes = pushed,
                "Curator surfaced memories"
            );
        }

        Ok(pushed)
    }

    /// Curate without embedding - placeholder for future implementation
    ///
    /// In a full implementation, this would:
    /// 1. Embed the transcript summary using an embedding client
    /// 2. Search the vector store
    /// 3. Push relevant results as flashes
    pub async fn curate(
        &self,
        _transcript_summary: &str,
        _vector_store: &VectorStore,
        _bus: &EventBus,
    ) -> Result<(), String> {
        // TODO: Implement when embedding client integration is complete
        // For now, this is a no-op
        //
        // Full implementation:
        // let embedding = embedding_client.embed(transcript_summary).await?;
        // self.curate_with_embedding(&embedding, vector_store, bus).await?;

        Ok(())
    }

    /// Direct flash push (for manual curation)
    pub async fn push_flash(
        &self,
        content: String,
        source: String,
        ttl_turns: u8,
        bus: &EventBus,
    ) {
        bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::Flash {
            content,
            source,
            ttl_turns,
            timestamp: Utc::now(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::Coordinator;

    #[tokio::test]
    async fn test_curator_creation() {
        let flash_queue = Arc::new(FlashQueue::new(10));
        let curator = Curator::new(flash_queue);

        assert_eq!(curator.similarity_threshold, 0.6);
        assert_eq!(curator.max_flashes_per_turn, 3);
    }

    #[tokio::test]
    async fn test_curator_with_config() {
        let flash_queue = Arc::new(FlashQueue::new(10));
        let curator = Curator::with_config(flash_queue, 0.8, 5);

        assert_eq!(curator.similarity_threshold, 0.8);
        assert_eq!(curator.max_flashes_per_turn, 5);
    }

    #[tokio::test]
    async fn test_push_flash() {
        let coord = Coordinator::new();
        let bus = coord.bus().clone();
        let mut rx = bus.subscribe();

        let flash_queue = Arc::new(FlashQueue::new(10));
        let curator = Curator::new(flash_queue);

        curator.push_flash(
            "Remember this".to_string(),
            "notes/test.md".to_string(),
            5,
            &bus,
        ).await;

        let event = rx.try_recv();
        assert!(matches!(
            event,
            Ok(CoordinatorEvent::Spectator(SpectatorEvent::Flash { ttl_turns: 5, .. }))
        ));
    }

    #[tokio::test]
    async fn test_queue_length() {
        let flash_queue = Arc::new(FlashQueue::new(10));
        let curator = Curator::new(flash_queue.clone());

        assert_eq!(curator.queue_length().await, 0);

        // Push a flash directly to the queue
        use crate::flash::{Flash, FlashTTL};
        flash_queue.push(Flash {
            id: "test".to_string(),
            content: "test".to_string(),
            source: "test".to_string(),
            ttl: FlashTTL::Turns(3),
            created: Utc::now(),
        }).await;

        assert_eq!(curator.queue_length().await, 1);
    }
}

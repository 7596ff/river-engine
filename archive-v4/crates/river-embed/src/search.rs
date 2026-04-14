//! Search logic and cursor management.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use rand::Rng;
use serde::{Deserialize, Serialize};

/// Search result returned to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub content: String,
    pub source: String,
    pub score: f32,
}

/// Response from search endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub cursor: String,
    pub result: Option<SearchResult>,
    pub remaining: usize,
}

/// Internal cursor state.
pub struct Cursor {
    pub query_embedding: Vec<f32>,
    pub offset: usize,
    pub total_results: usize,
    pub expires_at: Instant,
}

/// Cursor manager with expiration.
#[derive(Clone)]
pub struct CursorManager {
    cursors: Arc<RwLock<HashMap<String, Cursor>>>,
    ttl: Duration,
}

impl CursorManager {
    /// Create a new cursor manager.
    pub fn new(ttl: Duration) -> Self {
        Self {
            cursors: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    /// Create a new cursor.
    pub fn create(&self, query_embedding: Vec<f32>, total_results: usize) -> String {
        let id = generate_cursor_id();
        let cursor = Cursor {
            query_embedding,
            offset: 1, // Start at 1 since /search already returned offset 0
            total_results,
            expires_at: Instant::now() + self.ttl,
        };

        let mut cursors = self.cursors.write().unwrap();
        cursors.insert(id.clone(), cursor);
        id
    }

    /// Remove all expired cursors.
    pub fn cleanup_expired(&self) {
        let mut cursors = self.cursors.write().unwrap();
        let now = Instant::now();
        cursors.retain(|_, cursor| cursor.expires_at > now);
    }

    /// Get a cursor and advance its offset.
    pub fn advance(&self, id: &str) -> Option<(Vec<f32>, usize, usize)> {
        let mut cursors = self.cursors.write().unwrap();
        let cursor = cursors.get_mut(id)?;

        if Instant::now() > cursor.expires_at {
            cursors.remove(id);
            return None;
        }

        // Refresh expiration
        cursor.expires_at = Instant::now() + self.ttl;

        let embedding = cursor.query_embedding.clone();
        let offset = cursor.offset;
        let remaining = cursor.total_results.saturating_sub(offset + 1);

        cursor.offset += 1;

        Some((embedding, offset, remaining))
    }
}

impl Default for CursorManager {
    fn default() -> Self {
        Self::new(Duration::from_secs(300)) // 5 minutes
    }
}

fn generate_cursor_id() -> String {
    let mut rng = rand::rng();
    let hex: String = (0..8)
        .map(|_| format!("{:x}", rng.random::<u8>()))
        .collect();
    format!("emb_{}", hex)
}

/// Create SearchResult from a search hit.
pub fn hit_to_result(
    id: String,
    source_path: String,
    line_start: usize,
    line_end: usize,
    text: String,
    distance: f32,
) -> SearchResult {
    SearchResult {
        id,
        content: text,
        source: format!("{}:{}-{}", source_path, line_start, line_end),
        score: 1.0 - distance, // Convert distance to similarity score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_starts_at_offset_1() {
        let manager = CursorManager::new(Duration::from_secs(60));
        let embedding = vec![1.0, 2.0, 3.0];

        let cursor_id = manager.create(embedding.clone(), 10);

        // First advance should return offset 1, not 0
        let (_, offset, remaining) = manager.advance(&cursor_id).unwrap();
        assert_eq!(offset, 1, "First /next call should return offset 1");
        assert_eq!(remaining, 8, "After returning offset 1, 8 results remain (2-9)");

        // Second advance should return offset 2
        let (_, offset, _) = manager.advance(&cursor_id).unwrap();
        assert_eq!(offset, 2);
    }

    #[test]
    fn test_cleanup_expired() {
        let manager = CursorManager::new(Duration::from_millis(1));
        let embedding = vec![1.0, 2.0, 3.0];

        let cursor_id = manager.create(embedding.clone(), 10);

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(10));

        // Cleanup
        manager.cleanup_expired();

        // Cursor should be gone
        assert!(manager.advance(&cursor_id).is_none());
    }
}

//! Vector similarity search for semantic memory

use crate::db::{Database, Memory};
use river_core::{RiverResult, Snowflake};

/// Search result with similarity score
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub memory: Memory,
    pub similarity: f32,
}

/// Memory searcher using cosine similarity
pub struct MemorySearcher;

impl MemorySearcher {
    /// Compute cosine similarity between two vectors
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot / (norm_a * norm_b)
    }

    /// Search memories by similarity to query embedding
    pub fn search(
        db: &Database,
        query_embedding: &[f32],
        limit: usize,
        source_filter: Option<&str>,
        after: Option<i64>,
        before: Option<i64>,
    ) -> RiverResult<Vec<SearchResult>> {
        // Get all embeddings
        let all_embeddings = db.get_all_memory_embeddings()?;

        // Compute similarities
        let mut scored: Vec<(Snowflake, f32)> = all_embeddings
            .iter()
            .map(|(id, emb)| (*id, Self::cosine_similarity(query_embedding, emb)))
            .collect();

        // Sort by similarity descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Take top N
        let top_ids: Vec<Snowflake> = scored.iter().take(limit * 2).map(|(id, _)| *id).collect();

        // Fetch full memories
        let memories = db.get_memories_by_ids(&top_ids)?;

        // Build results with filtering
        let mut results: Vec<SearchResult> = Vec::new();
        for (id, similarity) in scored.iter().take(limit * 2) {
            if let Some(memory) = memories.iter().find(|m| m.id == *id) {
                // Apply filters
                if let Some(src) = source_filter {
                    if memory.source != src {
                        continue;
                    }
                }
                if let Some(after_ts) = after {
                    if memory.timestamp < after_ts {
                        continue;
                    }
                }
                if let Some(before_ts) = before {
                    if memory.timestamp > before_ts {
                        continue;
                    }
                }

                results.push(SearchResult {
                    memory: memory.clone(),
                    similarity: *similarity,
                });

                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = MemorySearcher::cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = MemorySearcher::cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = MemorySearcher::cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 0.0001);
    }

    #[test]
    fn test_search() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        // Insert test memories with different embeddings
        let embeddings = vec![
            vec![1.0, 0.0, 0.0], // "north"
            vec![0.0, 1.0, 0.0], // "east"
            vec![0.7, 0.7, 0.0], // "northeast"
        ];

        for (i, emb) in embeddings.iter().enumerate() {
            let mem = Memory {
                id: gen.next_id(SnowflakeType::Embedding),
                content: format!("Memory {}", i),
                embedding: emb.clone(),
                source: "test".to_string(),
                timestamp: 1000 + i as i64,
                expires_at: None,
                metadata: None,
            };
            db.insert_memory(&mem).unwrap();
        }

        // Search for "north" direction
        let query = vec![1.0, 0.0, 0.0];
        let results = MemorySearcher::search(&db, &query, 2, None, None, None).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].memory.content, "Memory 0"); // Exact match
        assert!((results[0].similarity - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_search_with_source_filter() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        // Insert memories with different sources
        for i in 0..3 {
            let source = if i % 2 == 0 { "message" } else { "file" };
            let mem = Memory {
                id: gen.next_id(SnowflakeType::Embedding),
                content: format!("Memory {}", i),
                embedding: vec![1.0, 0.0, 0.0],
                source: source.to_string(),
                timestamp: 1000 + i as i64,
                expires_at: None,
                metadata: None,
            };
            db.insert_memory(&mem).unwrap();
        }

        let query = vec![1.0, 0.0, 0.0];
        let results = MemorySearcher::search(&db, &query, 10, Some("message"), None, None).unwrap();

        assert_eq!(results.len(), 2);
        for r in &results {
            assert_eq!(r.memory.source, "message");
        }
    }
}

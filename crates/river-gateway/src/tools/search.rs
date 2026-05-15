//! Semantic search tool — searches embeddings via VectorStore

use river_core::RiverError;
use serde_json::{json, Value};
use std::sync::Arc;

use super::registry::{Tool, ToolResult};
use crate::embeddings::VectorStore;
use crate::memory::EmbeddingClient;

pub struct SearchTool {
    store: VectorStore,
    embedding_client: Arc<EmbeddingClient>,
}

impl SearchTool {
    pub fn new(store: VectorStore, embedding_client: Arc<EmbeddingClient>) -> Self {
        Self {
            store,
            embedding_client,
        }
    }
}

impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Semantic search over embedded files. Finds content similar in meaning to the query, unlike grep which matches exact text."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default: 5)" }
            },
            "required": ["query"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: query"))?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        let embedding = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.embedding_client.embed(query))
        })?;

        let results = self
            .store
            .search(&embedding, limit)
            .map_err(|e| RiverError::tool(format!("Search failed: {}", e)))?;

        if results.is_empty() {
            return Ok(ToolResult::success("No results found."));
        }

        let mut output = format!("Found {} results for \"{}\":\n\n", results.len(), query);
        for (i, result) in results.iter().enumerate() {
            let snippet: String = result.content.chars().take(200).collect();
            output.push_str(&format!(
                "{}. [{:.2}] {}\n   {}\n\n",
                i + 1,
                result.similarity,
                result.source_path,
                snippet,
            ));
        }

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_tool_schema() {
        let store = VectorStore::open_in_memory().unwrap();
        let client = Arc::new(EmbeddingClient::new(
            crate::memory::EmbeddingConfig::default(),
        ));
        let tool = SearchTool::new(store, client);
        assert_eq!(tool.name(), "search");
        let params = tool.parameters();
        assert!(params["properties"]["query"].is_object());
        assert!(params["properties"]["limit"].is_object());
    }
}

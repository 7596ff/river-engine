//! Memory tools: embed, memory_search, memory_delete, memory_delete_by_source

use river_core::{RiverError, Snowflake, SnowflakeGenerator, SnowflakeType};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use crate::db::{Database, Memory};
use crate::memory::{EmbeddingClient, MemorySearcher};
use river_tools::{Tool, ToolResult};

/// Embed tool - create embedding and store in memory
pub struct EmbedTool {
    db: Arc<Mutex<Database>>,
    embedding_client: Arc<EmbeddingClient>,
    snowflake_gen: Arc<SnowflakeGenerator>,
}

impl EmbedTool {
    pub fn new(
        db: Arc<Mutex<Database>>,
        embedding_client: Arc<EmbeddingClient>,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        Self {
            db,
            embedding_client,
            snowflake_gen,
        }
    }
}

impl Tool for EmbedTool {
    fn name(&self) -> &str {
        "embed"
    }

    fn description(&self) -> &str {
        "Create embedding and store in memory index"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Text to embed" },
                "source": { "type": "string", "description": "Source identifier (e.g., 'agent', 'file')" },
                "metadata": { "type": "object", "description": "Additional metadata (optional)" }
            },
            "required": ["content", "source"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: content".to_string()))?;

        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: source".to_string()))?;

        let metadata = args.get("metadata").map(|v| v.to_string());

        // Get embedding synchronously by blocking on the async call
        let embedding = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.embedding_client.embed(content))
        })?;

        let id = self.snowflake_gen.next_id(SnowflakeType::Embedding);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let memory = Memory {
            id,
            content: content.to_string(),
            embedding,
            source: source.to_string(),
            timestamp,
            expires_at: None,  // Agent-created embeddings are permanent
            metadata,
        };

        let db = self.db.lock().map_err(|_| RiverError::tool("Database lock poisoned".to_string()))?;
        db.insert_memory(&memory)?;

        Ok(ToolResult {
            output: format!("Created embedding with ID: {}", id),
            output_file: None,
        })
    }
}

/// Memory search tool
pub struct MemorySearchTool {
    db: Arc<Mutex<Database>>,
    embedding_client: Arc<EmbeddingClient>,
}

impl MemorySearchTool {
    pub fn new(db: Arc<Mutex<Database>>, embedding_client: Arc<EmbeddingClient>) -> Self {
        Self {
            db,
            embedding_client,
        }
    }
}

impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Semantic search over embeddings"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Maximum results", "default": 10 },
                "source": { "type": "string", "description": "Filter by source (optional)" },
                "after": { "type": "string", "description": "Filter by date (ISO 8601, optional)" },
                "before": { "type": "string", "description": "Filter by date (ISO 8601, optional)" }
            },
            "required": ["query"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: query".to_string()))?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let source = args.get("source").and_then(|v| v.as_str());

        // Parse date filters
        let after = args
            .get("after")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp());

        let before = args
            .get("before")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp());

        // Get query embedding
        let query_embedding = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.embedding_client.embed(query))
        })?;

        // Search
        let db = self.db.lock().map_err(|_| RiverError::tool("Database lock poisoned".to_string()))?;
        let results = MemorySearcher::search(&db, &query_embedding, limit, source, after, before)?;

        // Format results
        let mut output = String::new();
        for (i, result) in results.iter().enumerate() {
            output.push_str(&format!(
                "{}. [score: {:.3}] {}\n   Source: {}, Time: {}\n   ID: {}\n\n",
                i + 1,
                result.similarity,
                result.memory.content,
                result.memory.source,
                result.memory.timestamp,
                result.memory.id
            ));
        }

        if output.is_empty() {
            output = "No matches found".to_string();
        }

        Ok(ToolResult {
            output,
            output_file: None,
        })
    }
}

/// Memory delete tool
pub struct MemoryDeleteTool {
    db: Arc<Mutex<Database>>,
}

impl MemoryDeleteTool {
    pub fn new(db: Arc<Mutex<Database>>) -> Self {
        Self { db }
    }
}

impl Tool for MemoryDeleteTool {
    fn name(&self) -> &str {
        "memory_delete"
    }

    fn description(&self) -> &str {
        "Delete embedding by ID"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Snowflake ID of embedding to delete" }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let id_str = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: id".to_string()))?;

        let id = parse_snowflake_id(id_str)?;

        let db = self.db.lock().map_err(|_| RiverError::tool("Database lock poisoned".to_string()))?;
        let deleted = db.delete_memory(id)?;

        if deleted {
            Ok(ToolResult {
                output: format!("Deleted memory with ID: {}", id),
                output_file: None,
            })
        } else {
            Ok(ToolResult {
                output: format!("Memory not found: {}", id),
                output_file: None,
            })
        }
    }
}

/// Memory delete by source tool (bulk deletion)
pub struct MemoryDeleteBySourceTool {
    db: Arc<Mutex<Database>>,
}

impl MemoryDeleteBySourceTool {
    pub fn new(db: Arc<Mutex<Database>>) -> Self {
        Self { db }
    }
}

impl Tool for MemoryDeleteBySourceTool {
    fn name(&self) -> &str {
        "memory_delete_by_source"
    }

    fn description(&self) -> &str {
        "Delete embeddings by source, optionally before a timestamp"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source": { "type": "string", "description": "Source identifier to delete (e.g., 'message', 'file')" },
                "before": { "type": "string", "description": "Delete only entries before this date (ISO 8601, optional)" }
            },
            "required": ["source"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: source".to_string()))?;

        let before = args
            .get("before")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp());

        let db = self.db.lock().map_err(|_| RiverError::tool("Database lock poisoned".to_string()))?;
        let deleted = db.delete_memories_by_source(source, before)?;

        Ok(ToolResult {
            output: format!("Deleted {} memories with source '{}'", deleted, source),
            output_file: None,
        })
    }
}

/// Parse a Snowflake ID from its string representation (hex format: high-low)
fn parse_snowflake_id(s: &str) -> Result<Snowflake, RiverError> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return Err(RiverError::tool(format!("Invalid snowflake ID format: {}", s)));
    }

    let high = u64::from_str_radix(parts[0], 16)
        .map_err(|_| RiverError::tool(format!("Invalid snowflake ID: {}", s)))?;
    let low = u64::from_str_radix(parts[1], 16)
        .map_err(|_| RiverError::tool(format!("Invalid snowflake ID: {}", s)))?;

    Ok(Snowflake::from_parts(high, low))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration tests would require a running embedding server
    // Unit tests focus on parameter validation

    #[test]
    fn test_embed_tool_schema() {
        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let client = Arc::new(EmbeddingClient::new(crate::memory::EmbeddingConfig::default()));
        let birth = river_core::AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = Arc::new(SnowflakeGenerator::new(birth));

        let tool = EmbedTool::new(db, client, gen);
        let params = tool.parameters();

        assert!(params.get("properties").unwrap().get("content").is_some());
        assert!(params.get("properties").unwrap().get("source").is_some());
    }
}

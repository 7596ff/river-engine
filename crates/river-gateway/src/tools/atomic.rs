//! write_atomic tool — create single-claim knowledge notes with typed links

use chrono::Utc;
use river_core::{RiverError, SnowflakeGenerator, SnowflakeType};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use super::registry::{Tool, ToolResult};

pub struct WriteAtomicTool {
    workspace: PathBuf,
    snowflake_gen: Arc<SnowflakeGenerator>,
    agent_name: String,
}

impl WriteAtomicTool {
    pub fn new(
        workspace: PathBuf,
        snowflake_gen: Arc<SnowflakeGenerator>,
        agent_name: String,
    ) -> Self {
        Self {
            workspace,
            snowflake_gen,
            agent_name,
        }
    }
}

impl Tool for WriteAtomicTool {
    fn name(&self) -> &str {
        "write_atomic"
    }

    fn description(&self) -> &str {
        "Create a single-claim knowledge note with typed links. Content must be ≤100 words. At least one link and one tag required."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Single claim, observation, or connection (max 100 words)"
                },
                "links": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "description": "Link type (extends, complicates, contradicts, supports, resonates-with, etc.)" },
                            "target": { "type": "string", "description": "Target note ID (snowflake hex)" }
                        },
                        "required": ["type", "target"]
                    },
                    "description": "Typed links to other atomic notes (at least one required)"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags for categorization (at least one required)"
                }
            },
            "required": ["content", "links", "tags"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: content"))?;

        let links = args
            .get("links")
            .and_then(|v| v.as_array())
            .ok_or_else(|| RiverError::tool("Missing required parameter: links"))?;

        let tags = args
            .get("tags")
            .and_then(|v| v.as_array())
            .ok_or_else(|| RiverError::tool("Missing required parameter: tags"))?;

        // Validate content length
        let word_count = content.split_whitespace().count();
        if word_count > 100 {
            return Err(RiverError::tool(format!(
                "Content exceeds 100 word limit ({} words). Atomic notes must state a single claim.",
                word_count
            )));
        }

        // Validate links
        if links.is_empty() {
            return Err(RiverError::tool(
                "At least one link is required. Every atomic note must connect to something.",
            ));
        }
        for (i, link) in links.iter().enumerate() {
            if link.get("type").and_then(|v| v.as_str()).is_none() {
                return Err(RiverError::tool(format!(
                    "Link {} missing 'type' field",
                    i
                )));
            }
            if link.get("target").and_then(|v| v.as_str()).is_none() {
                return Err(RiverError::tool(format!(
                    "Link {} missing 'target' field",
                    i
                )));
            }
        }

        // Validate tags
        if tags.is_empty() {
            return Err(RiverError::tool("At least one tag is required."));
        }

        // Generate ID
        let id = self.snowflake_gen.next_id(SnowflakeType::AtomicNote);
        let id_hex = id.to_string();

        // Build links YAML
        let links_yaml: Vec<String> = links
            .iter()
            .map(|l| {
                format!(
                    "  - type: {}\n    target: \"{}\"",
                    l["type"].as_str().unwrap(),
                    l["target"].as_str().unwrap()
                )
            })
            .collect();

        // Build tags YAML
        let tags_str: Vec<&str> = tags.iter().filter_map(|t| t.as_str()).collect();

        // Build frontmatter
        let now = Utc::now().to_rfc3339();
        let frontmatter = format!(
            "---\nid: \"{}\"\ntype: atomic\ncreated: {}\nauthor: {}\nlinks:\n{}\ntags: [{}]\n---",
            id_hex,
            now,
            self.agent_name,
            links_yaml.join("\n"),
            tags_str.join(", "),
        );

        // Build full file
        let file_content = format!("{}\n\n{}\n", frontmatter, content);

        // Write file
        let atomic_dir = self.workspace.join("embeddings").join("atomic");
        std::fs::create_dir_all(&atomic_dir).map_err(|e| {
            RiverError::tool(format!("Failed to create embeddings/atomic/: {}", e))
        })?;

        let filename = format!("{}-z.md", id_hex);
        let file_path = atomic_dir.join(&filename);
        std::fs::write(&file_path, &file_content).map_err(|e| {
            RiverError::tool(format!("Failed to write atomic note: {}", e))
        })?;

        // Build link summary for output
        let link_summary: Vec<String> = links
            .iter()
            .map(|l| {
                format!(
                    "{} → {}",
                    l["type"].as_str().unwrap(),
                    l["target"].as_str().unwrap()
                )
            })
            .collect();

        let output = format!(
            "Created atomic note: {}\nPath: embeddings/atomic/{}\nLinks: {}\nTags: {}",
            id_hex,
            filename,
            link_summary.join(", "),
            tags_str.join(", "),
        );

        Ok(ToolResult::with_file(
            output,
            file_path.to_string_lossy().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator};
    use tempfile::TempDir;

    fn test_tool() -> (WriteAtomicTool, TempDir) {
        let temp = TempDir::new().unwrap();
        let birth = AgentBirth::new(2026, 5, 15, 12, 0, 0).unwrap();
        let gen = Arc::new(SnowflakeGenerator::new(birth));
        let tool = WriteAtomicTool::new(temp.path().to_path_buf(), gen, "test-agent".to_string());
        (tool, temp)
    }

    #[test]
    fn test_schema() {
        let (tool, _temp) = test_tool();
        assert_eq!(tool.name(), "write_atomic");
        let params = tool.parameters();
        assert!(params["properties"]["content"].is_object());
        assert!(params["properties"]["links"].is_object());
        assert!(params["properties"]["tags"].is_object());
    }

    #[test]
    fn test_write_success() {
        let (tool, _temp) = test_tool();
        let result = tool
            .execute(json!({
                "content": "Hobbes requires agreed-upon names for reason to work.",
                "links": [{"type": "extends", "target": "abc123"}],
                "tags": ["hobbes", "reason"]
            }))
            .unwrap();

        assert!(result.output.contains("Created atomic note"));
        assert!(result.output.contains("extends → abc123"));
        assert!(result.output_file.is_some());

        // Verify file exists and parses
        let path = result.output_file.unwrap();
        assert!(std::path::Path::new(&path).exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("type: atomic"));
        assert!(content.contains("Hobbes requires"));
        assert!(content.contains("extends"));
    }

    #[test]
    fn test_reject_over_100_words() {
        let (tool, _temp) = test_tool();
        let long_content = "word ".repeat(101);
        let result = tool.execute(json!({
            "content": long_content.trim(),
            "links": [{"type": "extends", "target": "abc"}],
            "tags": ["test"]
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("100 word limit"));
    }

    #[test]
    fn test_reject_empty_links() {
        let (tool, _temp) = test_tool();
        let result = tool.execute(json!({
            "content": "A claim.",
            "links": [],
            "tags": ["test"]
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("link is required"));
    }

    #[test]
    fn test_reject_empty_tags() {
        let (tool, _temp) = test_tool();
        let result = tool.execute(json!({
            "content": "A claim.",
            "links": [{"type": "extends", "target": "abc"}],
            "tags": []
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tag is required"));
    }

    #[test]
    fn test_reject_link_missing_type() {
        let (tool, _temp) = test_tool();
        let result = tool.execute(json!({
            "content": "A claim.",
            "links": [{"target": "abc"}],
            "tags": ["test"]
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'type'"));
    }

    #[test]
    fn test_reject_link_missing_target() {
        let (tool, _temp) = test_tool();
        let result = tool.execute(json!({
            "content": "A claim.",
            "links": [{"type": "extends"}],
            "tags": ["test"]
        }));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing 'target'"));
    }
}

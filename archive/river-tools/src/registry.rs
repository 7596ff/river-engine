use river_core::RiverError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Tool execution result (success case)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    pub output_file: Option<String>,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            output_file: None,
        }
    }

    pub fn with_file(output: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            output_file: Some(file.into()),
        }
    }
}

/// JSON Schema for tool parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON Schema object
}

/// Tool trait - implemented by each tool
pub trait Tool: Send + Sync {
    /// Tool name
    fn name(&self) -> &str;

    /// Tool description
    fn description(&self) -> &str;

    /// Parameter schema (JSON Schema)
    fn parameters(&self) -> Value;

    /// Execute the tool with given arguments
    /// Returns Ok(ToolResult) on success, Err with descriptive message on failure
    fn execute(&self, args: Value) -> Result<ToolResult, RiverError>;

    /// Get full schema for this tool
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }
}

/// Registry of available tools
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Execute a tool by name
    pub fn execute(&self, name: &str, args: Value) -> Result<ToolResult, RiverError> {
        self.get(name)
            .ok_or_else(|| RiverError::tool(format!("Unknown tool: {}", name)))?
            .execute(args)
    }

    /// Get number of registered tools
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Get all tool schemas (for sending to model)
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// List tool names
    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool;

    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "A dummy tool for testing"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                },
                "required": ["input"]
            })
        }
        fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
            let input = args
                .get("input")
                .and_then(|v| v.as_str())
                .ok_or_else(|| RiverError::tool("Missing required parameter: input"))?;
            Ok(ToolResult::success(format!("Received: {}", input)))
        }
    }

    #[test]
    fn test_registry() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool));

        assert!(registry.get("dummy").is_some());
        assert!(registry.get("nonexistent").is_none());
        assert_eq!(registry.names().len(), 1);
    }

    #[test]
    fn test_tool_execution() {
        let tool = DummyTool;
        let result = tool.execute(serde_json::json!({"input": "hello"})).unwrap();
        assert!(result.output.contains("hello"));
    }

    #[test]
    fn test_tool_execution_error() {
        let tool = DummyTool;
        let result = tool.execute(serde_json::json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn test_registry_len_and_is_empty() {
        let mut registry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        registry.register(Box::new(DummyTool));
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_registry_execute() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool));

        let result = registry
            .execute("dummy", serde_json::json!({"input": "test"}))
            .unwrap();
        assert!(result.output.contains("test"));

        let err = registry
            .execute("nonexistent", serde_json::json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("Unknown tool"));
    }
}

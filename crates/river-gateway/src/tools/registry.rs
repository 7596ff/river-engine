use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub output_file: Option<String>,  // If output was redirected to file
}

/// JSON Schema for tool parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,  // JSON Schema object
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
    fn execute(&self, args: Value) -> ToolResult;

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
        fn name(&self) -> &str { "dummy" }
        fn description(&self) -> &str { "A dummy tool for testing" }
        fn parameters(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                },
                "required": ["input"]
            })
        }
        fn execute(&self, args: Value) -> ToolResult {
            let input = args.get("input").and_then(|v| v.as_str()).unwrap_or("");
            ToolResult {
                success: true,
                output: format!("Received: {}", input),
                output_file: None,
            }
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
        let result = tool.execute(serde_json::json!({"input": "hello"}));
        assert!(result.success);
        assert!(result.output.contains("hello"));
    }
}

//! Tool executor with context tracking

use super::registry::{ToolRegistry, ToolResult, ToolSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool call from the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Result of executing a tool call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResponse {
    pub tool_call_id: String,
    pub result: Result<ToolResult, String>,  // String error for serialization
}

/// Executes tools
pub struct ToolExecutor {
    registry: ToolRegistry,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
        }
    }

    /// Execute a tool call
    pub fn execute(&mut self, call: &ToolCall) -> ToolCallResponse {
        tracing::info!(
            tool = %call.name,
            call_id = %call.id,
            args = %serde_json::to_string(&call.arguments).unwrap_or_default(),
            "Executing tool"
        );

        let result = match self.registry.get(&call.name) {
            Some(tool) => {
                tracing::debug!(tool = %call.name, "Tool found in registry, executing...");
                match tool.execute(call.arguments.clone()) {
                    Ok(tool_result) => {
                        let output_len = tool_result.output.len();
                        tracing::info!(
                            tool = %call.name,
                            call_id = %call.id,
                            output_len = output_len,
                            output_preview = %tool_result.output.chars().take(200).collect::<String>(),
                            "Tool succeeded"
                        );
                        Ok(tool_result)
                    }
                    Err(e) => {
                        tracing::error!(
                            tool = %call.name,
                            call_id = %call.id,
                            error = %e,
                            "Tool execution failed"
                        );
                        Err(e.to_string())
                    }
                }
            }
            None => {
                tracing::error!(
                    tool = %call.name,
                    call_id = %call.id,
                    available_tools = ?self.registry.schemas().iter().map(|s| &s.name).collect::<Vec<_>>(),
                    "Unknown tool requested"
                );
                Err(format!("Unknown tool: {}", call.name))
            }
        };

        ToolCallResponse {
            tool_call_id: call.id.clone(),
            result,
        }
    }

    /// Execute multiple tool calls
    pub fn execute_all(&mut self, calls: &[ToolCall]) -> Vec<ToolCallResponse> {
        calls.iter().map(|c| self.execute(c)).collect()
    }

    /// Get all tool schemas
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.registry.schemas()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use river_core::RiverError;

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
    fn test_executor() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool));

        let mut executor = ToolExecutor::new(registry);

        let call = ToolCall {
            id: "call_1".to_string(),
            name: "dummy".to_string(),
            arguments: serde_json::json!({
                "input": "Hello!"
            }),
        };

        let response = executor.execute(&call);
        assert!(response.result.is_ok());
        assert!(response.result.unwrap().output.contains("Hello!"));
    }

    #[test]
    fn test_unknown_tool() {
        let registry = ToolRegistry::new();
        let mut executor = ToolExecutor::new(registry);

        let call = ToolCall {
            id: "call_1".to_string(),
            name: "nonexistent".to_string(),
            arguments: serde_json::json!({}),
        };

        let response = executor.execute(&call);
        assert!(response.result.is_err());
        assert!(response.result.unwrap_err().contains("Unknown tool"));
    }
}

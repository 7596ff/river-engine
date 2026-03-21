//! Tool executor with context tracking

use river_core::ContextStatus;
use super::{ToolRegistry, ToolResult, ToolSchema};
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

/// Executes tools and tracks context
pub struct ToolExecutor {
    registry: ToolRegistry,
    context_used: u64,
    context_limit: u64,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry, context_limit: u64) -> Self {
        Self {
            registry,
            context_used: 0,
            context_limit,
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
                        // Rough estimate: ~4 characters per token (typical for English text)
                        let output_len = tool_result.output.len();
                        self.context_used += (output_len as u64) / 4;
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

    /// Get current context status
    pub fn context_status(&self) -> ContextStatus {
        ContextStatus {
            used: self.context_used,
            limit: self.context_limit,
        }
    }

    /// Get all tool schemas
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.registry.schemas()
    }

    /// Update context usage
    pub fn add_context(&mut self, tokens: u64) {
        self.context_used += tokens;
    }

    /// Reset context (on rotation)
    pub fn reset_context(&mut self) {
        self.context_used = 0;
    }

    /// Check if context is getting full (>90%)
    pub fn context_warning(&self) -> bool {
        self.context_status().percent() >= 90.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ReadTool, WriteTool};
    use tempfile::TempDir;

    #[test]
    fn test_executor() {
        let dir = TempDir::new().unwrap();
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ReadTool::new(dir.path())));
        registry.register(Box::new(WriteTool::new(dir.path())));

        let mut executor = ToolExecutor::new(registry, 65536);

        // Write a file
        let write_call = ToolCall {
            id: "call_1".to_string(),
            name: "write".to_string(),
            arguments: serde_json::json!({
                "path": "test.txt",
                "content": "Hello!"
            }),
        };

        let response = executor.execute(&write_call);
        assert!(response.result.is_ok());

        // Read it back
        let read_call = ToolCall {
            id: "call_2".to_string(),
            name: "read".to_string(),
            arguments: serde_json::json!({
                "path": "test.txt"
            }),
        };

        let response = executor.execute(&read_call);
        assert!(response.result.is_ok());
        assert!(response.result.unwrap().output.contains("Hello!"));
    }

    #[test]
    fn test_context_tracking() {
        let registry = ToolRegistry::new();
        let mut executor = ToolExecutor::new(registry, 1000);

        executor.add_context(500);
        assert_eq!(executor.context_status().used, 500);
        assert_eq!(executor.context_status().percent(), 50.0);

        executor.add_context(450);
        assert!(executor.context_warning()); // 95% used

        executor.reset_context();
        assert_eq!(executor.context_status().used, 0);
    }

    #[test]
    fn test_unknown_tool() {
        let registry = ToolRegistry::new();
        let mut executor = ToolExecutor::new(registry, 65536);

        let call = ToolCall {
            id: "call_1".to_string(),
            name: "nonexistent".to_string(),
            arguments: serde_json::json!({}),
        };

        let response = executor.execute(&call);
        assert!(response.result.is_err());
        assert!(response.result.unwrap_err().contains("Unknown tool"));
    }

    #[test]
    fn test_context_warning_threshold() {
        let registry = ToolRegistry::new();
        let mut executor = ToolExecutor::new(registry, 1000);

        executor.add_context(895);
        assert!(!executor.context_warning()); // 89.5%

        executor.add_context(6);
        assert!(executor.context_warning()); // 90.1%
    }
}

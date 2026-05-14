//! Tool dispatch wrapper for the agent task
//!
//! Provides a clean interface for executing tools within the agent task,
//! handling the async locking of the tool executor.

use crate::tools::{ToolCall, ToolCallResponse, ToolExecutor};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Execute a single tool call through the executor
pub async fn execute_tool(
    executor: &Arc<RwLock<ToolExecutor>>,
    call: &ToolCall,
) -> (ToolCallResponse, std::time::Duration) {
    let start = Instant::now();
    let mut executor = executor.write().await;
    let response = executor.execute(call);
    let duration = start.elapsed();
    (response, duration)
}

/// Execute a batch of tool calls sequentially
///
/// Returns results paired with execution duration for each call.
pub async fn execute_tools(
    executor: &Arc<RwLock<ToolExecutor>>,
    calls: &[ToolCall],
) -> Vec<(ToolCallResponse, std::time::Duration)> {
    let mut results = Vec::with_capacity(calls.len());

    for call in calls {
        let (response, duration) = execute_tool(executor, call).await;
        tracing::info!(
            tool = %call.name,
            call_id = %call.id,
            success = response.result.is_ok(),
            duration_ms = duration.as_millis(),
            "Tool executed"
        );
        results.push((response, duration));
    }

    results
}

/// Summary of tool execution results
#[derive(Debug, Default)]
pub struct ToolExecutionStats {
    pub total_calls: usize,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_duration_ms: u128,
}

impl ToolExecutionStats {
    /// Create stats from a batch of tool results
    pub fn from_results(results: &[(ToolCallResponse, std::time::Duration)]) -> Self {
        let mut stats = Self::default();
        for (response, duration) in results {
            stats.total_calls += 1;
            if response.result.is_ok() {
                stats.successful_calls += 1;
            } else {
                stats.failed_calls += 1;
            }
            stats.total_duration_ms += duration.as_millis();
        }
        stats
    }

    /// Check if all calls succeeded
    pub fn all_succeeded(&self) -> bool {
        self.failed_calls == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use serde_json::json;

    #[tokio::test]
    async fn test_execute_single_tool() {
        let executor = Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new())));

        // Read tool should exist by default
        let call = ToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            arguments: json!({"path": "/nonexistent/path"}),
        };

        let (response, duration) = execute_tool(&executor, &call).await;

        // Should get an error (file doesn't exist) but tool was found
        assert!(duration.as_millis() < 1000); // Should be fast
        assert_eq!(response.tool_call_id, "call_1");
    }

    #[tokio::test]
    async fn test_execute_multiple_tools() {
        let executor = Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new())));

        let calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "read".to_string(),
                arguments: json!({"path": "/nonexistent/1"}),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "read".to_string(),
                arguments: json!({"path": "/nonexistent/2"}),
            },
        ];

        let results = execute_tools(&executor, &calls).await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.tool_call_id, "call_1");
        assert_eq!(results[1].0.tool_call_id, "call_2");
    }

    #[test]
    fn test_execution_stats() {
        use crate::tools::ToolResult;

        let results = vec![
            (
                ToolCallResponse {
                    tool_call_id: "1".to_string(),
                    result: Ok(ToolResult {
                        output: "ok".to_string(),
                        output_file: None,
                    }),
                },
                std::time::Duration::from_millis(10),
            ),
            (
                ToolCallResponse {
                    tool_call_id: "2".to_string(),
                    result: Err("failed".to_string()),
                },
                std::time::Duration::from_millis(5),
            ),
            (
                ToolCallResponse {
                    tool_call_id: "3".to_string(),
                    result: Ok(ToolResult {
                        output: "ok".to_string(),
                        output_file: None,
                    }),
                },
                std::time::Duration::from_millis(15),
            ),
        ];

        let stats = ToolExecutionStats::from_results(&results);

        assert_eq!(stats.total_calls, 3);
        assert_eq!(stats.successful_calls, 2);
        assert_eq!(stats.failed_calls, 1);
        assert_eq!(stats.total_duration_ms, 30);
        assert!(!stats.all_succeeded());
    }

    #[test]
    fn test_all_succeeded() {
        use crate::tools::ToolResult;

        let results = vec![(
            ToolCallResponse {
                tool_call_id: "1".to_string(),
                result: Ok(ToolResult {
                    output: "ok".to_string(),
                    output_file: None,
                }),
            },
            std::time::Duration::from_millis(10),
        )];

        let stats = ToolExecutionStats::from_results(&results);
        assert!(stats.all_succeeded());
    }
}

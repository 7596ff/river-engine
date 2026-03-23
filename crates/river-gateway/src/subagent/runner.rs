//! SubagentRunner - simplified agent loop for subagents
//!
//! Key differences from main AgentLoop:
//! - No heartbeat scheduling
//! - No external message queue
//! - TaskWorker terminates when model returns no tool calls
//! - LongRunning waits for messages or shutdown
//! - Uses internal queue for parent communication

use super::{InternalQueue, SubagentResult, SubagentStatus, SubagentType};
use crate::preferences::{Preferences, format_current_time};
use crate::r#loop::{ChatMessage, ContextBuilder, ModelClient, ToolCallRequest};
use crate::tools::{ToolCall, ToolExecutor, ToolRegistry};
use river_core::{RiverError, Snowflake};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Configuration for a subagent runner
pub struct SubagentConfig {
    /// Workspace path
    pub workspace: PathBuf,
    /// Context limit (tokens)
    pub context_limit: u64,
    /// Maximum tool calls per generation
    pub max_tool_calls: usize,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            context_limit: 32768, // Subagents get smaller context by default
            max_tool_calls: 25,
        }
    }
}

/// Simplified agent loop for subagents
pub struct SubagentRunner {
    id: Snowflake,
    subagent_type: SubagentType,
    task: String,
    model_client: ModelClient,
    tool_executor: ToolExecutor,
    queue: Arc<InternalQueue>,
    shutdown_rx: oneshot::Receiver<()>,
    result_tx: Option<oneshot::Sender<SubagentResult>>,
    context: ContextBuilder,
    config: SubagentConfig,
}

impl SubagentRunner {
    pub fn new(
        id: Snowflake,
        subagent_type: SubagentType,
        task: String,
        model_client: ModelClient,
        registry: ToolRegistry,
        queue: Arc<InternalQueue>,
        shutdown_rx: oneshot::Receiver<()>,
        result_tx: oneshot::Sender<SubagentResult>,
        config: SubagentConfig,
    ) -> Self {
        let tool_executor = ToolExecutor::new(registry);

        Self {
            id,
            subagent_type,
            task,
            model_client,
            tool_executor,
            queue,
            shutdown_rx,
            result_tx: Some(result_tx),
            context: ContextBuilder::new(),
            config,
        }
    }

    /// Run the subagent loop
    pub async fn run(mut self) -> SubagentResult {
        tracing::info!(
            "Subagent {} starting (type: {}, task: {})",
            self.id,
            self.subagent_type,
            self.task
        );

        // Build initial context
        self.build_initial_context().await;

        let result = match self.subagent_type {
            SubagentType::TaskWorker => self.run_task_worker().await,
            SubagentType::LongRunning => self.run_long_running().await,
        };

        // Send result through channel
        let subagent_result = match result {
            Ok(output) => SubagentResult {
                id: self.id,
                status: SubagentStatus::Completed,
                result: Some(output),
                error: None,
            },
            Err(e) => SubagentResult {
                id: self.id,
                status: SubagentStatus::Failed,
                result: None,
                error: Some(e.to_string()),
            },
        };

        if let Some(tx) = self.result_tx.take() {
            let _ = tx.send(subagent_result.clone());
        }

        tracing::info!(
            "Subagent {} finished with status: {}",
            self.id,
            subagent_result.status
        );

        subagent_result
    }

    /// Build the initial context for the subagent
    async fn build_initial_context(&mut self) {
        self.context.clear();

        // System prompt for subagent
        let prefs = Preferences::load(&self.config.workspace);
        let time_str = format_current_time(prefs.timezone());
        let system_prompt = format!(
            "You are a subagent (ID: {}) spawned to complete a specific task.\n\n\
             Your task: {}\n\n\
             Guidelines:\n\
             - Focus only on the assigned task\n\
             - Use the available tools to complete the task\n\
             - Report your findings/results clearly\n\
             - You can send messages to parent using internal_send tool\n\
             - When done, simply stop making tool calls\n\n\
             Current time: {}",
            self.id,
            self.task,
            time_str
        );
        self.context.add_message(ChatMessage::system(system_prompt));

        // Add the task as a user message
        self.context
            .add_message(ChatMessage::user(format!("Execute task: {}", self.task)));

        // Set available tools
        self.context.set_tools(self.tool_executor.schemas());
    }

    /// Run as a task worker (terminates when no tool calls)
    async fn run_task_worker(&mut self) -> Result<String, RiverError> {
        let mut last_content = String::new();
        let mut iterations = 0;
        let max_iterations = 50; // Safety limit

        loop {
            // Check for shutdown
            if self.check_shutdown() {
                return Err(RiverError::tool("Subagent stopped by parent"));
            }

            // Check for parent messages
            self.process_parent_messages();

            // Call model
            let response = self
                .model_client
                .complete(self.context.messages(), self.context.tools())
                .await?;

            // Add assistant response to context
            let tool_calls = if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            };
            self.context
                .add_assistant_response(response.content.clone(), tool_calls);

            // Save content for result
            if let Some(content) = &response.content {
                last_content = content.clone();
            }

            // No tool calls = task complete
            if response.tool_calls.is_empty() {
                tracing::debug!("Subagent {} completed (no tool calls)", self.id);
                return Ok(last_content);
            }

            // Execute tool calls
            self.execute_tools(&response.tool_calls).await?;

            iterations += 1;
            if iterations >= max_iterations {
                return Err(RiverError::tool(format!(
                    "Subagent exceeded maximum iterations ({})",
                    max_iterations
                )));
            }
        }
    }

    /// Run as a long-running agent (waits for messages)
    async fn run_long_running(&mut self) -> Result<String, RiverError> {
        let mut last_content = String::new();

        loop {
            // Wait for messages or shutdown
            tokio::select! {
                _ = &mut self.shutdown_rx => {
                    tracing::info!("Subagent {} received shutdown signal", self.id);
                    return Ok(last_content);
                }
                // Poll for messages every 100ms
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                    if !self.queue.has_messages_for_subagent() {
                        continue;
                    }
                }
            }

            // Process incoming messages
            self.process_parent_messages();

            // Call model
            let response = self
                .model_client
                .complete(self.context.messages(), self.context.tools())
                .await?;

            let tool_calls = if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            };
            self.context
                .add_assistant_response(response.content.clone(), tool_calls);

            if let Some(content) = &response.content {
                last_content = content.clone();
            }

            // Execute tool calls if any
            if !response.tool_calls.is_empty() {
                self.execute_tools(&response.tool_calls).await?;
            }
        }
    }

    /// Check if shutdown was requested
    fn check_shutdown(&mut self) -> bool {
        match self.shutdown_rx.try_recv() {
            Ok(()) => true,
            Err(oneshot::error::TryRecvError::Empty) => false,
            Err(oneshot::error::TryRecvError::Closed) => true,
        }
    }

    /// Process messages from parent and add to context
    fn process_parent_messages(&mut self) {
        let messages = self.queue.drain_for_subagent();
        if messages.is_empty() {
            return;
        }

        tracing::debug!(
            "Subagent {} processing {} parent messages",
            self.id,
            messages.len()
        );

        for msg in messages {
            self.context.add_message(ChatMessage::system(format!(
                "[Parent Message] {}",
                msg.content
            )));
        }
    }

    /// Execute tool calls and add results to context
    async fn execute_tools(&mut self, tool_calls: &[ToolCallRequest]) -> Result<(), RiverError> {
        let mut results = Vec::new();

        for tc in tool_calls.iter().take(self.config.max_tool_calls) {
            let arguments = match serde_json::from_str(&tc.function.arguments) {
                Ok(args) => args,
                Err(e) => {
                    tracing::warn!(
                        "Invalid JSON arguments for tool {}: {}",
                        tc.function.name,
                        e
                    );
                    serde_json::Value::Object(serde_json::Map::new())
                }
            };

            let call = ToolCall {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments,
            };

            let result = self.tool_executor.execute(&call);
            tracing::debug!(
                "Subagent {} tool {}: {:?}",
                self.id,
                tc.function.name,
                result.result.is_ok()
            );
            results.push(result);
        }

        // Add results to context
        self.context
            .add_tool_results(results, Vec::new());

        Ok(())
    }
}

/// Create a filtered tool registry for subagents
///
/// Subagents get most tools but NOT subagent management tools (no recursion)
pub fn create_subagent_registry(
    workspace: &std::path::Path,
    subagent_id: Snowflake,
    queue: Arc<InternalQueue>,
) -> ToolRegistry {
    use crate::tools::{
        BashTool, EditTool, GlobTool, GrepTool, ReadTool, WriteTool,
    };

    let mut registry = ToolRegistry::new();

    // Core file tools
    registry.register(Box::new(ReadTool::new(workspace)));
    registry.register(Box::new(WriteTool::new(workspace)));
    registry.register(Box::new(EditTool::new(workspace)));
    registry.register(Box::new(GlobTool::new(workspace)));
    registry.register(Box::new(GrepTool::new(workspace)));
    registry.register(Box::new(BashTool::new(workspace)));

    // Internal communication tool (subagent -> parent only)
    registry.register(Box::new(InternalSendToParentTool::new(subagent_id, queue)));

    // Note: Subagents do NOT get:
    // - spawn_subagent, list_subagents, subagent_status, stop_subagent, wait_for_subagent
    // - internal_receive (parent-side only)
    // Memory tools and web tools can be added if needed

    registry
}

/// Tool for subagent to send messages to parent
struct InternalSendToParentTool {
    subagent_id: Snowflake,
    queue: Arc<InternalQueue>,
}

impl InternalSendToParentTool {
    fn new(subagent_id: Snowflake, queue: Arc<InternalQueue>) -> Self {
        Self { subagent_id, queue }
    }
}

impl crate::tools::Tool for InternalSendToParentTool {
    fn name(&self) -> &str {
        "internal_send"
    }

    fn description(&self) -> &str {
        "Send a message to the parent agent"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Message content to send to parent"
                }
            },
            "required": ["content"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Result<crate::tools::ToolResult, RiverError> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: content"))?;

        self.queue.send_to_parent(content);

        Ok(crate::tools::ToolResult::success(format!(
            "Message sent to parent from subagent {}",
            self.subagent_id
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_config_default() {
        let config = SubagentConfig::default();
        assert_eq!(config.context_limit, 32768);
        assert_eq!(config.max_tool_calls, 25);
    }

    fn test_snowflake() -> Snowflake {
        // For tests, we just need a unique ID
        Snowflake::from_parts(1, 0x0400000000000001) // type 0x04 = Subagent
    }

    #[test]
    fn test_create_subagent_registry() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let id = test_snowflake();
        let queue = Arc::new(InternalQueue::new());

        let registry = create_subagent_registry(dir.path(), id, queue);

        // Should have core tools plus internal_send
        let names = registry.names();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"write"));
        assert!(names.contains(&"edit"));
        assert!(names.contains(&"glob"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"internal_send"));

        // Should NOT have subagent management tools
        assert!(!names.contains(&"spawn_subagent"));
        assert!(!names.contains(&"list_subagents"));
    }

    #[test]
    fn test_internal_send_tool() {
        use crate::tools::Tool;

        let id = test_snowflake();
        let queue = Arc::new(InternalQueue::new());
        let tool = InternalSendToParentTool::new(id, queue.clone());

        let result = tool
            .execute(serde_json::json!({"content": "Hello parent"}))
            .unwrap();
        assert!(result.output.contains("sent to parent"));

        let messages = queue.drain_for_parent();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello parent");
    }
}

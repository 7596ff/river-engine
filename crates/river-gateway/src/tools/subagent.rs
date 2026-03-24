//! Subagent tools for the parent agent
//!
//! These tools allow the parent agent to spawn and manage subagents.

use crate::r#loop::ModelClient;
use crate::subagent::{
    create_subagent_registry, SubagentConfig, SubagentManager, SubagentRunner,
    SubagentResult, SubagentType,
};
use river_tools::{Tool, ToolResult};
use river_core::{RiverError, Snowflake};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, RwLock};

/// Parse a Snowflake ID from a string in format "high-low" (hex)
fn parse_snowflake(s: &str) -> Result<Snowflake, RiverError> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return Err(RiverError::tool(format!(
            "Invalid snowflake format: expected 'high-low', got '{}'",
            s
        )));
    }

    let high = u64::from_str_radix(parts[0], 16).map_err(|e| {
        RiverError::tool(format!("Invalid snowflake high part: {}", e))
    })?;
    let low = u64::from_str_radix(parts[1], 16).map_err(|e| {
        RiverError::tool(format!("Invalid snowflake low part: {}", e))
    })?;

    Ok(Snowflake::from_parts(high, low))
}

/// Tool to spawn a new subagent
pub struct SpawnSubagentTool {
    manager: Arc<RwLock<SubagentManager>>,
    workspace: PathBuf,
    model_url: String,
}

impl SpawnSubagentTool {
    pub fn new(
        manager: Arc<RwLock<SubagentManager>>,
        workspace: PathBuf,
        model_url: String,
        _model_name: String, // Accepted for compatibility but not stored; model comes from args
    ) -> Self {
        Self {
            manager,
            workspace,
            model_url,
        }
    }
}

impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn a new subagent to handle a task. Returns the subagent ID."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Task description for the subagent"
                },
                "model": {
                    "type": "string",
                    "description": "Model to use"
                },
                "type": {
                    "type": "string",
                    "enum": ["task_worker", "long_running"],
                    "description": "Type of subagent: task_worker (terminates on completion) or long_running (waits for messages)"
                },
                "priority": {
                    "type": "string",
                    "enum": ["interactive", "scheduled", "background"],
                    "description": "Priority level (default: background)"
                }
            },
            "required": ["task", "model", "type"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: task"))?
            .to_string();

        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: model"))?
            .to_string();

        let subagent_type = args
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: type"))
            .map(|s| match s {
                "long_running" => SubagentType::LongRunning,
                _ => SubagentType::TaskWorker,
            })?;

        // Priority is accepted but not yet used (orchestrator integration needed)
        let _priority = args
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("background");

        // Use block_in_place for async operations in sync context
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Register the subagent
                let (id, queue) = {
                    let mut manager = self.manager.write().await;
                    manager.register(subagent_type, task.clone(), model.clone())
                };

                // Create channels
                let (shutdown_tx, shutdown_rx) = oneshot::channel();
                let (result_tx, result_rx) = oneshot::channel();

                // Set up channels in manager
                {
                    let mut manager = self.manager.write().await;
                    manager.set_channels(id, shutdown_tx, result_rx);
                }

                // Create model client for subagent
                let model_client = ModelClient::new(
                    self.model_url.clone(),
                    model.clone(),
                    Duration::from_secs(120),
                )?;

                // Create filtered tool registry for subagent
                let registry = create_subagent_registry(&self.workspace, id, queue.clone());

                // Create runner config
                let config = SubagentConfig {
                    workspace: self.workspace.clone(),
                    context_limit: 32768,
                    max_tool_calls: 25,
                };

                // Create the runner
                let runner = SubagentRunner::new(
                    id,
                    subagent_type,
                    task.clone(),
                    model_client,
                    registry,
                    queue,
                    shutdown_rx,
                    result_tx,
                    config,
                );

                // Mark as running
                {
                    let mut manager = self.manager.write().await;
                    manager.set_running(id);
                }

                // Spawn the subagent task
                let manager_clone = self.manager.clone();
                tokio::spawn(async move {
                    let result = runner.run().await;

                    // Update manager with final status
                    let mut manager = manager_clone.write().await;
                    match result.status {
                        crate::subagent::SubagentStatus::Completed => {
                            manager.set_completed(
                                result.id,
                                result.result.unwrap_or_default(),
                            );
                        }
                        crate::subagent::SubagentStatus::Failed => {
                            manager.set_failed(
                                result.id,
                                result.error.unwrap_or_else(|| "Unknown error".to_string()),
                            );
                        }
                        _ => {}
                    }
                });

                Ok::<Snowflake, RiverError>(id)
            })
        })?;

        Ok(ToolResult::success(format!(
            "Spawned subagent {} (type: {}, task: {})",
            result,
            subagent_type,
            task.chars().take(50).collect::<String>()
        )))
    }
}

/// Tool to list all subagents
pub struct ListSubagentsTool {
    manager: Arc<RwLock<SubagentManager>>,
}

impl ListSubagentsTool {
    pub fn new(manager: Arc<RwLock<SubagentManager>>) -> Self {
        Self { manager }
    }
}

impl Tool for ListSubagentsTool {
    fn name(&self) -> &str {
        "list_subagents"
    }

    fn description(&self) -> &str {
        "List all subagents and their current status"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    fn execute(&self, _args: Value) -> Result<ToolResult, RiverError> {
        let list = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let manager = self.manager.read().await;
                manager.list()
            })
        });

        if list.is_empty() {
            return Ok(ToolResult::success("No subagents"));
        }

        let json = serde_json::to_string_pretty(&list)
            .map_err(|e| RiverError::tool(format!("Failed to serialize: {}", e)))?;

        Ok(ToolResult::success(json))
    }
}

/// Tool to get status of a specific subagent
pub struct SubagentStatusTool {
    manager: Arc<RwLock<SubagentManager>>,
}

impl SubagentStatusTool {
    pub fn new(manager: Arc<RwLock<SubagentManager>>) -> Self {
        Self { manager }
    }
}

impl Tool for SubagentStatusTool {
    fn name(&self) -> &str {
        "subagent_status"
    }

    fn description(&self) -> &str {
        "Get the status of a specific subagent"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Subagent ID"
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let id_str = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: id"))?;

        let id = parse_snowflake(id_str)?;

        let info = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let manager = self.manager.read().await;
                manager.get(id)
            })
        });

        match info {
            Some(info) => {
                let json = serde_json::to_string_pretty(&info)
                    .map_err(|e| RiverError::tool(format!("Failed to serialize: {}", e)))?;
                Ok(ToolResult::success(json))
            }
            None => Err(RiverError::tool(format!("Subagent {} not found", id))),
        }
    }
}

/// Tool to stop a subagent
pub struct StopSubagentTool {
    manager: Arc<RwLock<SubagentManager>>,
}

impl StopSubagentTool {
    pub fn new(manager: Arc<RwLock<SubagentManager>>) -> Self {
        Self { manager }
    }
}

impl Tool for StopSubagentTool {
    fn name(&self) -> &str {
        "stop_subagent"
    }

    fn description(&self) -> &str {
        "Stop a running subagent"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Subagent ID to stop"
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let id_str = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: id"))?;

        let id = parse_snowflake(id_str)?;

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut manager = self.manager.write().await;
                manager.stop(id)
            })
        })?;

        Ok(ToolResult::success(format!("Stopped subagent {}", id)))
    }
}

/// Tool to send a message to a subagent
pub struct InternalSendTool {
    manager: Arc<RwLock<SubagentManager>>,
}

impl InternalSendTool {
    pub fn new(manager: Arc<RwLock<SubagentManager>>) -> Self {
        Self { manager }
    }
}

impl Tool for InternalSendTool {
    fn name(&self) -> &str {
        "internal_send"
    }

    fn description(&self) -> &str {
        "Send a message to a subagent"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Subagent ID to send to"
                },
                "content": {
                    "type": "string",
                    "description": "Message content"
                }
            },
            "required": ["to", "content"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let to_str = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: to"))?;

        let to = parse_snowflake(to_str)?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: content"))?;

        let queue = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let manager = self.manager.read().await;
                manager.queue(to)
            })
        });

        match queue {
            Some(queue) => {
                queue.send_to_subagent(content);
                Ok(ToolResult::success(format!("Message sent to subagent {}", to)))
            }
            None => Err(RiverError::tool(format!("Subagent {} not found", to))),
        }
    }
}

/// Tool to receive messages from subagents
pub struct InternalReceiveTool {
    manager: Arc<RwLock<SubagentManager>>,
}

impl InternalReceiveTool {
    pub fn new(manager: Arc<RwLock<SubagentManager>>) -> Self {
        Self { manager }
    }
}

impl Tool for InternalReceiveTool {
    fn name(&self) -> &str {
        "internal_receive"
    }

    fn description(&self) -> &str {
        "Receive messages from subagents. Returns messages sent by subagents to parent."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "from": {
                    "type": "string",
                    "description": "Specific subagent ID to receive from (optional, receives from all if not specified)"
                }
            }
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let from_id: Option<Snowflake> = args
            .get("from")
            .and_then(|v| v.as_str())
            .map(parse_snowflake)
            .transpose()?;

        let messages: Vec<(Snowflake, Vec<crate::subagent::InternalMessage>)> =
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let manager = self.manager.read().await;

                    match from_id {
                        Some(id) => {
                            if let Some(queue) = manager.queue(id) {
                                vec![(id, queue.drain_for_parent())]
                            } else {
                                vec![]
                            }
                        }
                        None => {
                            // Drain from all subagents
                            manager
                                .list()
                                .iter()
                                .filter_map(|info| {
                                    manager.queue(info.id).map(|q| (info.id, q.drain_for_parent()))
                                })
                                .filter(|(_, msgs)| !msgs.is_empty())
                                .collect()
                        }
                    }
                })
            });

        if messages.is_empty() || messages.iter().all(|(_, m)| m.is_empty()) {
            return Ok(ToolResult::success("No messages"));
        }

        let formatted: Vec<serde_json::Value> = messages
            .into_iter()
            .flat_map(|(id, msgs)| {
                msgs.into_iter().map(move |msg| {
                    serde_json::json!({
                        "from": id.to_string(),
                        "content": msg.content,
                        "timestamp": msg.timestamp
                    })
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&formatted)
            .map_err(|e| RiverError::tool(format!("Failed to serialize: {}", e)))?;

        Ok(ToolResult::success(json))
    }
}

/// Tool to wait for a subagent to complete
pub struct WaitForSubagentTool {
    manager: Arc<RwLock<SubagentManager>>,
}

impl WaitForSubagentTool {
    pub fn new(manager: Arc<RwLock<SubagentManager>>) -> Self {
        Self { manager }
    }
}

impl Tool for WaitForSubagentTool {
    fn name(&self) -> &str {
        "wait_for_subagent"
    }

    fn description(&self) -> &str {
        "Wait for a subagent to complete. Blocks until the subagent finishes or timeout."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Subagent ID to wait for"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (optional, default: 300000)"
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let id_str = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: id"))?;

        let id = parse_snowflake(id_str)?;

        // Timeout in milliseconds, default 5 minutes
        let timeout_ms = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(300_000);

        // Check if already complete
        let info = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let manager = self.manager.read().await;
                manager.get(id)
            })
        });

        let info = info.ok_or_else(|| RiverError::tool(format!("Subagent {} not found", id)))?;

        if info.status.is_terminal() {
            let result = SubagentResult::from(&info);
            let json = serde_json::to_string_pretty(&result)
                .map_err(|e| RiverError::tool(format!("Failed to serialize: {}", e)))?;
            return Ok(ToolResult::success(json));
        }

        // Take the result receiver and wait
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Take the receiver
                let result_rx = {
                    let mut manager = self.manager.write().await;
                    manager.take_result_rx(id)
                };

                let Some(result_rx) = result_rx else {
                    // Check status again - might have completed
                    let manager = self.manager.read().await;
                    if let Some(info) = manager.get(id) {
                        if info.status.is_terminal() {
                            return Ok(SubagentResult::from(&info));
                        }
                    }
                    return Err(RiverError::tool("Result channel not available"));
                };

                // Wait with timeout
                match tokio::time::timeout(Duration::from_millis(timeout_ms), result_rx).await {
                    Ok(Ok(result)) => Ok(result),
                    Ok(Err(_)) => {
                        // Channel closed, check final status
                        let manager = self.manager.read().await;
                        if let Some(info) = manager.get(id) {
                            Ok(SubagentResult::from(&info))
                        } else {
                            Err(RiverError::tool("Subagent disappeared"))
                        }
                    }
                    Err(_) => Err(RiverError::tool(format!(
                        "Timeout waiting for subagent {} ({}ms)",
                        id, timeout_ms
                    ))),
                }
            })
        })?;

        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| RiverError::tool(format!("Failed to serialize: {}", e)))?;

        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::AgentBirth;
    use river_core::SnowflakeGenerator;

    fn test_manager() -> Arc<RwLock<SubagentManager>> {
        let birth = AgentBirth::new(2026, 3, 17, 12, 0, 0).unwrap();
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(birth));
        Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_list_subagents_empty() {
        let manager = test_manager();
        let tool = ListSubagentsTool::new(manager);

        let result = tool.execute(serde_json::json!({})).unwrap();
        assert_eq!(result.output, "No subagents");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_subagent_status_not_found() {
        let manager = test_manager();
        let tool = SubagentStatusTool::new(manager);

        let result = tool.execute(serde_json::json!({"id": "0000000000000001-0400000000000001"}));
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_internal_receive_no_messages() {
        let manager = test_manager();
        let tool = InternalReceiveTool::new(manager);

        let result = tool.execute(serde_json::json!({})).unwrap();
        assert_eq!(result.output, "No messages");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_internal_send_not_found() {
        let manager = test_manager();
        let tool = InternalSendTool::new(manager);

        let result = tool.execute(serde_json::json!({
            "to": "0000000000000001-0400000000000001",
            "content": "Hello"
        }));
        assert!(result.is_err());
    }
}

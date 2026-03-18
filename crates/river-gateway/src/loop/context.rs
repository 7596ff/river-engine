//! Context assembly for model calls

use crate::api::IncomingMessage;
use crate::tools::{ToolCallResponse, ToolSchema};
use crate::r#loop::state::{ToolCallRequest, WakeTrigger};
use river_core::ContextStatus;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A message in the chat format (OpenAI-compatible)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: Option<String>, tool_calls: Option<Vec<ToolCallRequest>>) -> Self {
        Self {
            role: "assistant".to_string(),
            content,
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }
}

/// Builds conversation context for model calls
pub struct ContextBuilder {
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSchema>,
}

impl ContextBuilder {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            tools: Vec::new(),
        }
    }

    /// Clear all messages (for new cycle)
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Get messages for API call
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Get tools for API call
    pub fn tools(&self) -> &[ToolSchema] {
        &self.tools
    }

    /// Set available tools
    pub fn set_tools(&mut self, tools: Vec<ToolSchema>) {
        self.tools = tools;
    }

    /// Add a message
    pub fn add_message(&mut self, msg: ChatMessage) {
        self.messages.push(msg);
    }

    /// Assemble context for a new wake cycle
    pub async fn assemble(
        &mut self,
        workspace: &Path,
        trigger: WakeTrigger,
        queued_messages: Vec<IncomingMessage>,
    ) {
        // Load system prompt from workspace files
        let system_prompt = self.build_system_prompt(workspace).await;
        self.messages.push(ChatMessage::system(system_prompt));

        // Load continuity state
        if let Some(state) = self.load_continuity_state(workspace).await {
            self.messages.push(ChatMessage::system(format!(
                "Continuing session. Last cycle you were:\n{}",
                state
            )));
        }

        // Add any queued messages first
        for msg in queued_messages {
            self.messages.push(ChatMessage::user(format!(
                "[{}] {}: {}",
                msg.channel, msg.author.name, msg.content
            )));
        }

        // Add wake trigger
        self.messages.push(self.format_trigger(&trigger));
    }

    async fn build_system_prompt(&self, workspace: &Path) -> String {
        let mut parts = Vec::new();

        // Load workspace files
        for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
            if let Ok(content) = tokio::fs::read_to_string(workspace.join(filename)).await {
                parts.push(content);
            }
        }

        // Add system state
        let now = chrono::Utc::now();
        parts.push(format!("Current time: {}", now.to_rfc3339()));

        if parts.is_empty() {
            "You are an AI assistant.".to_string()
        } else {
            parts.join("\n\n---\n\n")
        }
    }

    async fn load_continuity_state(&self, workspace: &Path) -> Option<String> {
        let path = workspace.join("thinking/current-state.md");
        tokio::fs::read_to_string(path).await.ok()
    }

    fn format_trigger(&self, trigger: &WakeTrigger) -> ChatMessage {
        match trigger {
            WakeTrigger::Message(msg) => ChatMessage::user(format!(
                "[{}] {}: {}",
                msg.channel, msg.author.name, msg.content
            )),
            WakeTrigger::Heartbeat => ChatMessage::system(
                "Heartbeat wake. No new messages. Check on your tasks and state."
            ),
        }
    }

    /// Add tool results with any incoming messages
    pub fn add_tool_results(
        &mut self,
        results: Vec<ToolCallResponse>,
        incoming: Vec<IncomingMessage>,
        context_status: ContextStatus,
    ) {
        // Add each tool result
        for result in results {
            let content = match result.result {
                Ok(r) => r.output,
                Err(e) => format!("Error: {}", e),
            };
            self.messages.push(ChatMessage::tool(result.tool_call_id, content));
        }

        // Add context status
        self.messages.push(ChatMessage::system(format!(
            "Context: {}/{} ({:.1}%)",
            context_status.used, context_status.limit, context_status.percent()
        )));

        // Add any incoming messages
        if !incoming.is_empty() {
            let mut content = String::from("Messages received during tool execution:\n");
            for msg in incoming {
                content.push_str(&format!(
                    "- [{}] {}: {}\n",
                    msg.channel, msg.author.name, msg.content
                ));
            }
            self.messages.push(ChatMessage::system(content));
        }
    }

    /// Add assistant message from model response
    pub fn add_assistant_response(&mut self, content: Option<String>, tool_calls: Option<Vec<ToolCallRequest>>) {
        self.messages.push(ChatMessage::assistant(content, tool_calls));
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Author;
    use crate::r#loop::state::FunctionCall;
    use crate::tools::{ToolCallResponse, ToolResult};
    use river_core::Priority;

    fn test_message(content: &str, channel: &str, author: &str) -> IncomingMessage {
        IncomingMessage {
            adapter: "test".to_string(),
            event_type: "message".to_string(),
            channel: channel.to_string(),
            author: Author {
                id: "user1".to_string(),
                name: author.to_string(),
            },
            content: content.to_string(),
            message_id: None,
            metadata: None,
            priority: Priority::Interactive,
        }
    }

    #[test]
    fn test_chat_message_system() {
        let msg = ChatMessage::system("Hello");
        assert_eq!(msg.role, "system");
        assert_eq!(msg.content, Some("Hello".to_string()));
        assert!(msg.tool_calls.is_none());
    }

    #[test]
    fn test_chat_message_user() {
        let msg = ChatMessage::user("Hi there");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, Some("Hi there".to_string()));
    }

    #[test]
    fn test_chat_message_tool() {
        let msg = ChatMessage::tool("call_123", "Result");
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.tool_call_id, Some("call_123".to_string()));
        assert_eq!(msg.content, Some("Result".to_string()));
    }

    #[test]
    fn test_chat_message_assistant() {
        let msg = ChatMessage::assistant(Some("Hello".to_string()), None);
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.content, Some("Hello".to_string()));
        assert!(msg.tool_calls.is_none());
    }

    #[test]
    fn test_chat_message_assistant_with_tool_calls() {
        let tool_calls = vec![ToolCallRequest {
            id: "call_1".to_string(),
            r#type: "function".to_string(),
            function: FunctionCall {
                name: "read".to_string(),
                arguments: "{\"path\": \"test.txt\"}".to_string(),
            },
        }];
        let msg = ChatMessage::assistant(None, Some(tool_calls.clone()));
        assert_eq!(msg.role, "assistant");
        assert!(msg.content.is_none());
        assert!(msg.tool_calls.is_some());
        assert_eq!(msg.tool_calls.unwrap().len(), 1);
    }

    #[test]
    fn test_context_builder_clear() {
        let mut builder = ContextBuilder::new();
        builder.add_message(ChatMessage::system("test"));
        assert_eq!(builder.messages().len(), 1);
        builder.clear();
        assert!(builder.messages().is_empty());
    }

    #[test]
    fn test_context_builder_set_tools() {
        let mut builder = ContextBuilder::new();
        assert!(builder.tools().is_empty());

        let tools = vec![ToolSchema {
            name: "test".to_string(),
            description: "A test tool".to_string(),
            parameters: serde_json::json!({}),
        }];
        builder.set_tools(tools);
        assert_eq!(builder.tools().len(), 1);
    }

    #[test]
    fn test_chat_message_serialization() {
        let msg = ChatMessage::system("test");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"system\""));
        assert!(json.contains("\"content\":\"test\""));
        // Optional None fields should be skipped
        assert!(!json.contains("tool_calls"));
    }

    #[test]
    fn test_chat_message_tool_serialization() {
        let msg = ChatMessage::tool("call_abc", "file contents here");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"tool\""));
        assert!(json.contains("\"tool_call_id\":\"call_abc\""));
        assert!(json.contains("\"content\":\"file contents here\""));
    }

    #[test]
    fn test_format_trigger_heartbeat() {
        let builder = ContextBuilder::new();
        let msg = builder.format_trigger(&WakeTrigger::Heartbeat);
        assert_eq!(msg.role, "system");
        assert!(msg.content.as_ref().unwrap().contains("Heartbeat"));
    }

    #[test]
    fn test_format_trigger_message() {
        let builder = ContextBuilder::new();
        let incoming = test_message("Hello!", "general", "Alice");
        let msg = builder.format_trigger(&WakeTrigger::Message(incoming));
        assert_eq!(msg.role, "user");
        let content = msg.content.unwrap();
        assert!(content.contains("[general]"));
        assert!(content.contains("Alice"));
        assert!(content.contains("Hello!"));
    }

    #[test]
    fn test_add_tool_results_basic() {
        let mut builder = ContextBuilder::new();
        let context_status = ContextStatus {
            used: 1000,
            limit: 65536,
        };
        let results = vec![ToolCallResponse {
            tool_call_id: "call_1".to_string(),
            result: Ok(ToolResult::success("Success!")),
            context_status: context_status.clone(),
        }];

        builder.add_tool_results(results, vec![], context_status);

        // Should have 2 messages: tool result + context status
        assert_eq!(builder.messages().len(), 2);
        assert_eq!(builder.messages()[0].role, "tool");
        assert_eq!(builder.messages()[0].content, Some("Success!".to_string()));
        assert_eq!(builder.messages()[1].role, "system");
        assert!(builder.messages()[1].content.as_ref().unwrap().contains("Context:"));
    }

    #[test]
    fn test_add_tool_results_with_error() {
        let mut builder = ContextBuilder::new();
        let context_status = ContextStatus {
            used: 500,
            limit: 65536,
        };
        let results = vec![ToolCallResponse {
            tool_call_id: "call_err".to_string(),
            result: Err("File not found".to_string()),
            context_status: context_status.clone(),
        }];

        builder.add_tool_results(results, vec![], context_status);

        assert_eq!(builder.messages()[0].role, "tool");
        let content = builder.messages()[0].content.as_ref().unwrap();
        assert!(content.contains("Error:"));
        assert!(content.contains("File not found"));
    }

    #[test]
    fn test_add_tool_results_with_incoming_messages() {
        let mut builder = ContextBuilder::new();
        let context_status = ContextStatus {
            used: 2000,
            limit: 65536,
        };
        let results = vec![ToolCallResponse {
            tool_call_id: "call_1".to_string(),
            result: Ok(ToolResult::success("Done")),
            context_status: context_status.clone(),
        }];
        let incoming = vec![
            test_message("Hey!", "dm", "Bob"),
            test_message("Urgent!", "alerts", "System"),
        ];

        builder.add_tool_results(results, incoming, context_status);

        // Should have 3 messages: tool result + context status + incoming messages
        assert_eq!(builder.messages().len(), 3);

        // Check incoming messages notification
        let incoming_msg = &builder.messages()[2];
        assert_eq!(incoming_msg.role, "system");
        let content = incoming_msg.content.as_ref().unwrap();
        assert!(content.contains("Messages received during tool execution"));
        assert!(content.contains("[dm] Bob: Hey!"));
        assert!(content.contains("[alerts] System: Urgent!"));
    }

    #[test]
    fn test_add_assistant_response() {
        let mut builder = ContextBuilder::new();
        builder.add_assistant_response(Some("Hello!".to_string()), None);

        assert_eq!(builder.messages().len(), 1);
        assert_eq!(builder.messages()[0].role, "assistant");
        assert_eq!(builder.messages()[0].content, Some("Hello!".to_string()));
    }

    #[test]
    fn test_add_assistant_response_with_tool_calls() {
        let mut builder = ContextBuilder::new();
        let tool_calls = vec![ToolCallRequest {
            id: "call_xyz".to_string(),
            r#type: "function".to_string(),
            function: FunctionCall {
                name: "write".to_string(),
                arguments: "{}".to_string(),
            },
        }];
        builder.add_assistant_response(None, Some(tool_calls));

        assert_eq!(builder.messages().len(), 1);
        let msg = &builder.messages()[0];
        assert!(msg.content.is_none());
        assert!(msg.tool_calls.is_some());
        assert_eq!(msg.tool_calls.as_ref().unwrap()[0].id, "call_xyz");
    }

    #[test]
    fn test_context_builder_default() {
        let builder = ContextBuilder::default();
        assert!(builder.messages().is_empty());
        assert!(builder.tools().is_empty());
    }

    #[test]
    fn test_tool_call_request_serialization() {
        let tc = ToolCallRequest {
            id: "call_123".to_string(),
            r#type: "function".to_string(),
            function: FunctionCall {
                name: "read".to_string(),
                arguments: "{\"path\": \"test.txt\"}".to_string(),
            },
        };
        let json = serde_json::to_string(&tc).unwrap();
        assert!(json.contains("\"id\":\"call_123\""));
        assert!(json.contains("\"type\":\"function\""));
        assert!(json.contains("\"name\":\"read\""));
    }

    #[test]
    fn test_context_status_display_in_results() {
        let mut builder = ContextBuilder::new();
        let context_status = ContextStatus {
            used: 32768,
            limit: 65536,
        };

        builder.add_tool_results(vec![], vec![], context_status);

        let status_msg = &builder.messages()[0];
        let content = status_msg.content.as_ref().unwrap();
        // Should show percentage
        assert!(content.contains("50.0%") || content.contains("50%"));
    }

    #[test]
    fn test_multiple_tool_results() {
        let mut builder = ContextBuilder::new();
        let context_status = ContextStatus {
            used: 1000,
            limit: 65536,
        };
        let results = vec![
            ToolCallResponse {
                tool_call_id: "call_1".to_string(),
                result: Ok(ToolResult::success("First result")),
                context_status: context_status.clone(),
            },
            ToolCallResponse {
                tool_call_id: "call_2".to_string(),
                result: Ok(ToolResult::success("Second result")),
                context_status: context_status.clone(),
            },
            ToolCallResponse {
                tool_call_id: "call_3".to_string(),
                result: Err("Third failed".to_string()),
                context_status: context_status.clone(),
            },
        ];

        builder.add_tool_results(results, vec![], context_status);

        // 3 tool results + 1 context status
        assert_eq!(builder.messages().len(), 4);

        assert_eq!(builder.messages()[0].tool_call_id, Some("call_1".to_string()));
        assert_eq!(builder.messages()[1].tool_call_id, Some("call_2".to_string()));
        assert_eq!(builder.messages()[2].tool_call_id, Some("call_3".to_string()));
        assert!(builder.messages()[2].content.as_ref().unwrap().contains("Error"));
    }
}

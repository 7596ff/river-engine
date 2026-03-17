//! Context assembly for model calls

use crate::api::IncomingMessage;
use crate::tools::{ToolCallResponse, ToolSchema};
use crate::r#loop::state::WakeTrigger;
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

/// Tool call as returned by the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub r#type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
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
    fn test_context_builder_clear() {
        let mut builder = ContextBuilder::new();
        builder.add_message(ChatMessage::system("test"));
        assert_eq!(builder.messages().len(), 1);
        builder.clear();
        assert!(builder.messages().is_empty());
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
}

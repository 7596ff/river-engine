//! Core types for river-oneshot.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What the user feeds into a cycle.
#[derive(Debug, Clone, Default)]
pub struct CycleInput {
    /// New text from user (if any).
    pub user_message: Option<String>,
    /// Result of last cycle.
    pub previous_output: Option<TurnOutput>,
}

/// What a cycle produces.
#[derive(Debug, Clone)]
pub enum TurnOutput {
    /// Loop A completed: LLM produced a plan.
    Thought(Plan),
    /// Loop B completed: a skill finished executing.
    Action(ActionResult),
}

/// LLM's proposed next steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Human-readable description.
    pub summary: String,
    /// Skills to invoke.
    pub actions: Vec<PlannedAction>,
    /// Message to send back to user.
    pub response: Option<String>,
}

/// A skill invocation the LLM wants to make.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAction {
    /// From LLM's tool_use block.
    pub tool_use_id: String,
    /// Skill name.
    pub skill_name: String,
    /// Parameters for the skill.
    pub parameters: serde_json::Value,
    /// Execution priority (lower = higher priority).
    pub priority: u8,
}

/// Result of running a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    /// Links back to LLM's tool_use request.
    pub tool_use_id: String,
    /// Skill that was executed.
    pub skill_name: String,
    /// Human-readable description of what happened.
    pub description: String,
    /// Result payload.
    pub payload: serde_json::Value,
    /// Whether execution succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

/// A turn in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversationTurn {
    User(String),
    Assistant(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        id: String,
        content: String,
        success: bool,
    },
}

impl ConversationTurn {
    pub fn user(text: impl Into<String>) -> Self {
        Self::User(text.into())
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::Assistant(text.into())
    }

    pub fn tool_use(id: impl Into<String>, name: impl Into<String>, input: serde_json::Value) -> Self {
        Self::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>, success: bool) -> Self {
        Self::ToolResult {
            id: id.into(),
            content: content.into(),
            success,
        }
    }
}

/// Context passed to skill execution.
#[derive(Debug, Clone)]
pub struct SkillContext {
    /// Working directory for file operations.
    pub workspace: std::path::PathBuf,
}

impl Default for SkillContext {
    fn default() -> Self {
        Self {
            workspace: std::env::current_dir().unwrap_or_default(),
        }
    }
}

/// Context for reasoning loop.
#[derive(Debug, Clone)]
pub struct ReasoningContext {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
}

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: MessageContent::Text(text.into()),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Text(text.into()),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::Text(text.into()),
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error,
            }]),
        }
    }
}

/// Tool definition for LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// LLM response.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Memory entry for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub source: String,
    pub embedding: Option<Vec<f32>>,
}

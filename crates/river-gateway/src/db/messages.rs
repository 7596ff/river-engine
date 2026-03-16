//! Message CRUD operations

use river_core::Snowflake;
use serde::{Deserialize, Serialize};

/// Message role types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A stored message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Snowflake,
    pub session_id: String,
    pub role: MessageRole,
    pub content: Option<String>,
    pub tool_calls: Option<String>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
    pub created_at: i64,
    pub metadata: Option<String>,
}

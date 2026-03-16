//! Message CRUD operations

/// Message role types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A stored message
#[derive(Debug, Clone)]
pub struct Message {
    pub id: Vec<u8>,
    pub session_id: String,
    pub role: MessageRole,
    pub content: Option<String>,
    pub tool_calls: Option<String>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
    pub created_at: i64,
    pub metadata: Option<String>,
}

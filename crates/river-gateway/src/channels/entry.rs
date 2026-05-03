//! Channel log entry types
//!
//! Each line in a channel JSONL log is one of these entries.

use serde::{Deserialize, Serialize};

/// A single entry in a channel log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChannelEntry {
    Message(MessageEntry),
    Cursor(CursorEntry),
}

/// A message from either the agent or another speaker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEntry {
    /// Snowflake ID — unique, sortable, encodes timestamp
    pub id: String,
    /// "agent" or "other"
    pub role: String,
    /// Display name of the speaker (for role: "other")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Adapter-specific unique ID of the speaker (for role: "other")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    /// The message text
    pub content: String,
    /// Which adapter the message came through
    pub adapter: String,
    /// Adapter-specific message ID (for replies, edits, deletes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
}

/// A cursor entry — agent read up to this point without speaking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorEntry {
    /// Snowflake ID
    pub id: String,
    /// Always "agent"
    pub role: String,
    /// Always true
    pub cursor: bool,
}

impl MessageEntry {
    /// Create an incoming message entry (role: "other")
    pub fn incoming(
        id: String,
        author: String,
        author_id: String,
        content: String,
        adapter: String,
        msg_id: Option<String>,
    ) -> Self {
        Self {
            id,
            role: "other".to_string(),
            author: Some(author),
            author_id: Some(author_id),
            content,
            adapter,
            msg_id,
        }
    }

    /// Create an outbound agent message entry (role: "agent")
    pub fn agent(
        id: String,
        content: String,
        adapter: String,
        msg_id: Option<String>,
    ) -> Self {
        Self {
            id,
            role: "agent".to_string(),
            author: None,
            author_id: None,
            content,
            adapter,
            msg_id,
        }
    }

    /// Returns true if this is an agent message
    pub fn is_agent(&self) -> bool {
        self.role == "agent"
    }
}

impl CursorEntry {
    pub fn new(id: String) -> Self {
        Self {
            id,
            role: "agent".to_string(),
            cursor: true,
        }
    }
}

impl ChannelEntry {
    /// Returns true if this entry is from the agent (message or cursor)
    pub fn is_agent(&self) -> bool {
        match self {
            ChannelEntry::Message(m) => m.is_agent(),
            ChannelEntry::Cursor(_) => true,
        }
    }

    /// Get the snowflake ID string
    pub fn id(&self) -> &str {
        match self {
            ChannelEntry::Message(m) => &m.id,
            ChannelEntry::Cursor(c) => &c.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message_serialization() {
        let entry = MessageEntry::incoming(
            "ABC123".to_string(),
            "cassie".to_string(),
            "12345".to_string(),
            "hello".to_string(),
            "discord".to_string(),
            Some("msg_001".to_string()),
        );
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"role\":\"other\""));
        assert!(json.contains("\"author\":\"cassie\""));
        assert!(json.contains("\"msg_id\":\"msg_001\""));
    }

    #[test]
    fn test_agent_message_serialization() {
        let entry = MessageEntry::agent(
            "ABC124".to_string(),
            "good morning".to_string(),
            "discord".to_string(),
            Some("msg_002".to_string()),
        );
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"role\":\"agent\""));
        assert!(!json.contains("\"author\""));
        assert!(!json.contains("\"author_id\""));
    }

    #[test]
    fn test_cursor_serialization() {
        let entry = CursorEntry::new("ABC125".to_string());
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"cursor\":true"));
        assert!(json.contains("\"role\":\"agent\""));
        assert!(!json.contains("\"content\""));
    }

    #[test]
    fn test_channel_entry_is_agent() {
        let msg = ChannelEntry::Message(MessageEntry::agent(
            "1".to_string(), "hi".to_string(), "discord".to_string(), None,
        ));
        assert!(msg.is_agent());

        let other = ChannelEntry::Message(MessageEntry::incoming(
            "2".to_string(), "user".to_string(), "u1".to_string(),
            "hello".to_string(), "discord".to_string(), None,
        ));
        assert!(!other.is_agent());

        let cursor = ChannelEntry::Cursor(CursorEntry::new("3".to_string()));
        assert!(cursor.is_agent());
    }

    #[test]
    fn test_roundtrip_message() {
        let entry = MessageEntry::incoming(
            "ABC123".to_string(),
            "cassie".to_string(),
            "12345".to_string(),
            "hello world".to_string(),
            "discord".to_string(),
            Some("msg_001".to_string()),
        );
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: MessageEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "ABC123");
        assert_eq!(parsed.role, "other");
        assert_eq!(parsed.author.unwrap(), "cassie");
        assert_eq!(parsed.content, "hello world");
    }

    #[test]
    fn test_roundtrip_cursor() {
        let entry = CursorEntry::new("ABC125".to_string());
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: CursorEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "ABC125");
        assert!(parsed.cursor);
    }
}

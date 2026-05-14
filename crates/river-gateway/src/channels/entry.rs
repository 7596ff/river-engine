//! Channel log entry types
//!
//! Each line in a channel JSONL log is one of these entries.

use river_core::Snowflake;
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
    pub id: Snowflake,
    /// "agent", "other", "user", "bystander", or "system"
    pub role: String,
    /// Display name of the speaker (for role: "other"/"user")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Adapter-specific unique ID of the speaker (for role: "other"/"user")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    /// The message text
    pub content: String,
    /// Which adapter the message came through
    pub adapter: String,
    /// Adapter-specific message ID (for replies, edits, deletes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    /// Source adapter (for user messages routed through home channel)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_adapter: Option<String>,
    /// Source channel ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_channel_id: Option<String>,
    /// Source channel name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_channel_name: Option<String>,
}

/// A tool call or tool result entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEntry {
    /// Snowflake ID
    pub id: Snowflake,
    /// "tool_call" or "tool_result"
    pub kind: String,
    /// Tool name
    pub tool_name: String,
    /// Tool call arguments (JSON value)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
    /// Tool result content, or file path if large
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// File path if result was persisted to disk
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_file: Option<String>,
    /// Model's tool call ID for threading
    pub tool_call_id: String,
}

/// A heartbeat entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatEntry {
    /// Snowflake ID
    pub id: Snowflake,
    /// Always "heartbeat"
    pub kind: String,
    /// ISO timestamp
    pub timestamp: String,
}

/// A cursor entry — agent read up to this point without speaking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorEntry {
    /// Snowflake ID
    pub id: Snowflake,
    /// Always "agent"
    pub role: String,
    /// Always true
    pub cursor: bool,
}

/// Entry in a home channel log — uses tagged serde for unambiguous deserialization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HomeChannelEntry {
    #[serde(rename = "message")]
    Message(MessageEntry),
    #[serde(rename = "cursor")]
    Cursor(CursorEntry),
    #[serde(rename = "tool")]
    Tool(ToolEntry),
    #[serde(rename = "heartbeat")]
    Heartbeat(HeartbeatEntry),
}

impl HomeChannelEntry {
    pub fn id(&self) -> Snowflake {
        match self {
            HomeChannelEntry::Message(m) => m.id,
            HomeChannelEntry::Cursor(c) => c.id,
            HomeChannelEntry::Tool(t) => t.id,
            HomeChannelEntry::Heartbeat(h) => h.id,
        }
    }
}

impl MessageEntry {
    /// Create an incoming message entry (role: "other")
    pub fn incoming(
        id: Snowflake,
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
            source_adapter: None,
            source_channel_id: None,
            source_channel_name: None,
        }
    }

    /// Create an outbound agent message entry (role: "agent")
    pub fn agent(
        id: Snowflake,
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
            source_adapter: None,
            source_channel_id: None,
            source_channel_name: None,
        }
    }

    /// Create a user message with source tracking (for home channel)
    pub fn user_home(
        id: Snowflake,
        author: String,
        author_id: String,
        content: String,
        source_adapter: String,
        source_channel_id: String,
        source_channel_name: Option<String>,
        msg_id: Option<String>,
    ) -> Self {
        Self {
            id,
            role: "user".to_string(),
            author: Some(author),
            author_id: Some(author_id),
            content,
            adapter: "home".to_string(),
            msg_id,
            source_adapter: Some(source_adapter),
            source_channel_id: Some(source_channel_id),
            source_channel_name,
        }
    }

    /// Create a bystander message (anonymous)
    pub fn bystander(id: Snowflake, content: String) -> Self {
        Self {
            id,
            role: "bystander".to_string(),
            author: None,
            author_id: None,
            content,
            adapter: "home".to_string(),
            msg_id: None,
            source_adapter: None,
            source_channel_id: None,
            source_channel_name: None,
        }
    }

    /// Create a system message
    pub fn system_msg(id: Snowflake, content: String) -> Self {
        Self {
            id,
            role: "system".to_string(),
            author: None,
            author_id: None,
            content,
            adapter: "home".to_string(),
            msg_id: None,
            source_adapter: None,
            source_channel_id: None,
            source_channel_name: None,
        }
    }

    /// Returns true if this is an agent message
    pub fn is_agent(&self) -> bool {
        self.role == "agent"
    }
}

impl ToolEntry {
    pub fn call(id: Snowflake, tool_name: String, arguments: serde_json::Value, tool_call_id: String) -> Self {
        Self {
            id, kind: "tool_call".to_string(), tool_name,
            arguments: Some(arguments), result: None, result_file: None, tool_call_id,
        }
    }

    pub fn result(id: Snowflake, tool_name: String, content: String, tool_call_id: String) -> Self {
        Self {
            id, kind: "tool_result".to_string(), tool_name,
            arguments: None, result: Some(content), result_file: None, tool_call_id,
        }
    }

    pub fn result_file(id: Snowflake, tool_name: String, file_path: String, tool_call_id: String) -> Self {
        Self {
            id, kind: "tool_result".to_string(), tool_name,
            arguments: None, result: None, result_file: Some(file_path), tool_call_id,
        }
    }
}

impl HeartbeatEntry {
    pub fn new(id: Snowflake, timestamp: String) -> Self {
        Self { id, kind: "heartbeat".to_string(), timestamp }
    }
}

impl CursorEntry {
    pub fn new(id: Snowflake) -> Self {
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
    pub fn id(&self) -> Snowflake {
        match self {
            ChannelEntry::Message(m) => m.id,
            ChannelEntry::Cursor(c) => c.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeType};

    fn test_snowflake() -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(0, birth, SnowflakeType::Message, 0)
    }

    fn test_snowflake_seq(seq: u32) -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(seq as u64 * 1_000_000, birth, SnowflakeType::Message, seq)
    }

    #[test]
    fn test_incoming_message_serialization() {
        let entry = MessageEntry::incoming(
            test_snowflake(),
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
        // source fields should be absent (None → skip)
        assert!(!json.contains("source_adapter"));
    }

    #[test]
    fn test_agent_message_serialization() {
        let entry = MessageEntry::agent(
            test_snowflake_seq(1),
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
        let entry = CursorEntry::new(test_snowflake_seq(2));
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"cursor\":true"));
        assert!(json.contains("\"role\":\"agent\""));
        assert!(!json.contains("\"content\""));
    }

    #[test]
    fn test_channel_entry_is_agent() {
        let msg = ChannelEntry::Message(MessageEntry::agent(
            test_snowflake_seq(1), "hi".to_string(), "discord".to_string(), None,
        ));
        assert!(msg.is_agent());

        let other = ChannelEntry::Message(MessageEntry::incoming(
            test_snowflake_seq(2), "user".to_string(), "u1".to_string(),
            "hello".to_string(), "discord".to_string(), None,
        ));
        assert!(!other.is_agent());

        let cursor = ChannelEntry::Cursor(CursorEntry::new(test_snowflake_seq(3)));
        assert!(cursor.is_agent());
    }

    #[test]
    fn test_roundtrip_message() {
        let entry = MessageEntry::incoming(
            test_snowflake(),
            "cassie".to_string(),
            "12345".to_string(),
            "hello world".to_string(),
            "discord".to_string(),
            Some("msg_001".to_string()),
        );
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: MessageEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, test_snowflake());
        assert_eq!(parsed.role, "other");
        assert_eq!(parsed.author.unwrap(), "cassie");
        assert_eq!(parsed.content, "hello world");
    }

    #[test]
    fn test_roundtrip_cursor() {
        let entry = CursorEntry::new(test_snowflake_seq(2));
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: CursorEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, test_snowflake_seq(2));
        assert!(parsed.cursor);
    }

    // ===== Home channel entry tests =====

    #[test]
    fn test_home_channel_message_tagged_roundtrip() {
        let msg = MessageEntry::user_home(
            test_snowflake_seq(1), "cassie".into(), "u1".into(), "hello".into(),
            "discord".into(), "general".into(), Some("general".into()), None,
        );
        let entry = HomeChannelEntry::Message(msg);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("\"source_adapter\":\"discord\""));
        assert!(json.contains("\"source_channel_id\":\"general\""));

        let parsed: HomeChannelEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id(), test_snowflake_seq(1));
        if let HomeChannelEntry::Message(m) = parsed {
            assert_eq!(m.role, "user");
            assert_eq!(m.source_adapter.unwrap(), "discord");
        } else {
            panic!("Expected Message variant");
        }
    }

    #[test]
    fn test_home_channel_tool_call_tagged_roundtrip() {
        let tool = ToolEntry::call(
            test_snowflake_seq(2), "read_file".into(),
            serde_json::json!({"path": "/tmp/test.txt"}),
            "tc_001".into(),
        );
        let entry = HomeChannelEntry::Tool(tool);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"type\":\"tool\""));
        assert!(json.contains("\"kind\":\"tool_call\""));
        assert!(json.contains("\"tool_name\":\"read_file\""));

        let parsed: HomeChannelEntry = serde_json::from_str(&json).unwrap();
        if let HomeChannelEntry::Tool(t) = parsed {
            assert_eq!(t.kind, "tool_call");
            assert_eq!(t.tool_name, "read_file");
            assert_eq!(t.tool_call_id, "tc_001");
            assert!(t.arguments.is_some());
        } else {
            panic!("Expected Tool variant");
        }
    }

    #[test]
    fn test_home_channel_tool_result_tagged_roundtrip() {
        let tool = ToolEntry::result(
            test_snowflake_seq(3), "read_file".into(),
            "file contents here".into(), "tc_001".into(),
        );
        let entry = HomeChannelEntry::Tool(tool);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"kind\":\"tool_result\""));

        let parsed: HomeChannelEntry = serde_json::from_str(&json).unwrap();
        if let HomeChannelEntry::Tool(t) = parsed {
            assert_eq!(t.kind, "tool_result");
            assert_eq!(t.result.unwrap(), "file contents here");
            assert!(t.arguments.is_none());
        } else {
            panic!("Expected Tool variant");
        }
    }

    #[test]
    fn test_home_channel_tool_result_file_roundtrip() {
        let tool = ToolEntry::result_file(
            test_snowflake_seq(4), "bash".into(),
            "/tmp/results/004.txt".into(), "tc_002".into(),
        );
        let entry = HomeChannelEntry::Tool(tool);
        let json = serde_json::to_string(&entry).unwrap();

        let parsed: HomeChannelEntry = serde_json::from_str(&json).unwrap();
        if let HomeChannelEntry::Tool(t) = parsed {
            assert!(t.result.is_none());
            assert_eq!(t.result_file.unwrap(), "/tmp/results/004.txt");
        } else {
            panic!("Expected Tool variant");
        }
    }

    #[test]
    fn test_home_channel_heartbeat_tagged_roundtrip() {
        let hb = HeartbeatEntry::new(test_snowflake_seq(5), "2026-05-12T12:00:00Z".into());
        let entry = HomeChannelEntry::Heartbeat(hb);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"type\":\"heartbeat\""));
        assert!(json.contains("\"kind\":\"heartbeat\""));

        let parsed: HomeChannelEntry = serde_json::from_str(&json).unwrap();
        if let HomeChannelEntry::Heartbeat(h) = parsed {
            assert_eq!(h.timestamp, "2026-05-12T12:00:00Z");
        } else {
            panic!("Expected Heartbeat variant");
        }
    }

    #[test]
    fn test_home_channel_cursor_tagged_roundtrip() {
        let cursor = CursorEntry::new(test_snowflake_seq(6));
        let entry = HomeChannelEntry::Cursor(cursor);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"type\":\"cursor\""));

        let parsed: HomeChannelEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id(), test_snowflake_seq(6));
    }

    #[test]
    fn test_home_channel_bystander_roundtrip() {
        let msg = MessageEntry::bystander(test_snowflake_seq(7), "interesting work".into());
        let entry = HomeChannelEntry::Message(msg);
        let json = serde_json::to_string(&entry).unwrap();

        let parsed: HomeChannelEntry = serde_json::from_str(&json).unwrap();
        if let HomeChannelEntry::Message(m) = parsed {
            assert_eq!(m.role, "bystander");
            assert!(m.author.is_none());
            assert!(m.source_adapter.is_none());
        } else {
            panic!("Expected Message variant");
        }
    }

    #[test]
    fn test_home_channel_system_msg_roundtrip() {
        let msg = MessageEntry::system_msg(test_snowflake_seq(8), "context pressure warning".into());
        let entry = HomeChannelEntry::Message(msg);
        let json = serde_json::to_string(&entry).unwrap();

        let parsed: HomeChannelEntry = serde_json::from_str(&json).unwrap();
        if let HomeChannelEntry::Message(m) = parsed {
            assert_eq!(m.role, "system");
            assert_eq!(m.adapter, "home");
        } else {
            panic!("Expected Message variant");
        }
    }

    #[test]
    fn test_home_channel_id_accessor() {
        let msg_entry = HomeChannelEntry::Message(
            MessageEntry::agent(test_snowflake_seq(10), "hi".into(), "home".into(), None),
        );
        assert_eq!(msg_entry.id(), test_snowflake_seq(10));

        let tool_entry = HomeChannelEntry::Tool(
            ToolEntry::call(test_snowflake_seq(11), "bash".into(), serde_json::json!({}), "tc1".into()),
        );
        assert_eq!(tool_entry.id(), test_snowflake_seq(11));

        let hb_entry = HomeChannelEntry::Heartbeat(
            HeartbeatEntry::new(test_snowflake_seq(12), "2026-01-01T00:00:00Z".into()),
        );
        assert_eq!(hb_entry.id(), test_snowflake_seq(12));
    }
}

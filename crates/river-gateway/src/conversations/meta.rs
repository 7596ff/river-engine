//! Conversation metadata (frontmatter)

use serde::{Deserialize, Serialize};

/// Routing metadata stored in conversation file frontmatter
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub adapter: String,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_deserialize_full() {
        let yaml = r#"
adapter: discord
channel_id: "789012345678901234"
channel_name: general
guild_id: "123456789012345678"
guild_name: myserver
"#;
        let meta: ConversationMeta = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.adapter, "discord");
        assert_eq!(meta.channel_id, "789012345678901234");
        assert_eq!(meta.channel_name, Some("general".to_string()));
        assert_eq!(meta.guild_id, Some("123456789012345678".to_string()));
        assert_eq!(meta.guild_name, Some("myserver".to_string()));
        assert_eq!(meta.thread_id, None);
    }

    #[test]
    fn test_meta_deserialize_minimal() {
        let yaml = r#"
adapter: slack
channel_id: C12345
"#;
        let meta: ConversationMeta = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.adapter, "slack");
        assert_eq!(meta.channel_id, "C12345");
        assert_eq!(meta.channel_name, None);
        assert_eq!(meta.guild_id, None);
    }

    #[test]
    fn test_meta_serialize_roundtrip() {
        let meta = ConversationMeta {
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: None,
            guild_name: None,
            thread_id: None,
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let parsed: ConversationMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(meta, parsed);
    }
}

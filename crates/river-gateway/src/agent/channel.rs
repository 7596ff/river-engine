//! Channel context for tracking the agent's current location

use crate::conversations::ConversationMeta;
use std::path::PathBuf;

/// Cached routing context for the current channel
#[derive(Debug, Clone)]
pub struct ChannelContext {
    /// Path to conversation file (relative to workspace)
    pub path: PathBuf,
    /// Adapter name (for registry lookup)
    pub adapter: String,
    /// Platform channel ID (for outbound messages)
    pub channel_id: String,
    /// Human-readable channel name (for logging/display)
    pub channel_name: Option<String>,
    /// Guild/server ID if applicable
    pub guild_id: Option<String>,
}

impl ChannelContext {
    /// Create from conversation path and metadata
    pub fn from_conversation(path: PathBuf, meta: &ConversationMeta) -> Self {
        Self {
            path,
            adapter: meta.adapter.clone(),
            channel_id: meta.channel_id.clone(),
            channel_name: meta.channel_name.clone(),
            guild_id: meta.guild_id.clone(),
        }
    }

    /// Get display name for logging (channel_name or channel_id)
    pub fn display_name(&self) -> &str {
        self.channel_name.as_deref().unwrap_or(&self.channel_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_context_from_conversation() {
        let meta = ConversationMeta {
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: Some("123456".to_string()),
            guild_name: Some("myserver".to_string()),
            thread_id: None,
        };

        let ctx = ChannelContext::from_conversation(
            PathBuf::from("conversations/discord/myserver/general.txt"),
            &meta,
        );

        assert_eq!(ctx.adapter, "discord");
        assert_eq!(ctx.channel_id, "789012");
        assert_eq!(ctx.channel_name, Some("general".to_string()));
        assert_eq!(ctx.guild_id, Some("123456".to_string()));
    }

    #[test]
    fn test_display_name_with_name() {
        let ctx = ChannelContext {
            path: PathBuf::from("test.txt"),
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: Some("general".to_string()),
            guild_id: None,
        };
        assert_eq!(ctx.display_name(), "general");
    }

    #[test]
    fn test_display_name_without_name() {
        let ctx = ChannelContext {
            path: PathBuf::from("test.txt"),
            adapter: "discord".to_string(),
            channel_id: "789012".to_string(),
            channel_name: None,
            guild_id: None,
        };
        assert_eq!(ctx.display_name(), "789012");
    }
}

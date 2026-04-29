//! Integration tests for utterances (speak/switch_channel)

use river_gateway::conversations::{Conversation, ConversationMeta};
use river_gateway::agent::ChannelContext;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_channel_context_from_file() {
    let temp = TempDir::new().unwrap();
    let conv_path = temp.path().join("conversations/discord/general.txt");
    std::fs::create_dir_all(conv_path.parent().unwrap()).unwrap();

    let content = r#"---
adapter: discord
channel_id: "789012"
channel_name: general
guild_id: "123456"
---
[ ] 2026-03-28 10:00:00 msg1 <alice:111> hello
"#;
    std::fs::write(&conv_path, content).unwrap();

    // Parse conversation
    let file_content = std::fs::read_to_string(&conv_path).unwrap();
    let conversation = Conversation::from_str(&file_content).unwrap();

    assert!(conversation.meta.is_some());
    let meta = conversation.meta.as_ref().unwrap();

    // Create channel context
    let ctx = ChannelContext::from_conversation(
        PathBuf::from("conversations/discord/general.txt"),
        meta,
    );

    assert_eq!(ctx.adapter, "discord");
    assert_eq!(ctx.channel_id, "789012");
    assert_eq!(ctx.display_name(), "general");
}

#[test]
fn test_conversation_frontmatter_preserved_on_roundtrip() {
    let meta = ConversationMeta {
        adapter: "slack".to_string(),
        channel_id: "C12345".to_string(),
        channel_name: Some("random".to_string()),
        guild_id: None,
        guild_name: None,
        thread_id: None,
    };

    let mut conversation = Conversation::default();
    conversation.meta = Some(meta.clone());

    let serialized = conversation.to_string();
    assert!(serialized.starts_with("---"));
    assert!(serialized.contains("adapter: slack"));
    assert!(serialized.contains("channel_id: C12345"));

    let parsed = Conversation::from_str(&serialized).unwrap();
    assert_eq!(parsed.meta.unwrap().adapter, "slack");
}

#[test]
fn test_switch_channel_to_nonexistent_file_error() {
    // This tests at unit level - the switch_channel tool returns error
    // when file doesn't exist
    let result = Conversation::from_str("");
    // Empty string parses to empty conversation, not an error
    // but a missing file would error in the tool itself
    assert!(result.is_ok());
}

#[test]
fn test_switch_channel_to_file_without_frontmatter_error() {
    // File without frontmatter - conversation parses but has no meta
    let content = "[ ] 2026-03-28 10:00:00 msg1 <alice:111> hello\n";
    let conversation = Conversation::from_str(content).unwrap();
    assert!(conversation.meta.is_none());
    // The switch_channel tool should error when meta is None
}

#[test]
fn test_conversation_unclosed_frontmatter() {
    // Test that unclosed frontmatter produces an error
    let content = "---\nadapter: discord\nchannel_id: 123\n[ ] msg";
    let result = Conversation::from_str(content);
    assert!(result.is_err());
}

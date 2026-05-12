//! Integration tests for conversation parsing

use river_gateway::conversations::{Conversation, ConversationMeta};

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
    let result = Conversation::from_str("");
    assert!(result.is_ok());
}

#[test]
fn test_switch_channel_to_file_without_frontmatter_error() {
    let content = "[ ] 2026-03-28 10:00:00 msg1 <alice:111> hello\n";
    let conversation = Conversation::from_str(content).unwrap();
    assert!(conversation.meta.is_none());
}

#[test]
fn test_conversation_unclosed_frontmatter() {
    let content = "---\nadapter: discord\nchannel_id: 123\n[ ] msg";
    let result = Conversation::from_str(content);
    assert!(result.is_err());
}

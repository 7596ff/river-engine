//! Home channel context builder — derives model context from home channel + moves

use crate::channels::entry::{HomeChannelEntry, MessageEntry};
use crate::channels::log::ChannelLog;
use crate::model::{ChatMessage, ToolCallRequest, FunctionCall};
use std::path::Path;

pub struct HomeContextConfig {
    pub max_tail_entries: usize,
}

impl Default for HomeContextConfig {
    fn default() -> Self {
        Self { max_tail_entries: 200 }
    }
}

/// Build model context from home channel + moves
pub async fn build_context(
    home_channel_path: &Path,
    moves: &[String],
    config: &HomeContextConfig,
) -> std::io::Result<Vec<ChatMessage>> {
    let log = ChannelLog::from_path(home_channel_path.to_path_buf());
    let all_entries = log.read_all_home().await?;

    let mut messages = Vec::new();

    // Moves as compressed history
    for mov in moves {
        messages.push(ChatMessage::system(mov.clone()));
    }

    // Tail entries
    let tail_start = all_entries.len().saturating_sub(config.max_tail_entries);
    let tail = &all_entries[tail_start..];

    // Process tail — need to group tool calls with their assistant message
    let mut i = 0;
    while i < tail.len() {
        match &tail[i] {
            HomeChannelEntry::Message(m) => {
                match m.role.as_str() {
                    "agent" => messages.push(ChatMessage::assistant(Some(m.content.clone()), None)),
                    "user" => {
                        let tagged = format_user_tag(m);
                        messages.push(ChatMessage::user(tagged));
                    }
                    "bystander" => {
                        let tagged = format!("[bystander] {}", m.content);
                        messages.push(ChatMessage::user(tagged));
                    }
                    "system" => messages.push(ChatMessage::system(m.content.clone())),
                    _ => {}
                }
            }
            HomeChannelEntry::Tool(t) => {
                match t.kind.as_str() {
                    "tool_call" => {
                        // Collect consecutive tool calls into one assistant message
                        let mut tool_calls = Vec::new();
                        while i < tail.len() {
                            if let HomeChannelEntry::Tool(tc) = &tail[i] {
                                if tc.kind == "tool_call" {
                                    tool_calls.push(ToolCallRequest {
                                        id: tc.tool_call_id.clone(),
                                        r#type: "function".to_string(),
                                        function: FunctionCall {
                                            name: tc.tool_name.clone(),
                                            arguments: tc.arguments
                                                .as_ref()
                                                .map(|a| serde_json::to_string(a).unwrap_or_default())
                                                .unwrap_or_default(),
                                        },
                                    });
                                    i += 1;
                                    continue;
                                }
                            }
                            break;
                        }
                        messages.push(ChatMessage::assistant(None, Some(tool_calls)));
                        continue; // don't increment i again
                    }
                    "tool_result" => {
                        let content = if let Some(ref result) = t.result {
                            result.clone()
                        } else if let Some(ref file) = t.result_file {
                            tokio::fs::read_to_string(file).await
                                .unwrap_or_else(|_| format!("[tool result file missing: {}]", file))
                        } else {
                            "[empty tool result]".to_string()
                        };
                        messages.push(ChatMessage::tool(&t.tool_call_id, &content));
                    }
                    _ => {}
                }
            }
            HomeChannelEntry::Heartbeat(_) => {
                messages.push(ChatMessage::system("[heartbeat]".to_string()));
            }
            HomeChannelEntry::Cursor(_) => {}
        }
        i += 1;
    }

    Ok(messages)
}

/// Format a user message with source adapter/channel tag
fn format_user_tag(m: &MessageEntry) -> String {
    let author = m.author.as_deref().unwrap_or("unknown");
    match (&m.source_adapter, &m.source_channel_id, &m.source_channel_name) {
        (Some(adapter), Some(ch_id), Some(ch_name)) => {
            format!("[user:{}:{}/{}] {}: {}", adapter, ch_id, ch_name, author, m.content)
        }
        (Some(adapter), Some(ch_id), None) => {
            format!("[user:{}:{}] {}: {}", adapter, ch_id, author, m.content)
        }
        _ => format!("{}: {}", author, m.content),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::entry::*;
    use crate::channels::log::ChannelLog;
    use tempfile::TempDir;

    async fn write_entries(dir: &TempDir, entries: &[HomeChannelEntry]) -> std::path::PathBuf {
        let path = dir.path().join("test-home.jsonl");
        let log = ChannelLog::from_path(path.clone());
        for entry in entries {
            log.append_entry(entry).await.unwrap();
        }
        path
    }

    #[tokio::test]
    async fn test_build_context_messages_only() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::user_home(
                "001".into(), "cassie".into(), "u1".into(), "hello".into(),
                "discord".into(), "general".into(), Some("general".into()), None,
            )),
            HomeChannelEntry::Message(MessageEntry::agent(
                "002".into(), "hi there!".into(), "home".into(), None,
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default()).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0].content.as_ref().unwrap().contains("[user:discord:general/general]"));
        assert!(msgs[0].content.as_ref().unwrap().contains("cassie: hello"));
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content.as_ref().unwrap(), "hi there!");
    }

    #[tokio::test]
    async fn test_build_context_tool_calls_grouped() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            // Two consecutive tool calls should become one assistant message
            HomeChannelEntry::Tool(ToolEntry::call(
                "001".into(), "read_file".into(),
                serde_json::json!({"path": "/tmp/a.txt"}), "tc1".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::call(
                "002".into(), "read_file".into(),
                serde_json::json!({"path": "/tmp/b.txt"}), "tc2".into(),
            )),
            // Then two tool results
            HomeChannelEntry::Tool(ToolEntry::result(
                "003".into(), "read_file".into(), "content a".into(), "tc1".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::result(
                "004".into(), "read_file".into(), "content b".into(), "tc2".into(),
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default()).await.unwrap();
        // 1 assistant (with 2 tool calls) + 2 tool results = 3 messages
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "assistant");
        assert!(msgs[0].content.is_none());
        let tc = msgs[0].tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 2);
        assert_eq!(tc[0].function.name, "read_file");
        assert_eq!(tc[1].function.name, "read_file");
        assert_eq!(tc[0].id, "tc1");
        assert_eq!(tc[1].id, "tc2");

        assert_eq!(msgs[1].role, "tool");
        assert_eq!(msgs[1].tool_call_id.as_ref().unwrap(), "tc1");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id.as_ref().unwrap(), "tc2");
    }

    #[tokio::test]
    async fn test_build_context_with_moves() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::agent(
                "001".into(), "working on it".into(), "home".into(), None,
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let moves = vec!["Turn 1-5: Agent set up the project.".to_string()];
        let msgs = build_context(&path, &moves, &HomeContextConfig::default()).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.as_ref().unwrap().contains("set up the project"));
        assert_eq!(msgs[1].role, "assistant");
    }

    #[tokio::test]
    async fn test_build_context_bystander() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::bystander(
                "001".into(), "nice work".into(),
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default()).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0].content.as_ref().unwrap().contains("[bystander]"));
    }

    #[tokio::test]
    async fn test_build_context_heartbeat() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Heartbeat(HeartbeatEntry::new(
                "001".into(), "2026-05-12T12:00:00Z".into(),
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default()).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.as_ref().unwrap().contains("[heartbeat]"));
    }

    #[tokio::test]
    async fn test_build_context_cursor_skipped() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Cursor(CursorEntry::new("001".into())),
            HomeChannelEntry::Message(MessageEntry::agent(
                "002".into(), "hi".into(), "home".into(), None,
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default()).await.unwrap();
        assert_eq!(msgs.len(), 1); // cursor skipped
    }

    #[tokio::test]
    async fn test_build_context_tail_limit() {
        let dir = TempDir::new().unwrap();
        let mut entries = Vec::new();
        for i in 0..10 {
            entries.push(HomeChannelEntry::Message(MessageEntry::agent(
                format!("{:03}", i), format!("msg {}", i), "home".into(), None,
            )));
        }
        let path = write_entries(&dir, &entries).await;

        let config = HomeContextConfig { max_tail_entries: 3 };
        let msgs = build_context(&path, &[], &config).await.unwrap();
        assert_eq!(msgs.len(), 3);
        // Should be the last 3
        assert!(msgs[0].content.as_ref().unwrap().contains("msg 7"));
        assert!(msgs[1].content.as_ref().unwrap().contains("msg 8"));
        assert!(msgs[2].content.as_ref().unwrap().contains("msg 9"));
    }

    #[tokio::test]
    async fn test_build_context_empty_log() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.jsonl");
        let msgs = build_context(&path, &[], &HomeContextConfig::default()).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_format_user_tag_with_all_fields() {
        let m = MessageEntry::user_home(
            "001".into(), "cassie".into(), "u1".into(), "hello".into(),
            "discord".into(), "123456".into(), Some("general".into()), None,
        );
        let tag = format_user_tag(&m);
        assert_eq!(tag, "[user:discord:123456/general] cassie: hello");
    }

    #[tokio::test]
    async fn test_format_user_tag_without_channel_name() {
        let m = MessageEntry::user_home(
            "001".into(), "cassie".into(), "u1".into(), "hello".into(),
            "discord".into(), "123456".into(), None, None,
        );
        let tag = format_user_tag(&m);
        assert_eq!(tag, "[user:discord:123456] cassie: hello");
    }

    #[tokio::test]
    async fn test_format_user_tag_no_source() {
        let m = MessageEntry::incoming(
            "001".into(), "cassie".into(), "u1".into(), "hello".into(),
            "discord".into(), None,
        );
        let tag = format_user_tag(&m);
        assert_eq!(tag, "cassie: hello");
    }
}

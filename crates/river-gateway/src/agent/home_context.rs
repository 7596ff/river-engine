//! Home channel context builder — derives model context from home channel + moves

use crate::channels::entry::{HomeChannelEntry, MessageEntry};
use crate::channels::log::ChannelLog;
use crate::model::{ChatMessage, FunctionCall, ToolCallRequest};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct HomeContextConfig {
    pub max_tail_entries: usize,
}

impl Default for HomeContextConfig {
    fn default() -> Self {
        Self {
            max_tail_entries: 200,
        }
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

    // Process tail — merge agent content with tool calls, reorder interleaved messages
    let mut i = 0;
    while i < tail.len() {
        match &tail[i] {
            HomeChannelEntry::Message(m) => match m.role.as_str() {
                "agent" => {
                    let content = if m.content.is_empty() {
                        None
                    } else {
                        Some(m.content.clone())
                    };

                    // Look ahead for immediately following tool_call entries
                    let mut next = i + 1;
                    let mut tool_calls = Vec::new();
                    while next < tail.len() {
                        if let HomeChannelEntry::Tool(tc) = &tail[next] {
                            if tc.kind == "tool_call" {
                                tool_calls.push(ToolCallRequest {
                                    id: tc.tool_call_id.clone(),
                                    r#type: "function".to_string(),
                                    function: FunctionCall {
                                        name: tc.tool_name.clone(),
                                        arguments: tc
                                            .arguments
                                            .as_ref()
                                            .map(|a| serde_json::to_string(a).unwrap_or_default())
                                            .unwrap_or_default(),
                                    },
                                });
                                next += 1;
                                continue;
                            }
                        }
                        break;
                    }

                    if tool_calls.is_empty() {
                        // No tool calls — emit plain assistant message (skip if empty)
                        if content.is_some() {
                            messages.push(ChatMessage::assistant(content, None));
                        }
                    } else {
                        // Emit assistant message with tool calls
                        let expected_ids: HashSet<String> =
                            tool_calls.iter().map(|tc| tc.id.clone()).collect();
                        messages
                            .push(ChatMessage::assistant(content, Some(tool_calls)));

                        // Now collect tool results for these IDs, deferring any
                        // interleaved non-tool messages (e.g. Discord echo from send_message)
                        let mut deferred: Vec<ChatMessage> = Vec::new();
                        let mut found_ids: HashSet<String> = HashSet::new();
                        let mut scan = next;

                        while scan < tail.len() && found_ids.len() < expected_ids.len() {
                            match &tail[scan] {
                                HomeChannelEntry::Tool(t) if t.kind == "tool_result" => {
                                    if expected_ids.contains(&t.tool_call_id) {
                                        let result_content =
                                            if let Some(ref result) = t.result {
                                                result.clone()
                                            } else if let Some(ref file) = t.result_file {
                                                // Can't async read here in a sync scan,
                                                // use placeholder — will be filled below
                                                format!("[file: {}]", file)
                                            } else {
                                                "[empty tool result]".to_string()
                                            };
                                        messages.push(ChatMessage::tool(
                                            &t.tool_call_id,
                                            &result_content,
                                        ));
                                        found_ids.insert(t.tool_call_id.clone());
                                    } else {
                                        // Tool result for a different call — defer
                                        deferred.push(ChatMessage::tool(
                                            &t.tool_call_id,
                                            t.result.as_deref().unwrap_or("[empty]"),
                                        ));
                                    }
                                }
                                other => {
                                    // Non-tool entry interleaved — defer it
                                    if let Some(msg) = entry_to_chat_message(other) {
                                        deferred.push(msg);
                                    }
                                }
                            }
                            scan += 1;
                        }

                        // Emit deferred messages after tool results
                        messages.extend(deferred);

                        i = scan;
                        continue;
                    }
                }
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
            },
            HomeChannelEntry::Tool(t) => {
                match t.kind.as_str() {
                    "tool_call" => {
                        // Orphan tool calls (no preceding agent message)
                        let mut tool_calls = Vec::new();
                        while i < tail.len() {
                            if let HomeChannelEntry::Tool(tc) = &tail[i] {
                                if tc.kind == "tool_call" {
                                    tool_calls.push(ToolCallRequest {
                                        id: tc.tool_call_id.clone(),
                                        r#type: "function".to_string(),
                                        function: FunctionCall {
                                            name: tc.tool_name.clone(),
                                            arguments: tc
                                                .arguments
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

                        // Same reordering logic for orphan tool calls
                        let expected_ids: HashSet<String> =
                            tool_calls.iter().map(|tc| tc.id.clone()).collect();
                        messages.push(ChatMessage::assistant(None, Some(tool_calls)));

                        let mut deferred: Vec<ChatMessage> = Vec::new();
                        let mut found_ids: HashSet<String> = HashSet::new();
                        let mut scan = i;

                        while scan < tail.len() && found_ids.len() < expected_ids.len() {
                            match &tail[scan] {
                                HomeChannelEntry::Tool(t) if t.kind == "tool_result" => {
                                    if expected_ids.contains(&t.tool_call_id) {
                                        let result_content =
                                            if let Some(ref result) = t.result {
                                                result.clone()
                                            } else if let Some(ref file) = t.result_file {
                                                format!("[file: {}]", file)
                                            } else {
                                                "[empty tool result]".to_string()
                                            };
                                        messages.push(ChatMessage::tool(
                                            &t.tool_call_id,
                                            &result_content,
                                        ));
                                        found_ids.insert(t.tool_call_id.clone());
                                    } else {
                                        deferred.push(ChatMessage::tool(
                                            &t.tool_call_id,
                                            t.result.as_deref().unwrap_or("[empty]"),
                                        ));
                                    }
                                }
                                other => {
                                    if let Some(msg) = entry_to_chat_message(other) {
                                        deferred.push(msg);
                                    }
                                }
                            }
                            scan += 1;
                        }

                        messages.extend(deferred);
                        i = scan;
                        continue;
                    }
                    "tool_result" => {
                        // Orphan tool result — emit as-is
                        let content = if let Some(ref result) = t.result {
                            result.clone()
                        } else if let Some(ref file) = t.result_file {
                            format!("[file: {}]", file)
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

/// Convert a HomeChannelEntry to a ChatMessage (for deferred messages)
fn entry_to_chat_message(entry: &HomeChannelEntry) -> Option<ChatMessage> {
    match entry {
        HomeChannelEntry::Message(m) => match m.role.as_str() {
            "agent" => {
                if m.content.is_empty() {
                    None
                } else {
                    Some(ChatMessage::assistant(Some(m.content.clone()), None))
                }
            }
            "user" => Some(ChatMessage::user(format_user_tag(m))),
            "bystander" => Some(ChatMessage::user(format!("[bystander] {}", m.content))),
            "system" => Some(ChatMessage::system(m.content.clone())),
            _ => None,
        },
        HomeChannelEntry::Heartbeat(_) => Some(ChatMessage::system("[heartbeat]".to_string())),
        HomeChannelEntry::Cursor(_) => None,
        HomeChannelEntry::Tool(_) => None, // handled by caller
    }
}

/// Format a user message with source adapter/channel tag
fn format_user_tag(m: &MessageEntry) -> String {
    let author = m.author.as_deref().unwrap_or("unknown");
    match (
        &m.source_adapter,
        &m.source_channel_id,
        &m.source_channel_name,
    ) {
        (Some(adapter), Some(ch_id), Some(ch_name)) => {
            format!(
                "[user:{}:{}/{}] {}: {}",
                adapter, ch_id, ch_name, author, m.content
            )
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
    use river_core::{AgentBirth, Snowflake, SnowflakeType};

    fn test_snowflake() -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(0, birth, SnowflakeType::Message, 0)
    }

    fn test_snowflake_seq(seq: u32) -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(seq as u64 * 1_000_000, birth, SnowflakeType::Message, seq)
    }

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
                test_snowflake_seq(1),
                "cassie".into(),
                "u1".into(),
                "hello".into(),
                "discord".into(),
                "general".into(),
                Some("general".into()),
                None,
            )),
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(2),
                "hi there!".into(),
                "home".into(),
                None,
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0]
            .content
            .as_ref()
            .unwrap()
            .contains("[user:discord:general/general]"));
        assert!(msgs[0].content.as_ref().unwrap().contains("cassie: hello"));
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content.as_ref().unwrap(), "hi there!");
    }

    #[tokio::test]
    async fn test_build_context_tool_calls_grouped() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Tool(ToolEntry::call(
                test_snowflake_seq(1),
                "read_file".into(),
                serde_json::json!({"path": "/tmp/a.txt"}),
                "tc1".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::call(
                test_snowflake_seq(2),
                "read_file".into(),
                serde_json::json!({"path": "/tmp/b.txt"}),
                "tc2".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::result(
                test_snowflake_seq(3),
                "read_file".into(),
                "content a".into(),
                "tc1".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::result(
                test_snowflake_seq(4),
                "read_file".into(),
                "content b".into(),
                "tc2".into(),
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "assistant");
        assert!(msgs[0].content.is_none());
        let tc = msgs[0].tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 2);
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
        let entries = vec![HomeChannelEntry::Message(MessageEntry::agent(
            test_snowflake_seq(1),
            "working on it".into(),
            "home".into(),
            None,
        ))];
        let path = write_entries(&dir, &entries).await;

        let moves = vec!["Turn 1-5: Agent set up the project.".to_string()];
        let msgs = build_context(&path, &moves, &HomeContextConfig::default())
            .await
            .unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0]
            .content
            .as_ref()
            .unwrap()
            .contains("set up the project"));
        assert_eq!(msgs[1].role, "assistant");
    }

    #[tokio::test]
    async fn test_build_context_bystander() {
        let dir = TempDir::new().unwrap();
        let entries = vec![HomeChannelEntry::Message(MessageEntry::bystander(
            test_snowflake_seq(1),
            "nice work".into(),
        ))];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0].content.as_ref().unwrap().contains("[bystander]"));
    }

    #[tokio::test]
    async fn test_build_context_heartbeat() {
        let dir = TempDir::new().unwrap();
        let entries = vec![HomeChannelEntry::Heartbeat(HeartbeatEntry::new(
            test_snowflake_seq(1),
            "2026-05-12T12:00:00Z".into(),
        ))];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.as_ref().unwrap().contains("[heartbeat]"));
    }

    #[tokio::test]
    async fn test_build_context_cursor_skipped() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Cursor(CursorEntry::new(test_snowflake_seq(1))),
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(2),
                "hi".into(),
                "home".into(),
                None,
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();
        assert_eq!(msgs.len(), 1);
    }

    #[tokio::test]
    async fn test_build_context_tail_limit() {
        let dir = TempDir::new().unwrap();
        let mut entries = Vec::new();
        for i in 0..10 {
            entries.push(HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(i as u32),
                format!("msg {}", i),
                "home".into(),
                None,
            )));
        }
        let path = write_entries(&dir, &entries).await;

        let config = HomeContextConfig {
            max_tail_entries: 3,
        };
        let msgs = build_context(&path, &[], &config).await.unwrap();
        assert_eq!(msgs.len(), 3);
        assert!(msgs[0].content.as_ref().unwrap().contains("msg 7"));
        assert!(msgs[1].content.as_ref().unwrap().contains("msg 8"));
        assert!(msgs[2].content.as_ref().unwrap().contains("msg 9"));
    }

    #[tokio::test]
    async fn test_build_context_empty_log() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.jsonl");
        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_build_context_agent_content_merged_with_tool_calls() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(1),
                "Let me read those files.".into(),
                "home".into(),
                None,
            )),
            HomeChannelEntry::Tool(ToolEntry::call(
                test_snowflake_seq(2),
                "read_file".into(),
                serde_json::json!({"path": "/tmp/a.txt"}),
                "tc1".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::call(
                test_snowflake_seq(3),
                "read_file".into(),
                serde_json::json!({"path": "/tmp/b.txt"}),
                "tc2".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::result(
                test_snowflake_seq(4),
                "read_file".into(),
                "content a".into(),
                "tc1".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::result(
                test_snowflake_seq(5),
                "read_file".into(),
                "content b".into(),
                "tc2".into(),
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();

        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "assistant");
        assert_eq!(
            msgs[0].content.as_ref().unwrap(),
            "Let me read those files."
        );
        let tc = msgs[0].tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 2);
        assert_eq!(tc[0].id, "tc1");
        assert_eq!(tc[1].id, "tc2");
        assert_eq!(msgs[1].role, "tool");
        assert_eq!(msgs[1].tool_call_id.as_ref().unwrap(), "tc1");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id.as_ref().unwrap(), "tc2");
    }

    #[tokio::test]
    async fn test_build_context_agent_empty_content_with_tool_calls() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(1),
                "".into(),
                "home".into(),
                None,
            )),
            HomeChannelEntry::Tool(ToolEntry::call(
                test_snowflake_seq(2),
                "glob".into(),
                serde_json::json!({"pattern": "*.rs"}),
                "tc1".into(),
            )),
            HomeChannelEntry::Tool(ToolEntry::result(
                test_snowflake_seq(3),
                "glob".into(),
                "main.rs\nlib.rs".into(),
                "tc1".into(),
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();

        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "assistant");
        assert!(msgs[0].content.is_none());
        assert_eq!(msgs[0].tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(msgs[1].role, "tool");
    }

    #[tokio::test]
    async fn test_build_context_agent_without_following_tool_calls() {
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(1),
                "Just a normal response.".into(),
                "home".into(),
                None,
            )),
            HomeChannelEntry::Message(MessageEntry::bystander(
                test_snowflake_seq(2),
                "thanks".into(),
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();

        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "assistant");
        assert_eq!(
            msgs[0].content.as_ref().unwrap(),
            "Just a normal response."
        );
        assert!(msgs[0].tool_calls.is_none());
        assert_eq!(msgs[1].role, "user");
    }

    #[tokio::test]
    async fn test_build_context_interleaved_message_during_tool_execution() {
        // Simulates: send_message tool sends to Discord, Discord echoes back
        // as an incoming user message BEFORE the tool result is written
        let dir = TempDir::new().unwrap();
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(1),
                "Sending message now.".into(),
                "home".into(),
                None,
            )),
            HomeChannelEntry::Tool(ToolEntry::call(
                test_snowflake_seq(2),
                "send_message".into(),
                serde_json::json!({"adapter": "discord", "channel": "123", "content": "hello from viola"}),
                "tc1".into(),
            )),
            // Discord echo arrives before tool result
            HomeChannelEntry::Message(MessageEntry::user_home(
                test_snowflake_seq(3),
                "viola".into(),
                "bot".into(),
                "hello from viola".into(),
                "discord".into(),
                "123".into(),
                None,
                None,
            )),
            // Tool result arrives after
            HomeChannelEntry::Tool(ToolEntry::result(
                test_snowflake_seq(4),
                "send_message".into(),
                "Message sent successfully".into(),
                "tc1".into(),
            )),
            // Next user message
            HomeChannelEntry::Message(MessageEntry::bystander(
                test_snowflake_seq(5),
                "nice, it worked!".into(),
            )),
        ];
        let path = write_entries(&dir, &entries).await;

        let msgs = build_context(&path, &[], &HomeContextConfig::default())
            .await
            .unwrap();

        // Expected order:
        // 0: assistant("Sending message now.", tool_calls=[send_message])
        // 1: tool(result for tc1)
        // 2: user(discord echo — deferred)
        // 3: user(bystander — after the reordered block)
        assert_eq!(msgs.len(), 4);

        assert_eq!(msgs[0].role, "assistant");
        assert_eq!(msgs[0].content.as_ref().unwrap(), "Sending message now.");
        assert!(msgs[0].tool_calls.is_some());

        assert_eq!(msgs[1].role, "tool");
        assert_eq!(msgs[1].tool_call_id.as_ref().unwrap(), "tc1");

        assert_eq!(msgs[2].role, "user"); // deferred discord echo
        assert!(msgs[2].content.as_ref().unwrap().contains("hello from viola"));

        assert_eq!(msgs[3].role, "user"); // bystander
        assert!(msgs[3].content.as_ref().unwrap().contains("nice, it worked"));
    }

    #[tokio::test]
    async fn test_format_user_tag_with_all_fields() {
        let m = MessageEntry::user_home(
            test_snowflake_seq(1),
            "cassie".into(),
            "u1".into(),
            "hello".into(),
            "discord".into(),
            "123456".into(),
            Some("general".into()),
            None,
        );
        let tag = format_user_tag(&m);
        assert_eq!(tag, "[user:discord:123456/general] cassie: hello");
    }

    #[tokio::test]
    async fn test_format_user_tag_without_channel_name() {
        let m = MessageEntry::user_home(
            test_snowflake_seq(1),
            "cassie".into(),
            "u1".into(),
            "hello".into(),
            "discord".into(),
            "123456".into(),
            None,
            None,
        );
        let tag = format_user_tag(&m);
        assert_eq!(tag, "[user:discord:123456] cassie: hello");
    }

    #[tokio::test]
    async fn test_format_user_tag_no_source() {
        let m = MessageEntry::incoming(
            test_snowflake_seq(1),
            "cassie".into(),
            "u1".into(),
            "hello".into(),
            "discord".into(),
            None,
        );
        let tag = format_user_tag(&m);
        assert_eq!(tag, "cassie: hello");
    }
}

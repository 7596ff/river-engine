//! Entry formatting for spectator sweeps
//!
//! Converts HomeChannelEntry entries into a transcript string for the LLM.
//! Tiered detail: full text for messages, name-only for tools.
//! Heartbeats, cursors, and spectator observability messages are filtered out.

use crate::channels::entry::{HomeChannelEntry, MessageEntry};

/// Format a single entry into a transcript line.
/// Returns None for entries that should be filtered out.
pub fn format_entry(entry: &HomeChannelEntry) -> Option<String> {
    match entry {
        HomeChannelEntry::Message(m) => format_message(m),
        HomeChannelEntry::Tool(t) => {
            match t.kind.as_str() {
                "tool_call" => Some(format!("[{}] tool_call: {}", t.id, t.tool_name)),
                "tool_result" => {
                    if let Some(ref file_path) = t.result_file {
                        Some(format!("[{}] tool_result({}): [file: {}]", t.id, t.tool_name, file_path))
                    } else {
                        let byte_count = t.result.as_ref().map_or(0, |r| r.len());
                        Some(format!("[{}] tool_result({}): [{} bytes]", t.id, t.tool_name, byte_count))
                    }
                }
                _ => None,
            }
        }
        HomeChannelEntry::Heartbeat(_) => None,
        HomeChannelEntry::Cursor(_) => None,
    }
}

/// Format a message entry with source tags.
/// Returns None for spectator's own messages (feedback loop prevention).
fn format_message(m: &MessageEntry) -> Option<String> {
    // Filter spectator's own observability messages
    if m.role == "system" && m.content.starts_with("[spectator]") {
        return None;
    }

    Some(match m.role.as_str() {
        "user" => {
            let author = m.author.as_deref().unwrap_or("unknown");
            match (&m.source_adapter, &m.source_channel_id, &m.source_channel_name) {
                (Some(adapter), Some(ch_id), Some(ch_name)) => {
                    format!("[{}] user:{}:{}/{} {}: {}", m.id, adapter, ch_id, ch_name, author, m.content)
                }
                (Some(adapter), Some(ch_id), None) => {
                    format!("[{}] user:{}:{} {}: {}", m.id, adapter, ch_id, author, m.content)
                }
                _ => format!("[{}] user: {}: {}", m.id, author, m.content),
            }
        }
        "agent" => format!("[{}] agent: {}", m.id, m.content),
        "bystander" => format!("[{}] bystander: {}", m.id, m.content),
        "system" => format!("[{}] system: {}", m.id, m.content),
        other => format!("[{}] {}: {}", m.id, other, m.content),
    })
}

/// Estimate tokens for a string (same heuristic as the rest of the codebase)
fn estimate_tokens(s: &str) -> usize {
    if s.is_empty() { return 0; }
    (s.len() + 3) / 4
}

/// Format entries with a token budget.
///
/// Returns (transcript, last_index) where last_index is the index of the
/// last entry included in the transcript. Entries are included oldest-first
/// until the budget is reached. At least one entry is always included.
pub fn format_entries_budgeted(entries: &[HomeChannelEntry], token_budget: usize) -> (String, usize) {
    let mut lines = Vec::new();
    let mut tokens_used = 0;
    let mut last_idx = 0;

    for (i, entry) in entries.iter().enumerate() {
        let line = match format_entry(entry) {
            Some(l) => l,
            None => continue, // Filtered out
        };

        let line_tokens = estimate_tokens(&line);

        // Always include at least one entry
        if !lines.is_empty() && tokens_used + line_tokens > token_budget {
            break;
        }

        tokens_used += line_tokens;
        lines.push(line);
        last_idx = i;
    }

    (lines.join("\n"), last_idx)
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

    #[test]
    fn test_format_user_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::user_home(
            test_snowflake_seq(1), "cassie".into(), "u1".into(), "hello world".into(),
            "discord".into(), "general".into(), Some("general".into()), None,
        ));
        let result = format_entry(&entry);
        let sf = test_snowflake_seq(1);
        assert_eq!(result, Some(format!("[{}] user:discord:general/general cassie: hello world", sf)));
    }

    #[test]
    fn test_format_agent_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::agent(
            test_snowflake_seq(2), "hi there!".into(), "home".into(), None,
        ));
        let result = format_entry(&entry);
        let sf = test_snowflake_seq(2);
        assert_eq!(result, Some(format!("[{}] agent: hi there!", sf)));
    }

    #[test]
    fn test_format_bystander_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::bystander(
            test_snowflake_seq(3), "interesting work".into(),
        ));
        let result = format_entry(&entry);
        let sf = test_snowflake_seq(3);
        assert_eq!(result, Some(format!("[{}] bystander: interesting work", sf)));
    }

    #[test]
    fn test_format_system_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::system_msg(
            test_snowflake_seq(4), "context pressure warning".into(),
        ));
        let result = format_entry(&entry);
        let sf = test_snowflake_seq(4);
        assert_eq!(result, Some(format!("[{}] system: context pressure warning", sf)));
    }

    #[test]
    fn test_format_spectator_message_filtered() {
        let entry = HomeChannelEntry::Message(MessageEntry::system_msg(
            test_snowflake_seq(10), "[spectator] move written covering entries abc001-abc009".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_tool_call() {
        let entry = HomeChannelEntry::Tool(ToolEntry::call(
            test_snowflake_seq(5), "read_file".into(),
            serde_json::json!({"path": "/tmp/test.txt"}), "tc1".into(),
        ));
        let result = format_entry(&entry);
        let sf = test_snowflake_seq(5);
        assert_eq!(result, Some(format!("[{}] tool_call: read_file", sf)));
    }

    #[test]
    fn test_format_tool_result() {
        let entry = HomeChannelEntry::Tool(ToolEntry::result(
            test_snowflake_seq(6), "read_file".into(),
            "file contents here, this is some data".into(), "tc1".into(),
        ));
        let result = format_entry(&entry);
        let sf = test_snowflake_seq(6);
        assert_eq!(result, Some(format!("[{}] tool_result(read_file): [37 bytes]", sf)));
    }

    #[test]
    fn test_format_tool_result_file() {
        let entry = HomeChannelEntry::Tool(ToolEntry::result_file(
            test_snowflake_seq(7), "bash".into(),
            "/tmp/results/abc007.txt".into(), "tc2".into(),
        ));
        let result = format_entry(&entry);
        let sf = test_snowflake_seq(7);
        assert_eq!(result, Some(format!("[{}] tool_result(bash): [file: /tmp/results/abc007.txt]", sf)));
    }

    #[test]
    fn test_format_heartbeat_filtered() {
        let entry = HomeChannelEntry::Heartbeat(HeartbeatEntry::new(
            test_snowflake_seq(8), "2026-05-12T12:00:00Z".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_cursor_filtered() {
        let entry = HomeChannelEntry::Cursor(CursorEntry::new(test_snowflake_seq(9)));
        let result = format_entry(&entry);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_entries_with_budget() {
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(1), "short".into(), "home".into(), None,
            )),
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(2), "also short".into(), "home".into(), None,
            )),
            HomeChannelEntry::Message(MessageEntry::agent(
                test_snowflake_seq(3), "third message".into(), "home".into(), None,
            )),
        ];

        // Large budget — all entries fit
        let (transcript, last_idx) = format_entries_budgeted(&entries, 10000);
        assert_eq!(last_idx, 2);
        assert!(transcript.contains(&format!("[{}]", test_snowflake_seq(1))));
        assert!(transcript.contains(&format!("[{}]", test_snowflake_seq(3))));

        // Tiny budget — only first entry fits
        let (transcript, last_idx) = format_entries_budgeted(&entries, 10);
        assert_eq!(last_idx, 0);
        assert!(transcript.contains(&format!("[{}]", test_snowflake_seq(1))));
        assert!(!transcript.contains(&format!("[{}]", test_snowflake_seq(2))));
    }
}

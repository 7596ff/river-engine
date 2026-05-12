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
    use crate::channels::entry::*;

    #[test]
    fn test_format_user_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::user_home(
            "abc001".into(), "cassie".into(), "u1".into(), "hello world".into(),
            "discord".into(), "general".into(), Some("general".into()), None,
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc001] user:discord:general/general cassie: hello world".to_string()));
    }

    #[test]
    fn test_format_agent_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::agent(
            "abc002".into(), "hi there!".into(), "home".into(), None,
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc002] agent: hi there!".to_string()));
    }

    #[test]
    fn test_format_bystander_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::bystander(
            "abc003".into(), "interesting work".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc003] bystander: interesting work".to_string()));
    }

    #[test]
    fn test_format_system_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::system_msg(
            "abc004".into(), "context pressure warning".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc004] system: context pressure warning".to_string()));
    }

    #[test]
    fn test_format_spectator_message_filtered() {
        let entry = HomeChannelEntry::Message(MessageEntry::system_msg(
            "abc010".into(), "[spectator] move written covering entries abc001-abc009".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_tool_call() {
        let entry = HomeChannelEntry::Tool(ToolEntry::call(
            "abc005".into(), "read_file".into(),
            serde_json::json!({"path": "/tmp/test.txt"}), "tc1".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc005] tool_call: read_file".to_string()));
    }

    #[test]
    fn test_format_tool_result() {
        let entry = HomeChannelEntry::Tool(ToolEntry::result(
            "abc006".into(), "read_file".into(),
            "file contents here, this is some data".into(), "tc1".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc006] tool_result(read_file): [37 bytes]".to_string()));
    }

    #[test]
    fn test_format_tool_result_file() {
        let entry = HomeChannelEntry::Tool(ToolEntry::result_file(
            "abc007".into(), "bash".into(),
            "/tmp/results/abc007.txt".into(), "tc2".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc007] tool_result(bash): [file: /tmp/results/abc007.txt]".to_string()));
    }

    #[test]
    fn test_format_heartbeat_filtered() {
        let entry = HomeChannelEntry::Heartbeat(HeartbeatEntry::new(
            "abc008".into(), "2026-05-12T12:00:00Z".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_cursor_filtered() {
        let entry = HomeChannelEntry::Cursor(CursorEntry::new("abc009".into()));
        let result = format_entry(&entry);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_entries_with_budget() {
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::agent(
                "001".into(), "short".into(), "home".into(), None,
            )),
            HomeChannelEntry::Message(MessageEntry::agent(
                "002".into(), "also short".into(), "home".into(), None,
            )),
            HomeChannelEntry::Message(MessageEntry::agent(
                "003".into(), "third message".into(), "home".into(), None,
            )),
        ];

        // Large budget — all entries fit
        let (transcript, last_idx) = format_entries_budgeted(&entries, 10000);
        assert_eq!(last_idx, 2);
        assert!(transcript.contains("[001]"));
        assert!(transcript.contains("[003]"));

        // Tiny budget — only first entry fits
        let (transcript, last_idx) = format_entries_budgeted(&entries, 10);
        assert_eq!(last_idx, 0);
        assert!(transcript.contains("[001]"));
        assert!(!transcript.contains("[002]"));
    }
}

//! TUI-specific entry formatting
//!
//! TuiEntry wraps HomeChannelEntry with collapsed tool rendering.
//! HomeChannelFormatter handles stateful tool call pairing.

use river_core::channels::entry::{HomeChannelEntry, ToolEntry};
use std::collections::HashMap;
use std::fmt;

/// Newtype for TUI-specific Display.
pub struct TuiEntry(pub HomeChannelEntry);

/// A formatted line ready for display.
#[derive(Debug, Clone)]
pub struct FormattedLine {
    pub text: String,
}

/// Stateful formatter that pairs tool calls with results.
pub struct HomeChannelFormatter {
    pending_calls: HashMap<String, PendingCall>,
}

#[derive(Debug)]
struct PendingCall {
    tool_name: String,
    args_summary: String,
    timestamp: String,
}

fn format_time(id: &river_core::Snowflake) -> String {
    id.to_datetime().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn summarize_args(args: &Option<serde_json::Value>) -> String {
    match args {
        Some(v) => {
            let s = serde_json::to_string(v).unwrap_or_default();
            if s.len() > 60 {
                format!("{}…", &s[..57])
            } else {
                s
            }
        }
        None => String::new(),
    }
}

fn summarize_result(t: &ToolEntry) -> String {
    if let Some(ref file) = t.result_file {
        file.clone()
    } else if let Some(ref result) = t.result {
        let lines = result.lines().count();
        if lines > 1 {
            format!("{} lines", lines)
        } else if result.is_empty() {
            "ok".to_string()
        } else if result.len() > 80 {
            format!("{}…", &result[..77])
        } else {
            result.clone()
        }
    } else {
        "ok".to_string()
    }
}

impl fmt::Display for TuiEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            HomeChannelEntry::Tool(t) => {
                let time = format_time(&t.id);
                match t.kind.as_str() {
                    "tool_call" => {
                        let args = summarize_args(&t.arguments);
                        write!(f, "{} 🔧 {}({})", time, t.tool_name, args)
                    }
                    "tool_result" => {
                        let summary = summarize_result(t);
                        write!(f, "{} 🔧 {} → {}", time, t.tool_name, summary)
                    }
                    _ => write!(f, "{}", self.0),
                }
            }
            other => write!(f, "{}", other),
        }
    }
}

impl HomeChannelFormatter {
    pub fn new() -> Self {
        Self {
            pending_calls: HashMap::new(),
        }
    }

    /// Push an entry and get back formatted lines.
    pub fn push(&mut self, entry: HomeChannelEntry) -> Vec<FormattedLine> {
        match &entry {
            HomeChannelEntry::Tool(t) if t.kind == "tool_call" => {
                let time = format_time(&t.id);
                let args = summarize_args(&t.arguments);
                let text = format!("{} 🔧 {}({})", time, t.tool_name, args);
                self.pending_calls.insert(
                    t.tool_call_id.clone(),
                    PendingCall {
                        tool_name: t.tool_name.clone(),
                        args_summary: args,
                        timestamp: time,
                    },
                );
                vec![FormattedLine { text }]
            }
            HomeChannelEntry::Tool(t) if t.kind == "tool_result" => {
                let summary = summarize_result(t);
                let text =
                    if let Some(call) = self.pending_calls.remove(&t.tool_call_id) {
                        format!(
                            "{} 🔧 {}({}) → {}",
                            call.timestamp, call.tool_name, call.args_summary, summary
                        )
                    } else {
                        let time = format_time(&t.id);
                        format!("{} 🔧 {} → {}", time, t.tool_name, summary)
                    };
                vec![FormattedLine { text }]
            }
            _ => {
                let text = format!("{}", TuiEntry(entry));
                vec![FormattedLine { text }]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::channels::entry::*;
    use river_core::snowflake::{AgentBirth, SnowflakeType};
    use river_core::Snowflake;

    fn test_snowflake() -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(0, birth, SnowflakeType::Message, 0)
    }

    #[test]
    fn test_tui_entry_message_delegates() {
        let entry = HomeChannelEntry::Message(MessageEntry::agent(
            test_snowflake(),
            "hello".into(),
            "home".into(),
            None,
        ));
        let tui = TuiEntry(entry.clone());
        assert_eq!(format!("{}", tui), format!("{}", entry));
    }

    #[test]
    fn test_tui_entry_heartbeat_delegates() {
        let entry = HomeChannelEntry::Heartbeat(HeartbeatEntry::new(
            test_snowflake(),
            "2026-05-14T12:00:00Z".into(),
        ));
        let tui = TuiEntry(entry.clone());
        assert_eq!(format!("{}", tui), format!("{}", entry));
    }

    #[test]
    fn test_formatter_tool_call_then_result() {
        let mut fmt = HomeChannelFormatter::new();

        let call = HomeChannelEntry::Tool(ToolEntry::call(
            test_snowflake(),
            "read_file".into(),
            serde_json::json!({"path": "src/main.rs"}),
            "tc_001".into(),
        ));
        let lines = fmt.push(call);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("🔧 read_file"));
        assert!(!lines[0].text.contains("→"));

        let result = HomeChannelEntry::Tool(ToolEntry::result(
            test_snowflake(),
            "read_file".into(),
            "fn main() {}\n".repeat(100),
            "tc_001".into(),
        ));
        let lines = fmt.push(result);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("→ 100 lines"));
    }

    #[test]
    fn test_formatter_tool_result_file() {
        let mut fmt = HomeChannelFormatter::new();

        let call = HomeChannelEntry::Tool(ToolEntry::call(
            test_snowflake(),
            "bash".into(),
            serde_json::json!({"command": "ls"}),
            "tc_002".into(),
        ));
        fmt.push(call);

        let result = HomeChannelEntry::Tool(ToolEntry::result_file(
            test_snowflake(),
            "bash".into(),
            "tool-results/abc123.txt".into(),
            "tc_002".into(),
        ));
        let lines = fmt.push(result);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("→ tool-results/abc123.txt"));
    }

    #[test]
    fn test_formatter_orphan_result() {
        let mut fmt = HomeChannelFormatter::new();

        let result = HomeChannelEntry::Tool(ToolEntry::result(
            test_snowflake(),
            "read_file".into(),
            "some content\nmore content".into(),
            "tc_orphan".into(),
        ));
        let lines = fmt.push(result);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("🔧 read_file → 2 lines"));
    }

    #[test]
    fn test_formatter_message_passthrough() {
        let mut fmt = HomeChannelFormatter::new();

        let msg = HomeChannelEntry::Message(MessageEntry::agent(
            test_snowflake(),
            "hello".into(),
            "home".into(),
            None,
        ));
        let lines = fmt.push(msg);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("[agent]"));
        assert!(lines[0].text.contains("hello"));
    }
}

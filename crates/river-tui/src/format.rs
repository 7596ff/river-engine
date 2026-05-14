//! TUI-specific entry formatting
//!
//! TuiEntry wraps HomeChannelEntry with collapsed tool rendering.

use river_core::channels::entry::{HomeChannelEntry, ToolEntry};
use std::fmt;

/// Newtype for TUI-specific Display.
pub struct TuiEntry(pub HomeChannelEntry);

/// A formatted line ready for display.
#[derive(Debug, Clone)]
pub struct FormattedLine {
    pub text: String,
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

/// Format a HomeChannelEntry into a FormattedLine.
pub fn format_entry(entry: HomeChannelEntry) -> FormattedLine {
    FormattedLine {
        text: format!("{}", TuiEntry(entry)),
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
    fn test_tool_call_collapsed() {
        let entry = HomeChannelEntry::Tool(ToolEntry::call(
            test_snowflake(),
            "read_file".into(),
            serde_json::json!({"path": "src/main.rs"}),
            "tc_001".into(),
        ));
        let line = format_entry(entry);
        assert!(line.text.contains("🔧 read_file"));
    }

    #[test]
    fn test_tool_result_collapsed() {
        let entry = HomeChannelEntry::Tool(ToolEntry::result(
            test_snowflake(),
            "read_file".into(),
            "fn main() {}\n".repeat(100),
            "tc_001".into(),
        ));
        let line = format_entry(entry);
        assert!(line.text.contains("🔧 read_file → 100 lines"));
    }

    #[test]
    fn test_tool_result_file() {
        let entry = HomeChannelEntry::Tool(ToolEntry::result_file(
            test_snowflake(),
            "bash".into(),
            "tool-results/abc123.txt".into(),
            "tc_002".into(),
        ));
        let line = format_entry(entry);
        assert!(line.text.contains("→ tool-results/abc123.txt"));
    }

    #[test]
    fn test_message_passthrough() {
        let entry = HomeChannelEntry::Message(MessageEntry::agent(
            test_snowflake(),
            "hello".into(),
            "home".into(),
            None,
        ));
        let line = format_entry(entry);
        assert!(line.text.contains("[agent]"));
        assert!(line.text.contains("hello"));
    }
}

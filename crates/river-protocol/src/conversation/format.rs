//! Conversation file format parsing and serialization.

use super::types::{Line, Message, MessageDirection, Reaction};
use crate::Author;

/// YAML frontmatter delimiter.
pub const FRONTMATTER_DELIMITER: &str = "---";

/// Parse a direction marker from a string.
pub fn parse_direction_marker(s: &str) -> Option<MessageDirection> {
    match s {
        "[ ]" => Some(MessageDirection::Unread),
        "[x]" => Some(MessageDirection::Read),
        "[>]" => Some(MessageDirection::Outgoing),
        "[!]" => Some(MessageDirection::Failed),
        _ => None,
    }
}

/// Convert a direction to its marker string.
pub fn direction_to_marker(d: MessageDirection) -> &'static str {
    match d {
        MessageDirection::Unread => "[ ]",
        MessageDirection::Read => "[x]",
        MessageDirection::Outgoing => "[>]",
        MessageDirection::Failed => "[!]",
    }
}

/// Parse a reaction line (indented with 4 spaces).
/// Formats:
/// - `    👍 bob, charlie` — usernames known
/// - `    👍 3` — count only (no usernames)
/// - `    👍 bob, charlie +1` — mixed
pub fn parse_reaction_line(line: &str) -> Option<Reaction> {
    if !line.starts_with("    ") {
        return None;
    }

    let content = line[4..].trim();
    let space_idx = content.find(' ')?;
    let emoji = content[..space_idx].to_string();
    let rest = content[space_idx + 1..].trim();

    if let Some(plus_idx) = rest.find(" +") {
        // Format: "users +N"
        let users_part = &rest[..plus_idx];
        let count_part = &rest[plus_idx + 2..];
        let users: Vec<String> = users_part.split(',').map(|s| s.trim().to_string()).collect();
        let unknown_count = count_part.parse::<usize>().ok()?;
        Some(Reaction { emoji, users, unknown_count })
    } else if rest.chars().all(|c| c.is_ascii_digit()) {
        // Format: "N" (count only)
        let unknown_count = rest.parse::<usize>().ok()?;
        Some(Reaction { emoji, users: vec![], unknown_count })
    } else {
        // Format: "users"
        let users: Vec<String> = rest.split(',').map(|s| s.trim().to_string()).collect();
        Some(Reaction { emoji, users, unknown_count: 0 })
    }
}

/// Format a reaction as a string.
pub fn format_reaction(r: &Reaction) -> String {
    if r.users.is_empty() {
        format!("    {} {}", r.emoji, r.unknown_count)
    } else if r.unknown_count > 0 {
        format!("    {} {} +{}", r.emoji, r.users.join(", "), r.unknown_count)
    } else {
        format!("    {} {}", r.emoji, r.users.join(", "))
    }
}

/// Parse a message line.
/// Format: `[marker] timestamp id <author_name:author_id> content`
pub fn parse_message_line(line: &str) -> Result<Message, String> {
    let line = line.trim();
    if line.is_empty() {
        return Err("empty line".to_string());
    }

    let (direction, rest) = if line.starts_with("[!] ") {
        (MessageDirection::Failed, &line[4..])
    } else if line.starts_with("[>] ") {
        (MessageDirection::Outgoing, &line[4..])
    } else if line.starts_with("[x] ") {
        (MessageDirection::Read, &line[4..])
    } else if line.starts_with("[ ] ") {
        (MessageDirection::Unread, &line[4..])
    } else {
        return Err("missing direction marker ([ ], [x], [>], or [!])".to_string());
    };

    // Split: date time id <author> content
    let mut parts = rest.splitn(4, ' ');
    let date = parts.next().ok_or("missing date")?;
    let time = parts.next().ok_or("missing time")?;
    let timestamp = format!("{} {}", date, time);
    let id = parts.next().ok_or("missing message ID")?.to_string();
    let remainder = parts.next().ok_or("missing author and content")?;

    if !remainder.starts_with('<') {
        return Err("missing author opening bracket '<'".to_string());
    }

    let author_end = remainder.find('>').ok_or("missing author closing bracket '>'")?;
    let author_part = &remainder[1..author_end];
    let content_start = author_end + 2;

    let (author_name, author_id) = author_part.rsplit_once(':')
        .ok_or("invalid author format (expected 'name:id')")?;

    let content = if content_start < remainder.len() {
        remainder[content_start..].to_string()
    } else {
        String::new()
    };

    Ok(Message {
        direction,
        timestamp,
        id,
        author: Author {
            name: author_name.to_string(),
            id: author_id.to_string(),
            bot: false, // Default, not stored in file format
        },
        content,
        reactions: vec![],
    })
}

/// Parse a read receipt line.
/// Format: `[r] timestamp message_id`
pub fn parse_read_receipt_line(line: &str) -> Option<Line> {
    let line = line.trim();
    if !line.starts_with("[r] ") {
        return None;
    }

    let rest = &line[4..];
    let mut parts = rest.splitn(3, ' ');
    let date = parts.next()?;
    let time = parts.next()?;
    let timestamp = format!("{} {}", date, time);
    let message_id = parts.next()?.to_string();

    Some(Line::ReadReceipt { timestamp, message_id })
}

/// Format a message as a string (including reactions).
pub fn format_message(msg: &Message) -> String {
    let marker = direction_to_marker(msg.direction);
    let mut result = format!(
        "{} {} {} <{}:{}> {}",
        marker, msg.timestamp, msg.id, msg.author.name, msg.author.id, msg.content
    );

    for reaction in &msg.reactions {
        result.push('\n');
        result.push_str(&format_reaction(reaction));
    }

    result
}

/// Format a line (message or read receipt).
pub fn format_line(line: &Line) -> String {
    match line {
        Line::Message(msg) => format_message(msg),
        Line::ReadReceipt { timestamp, message_id } => {
            format!("[r] {} {}", timestamp, message_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_direction_marker() {
        assert_eq!(parse_direction_marker("[ ]"), Some(MessageDirection::Unread));
        assert_eq!(parse_direction_marker("[x]"), Some(MessageDirection::Read));
        assert_eq!(parse_direction_marker("[>]"), Some(MessageDirection::Outgoing));
        assert_eq!(parse_direction_marker("[!]"), Some(MessageDirection::Failed));
        assert_eq!(parse_direction_marker("???"), None);
    }

    #[test]
    fn test_direction_to_marker() {
        assert_eq!(direction_to_marker(MessageDirection::Unread), "[ ]");
        assert_eq!(direction_to_marker(MessageDirection::Read), "[x]");
        assert_eq!(direction_to_marker(MessageDirection::Outgoing), "[>]");
        assert_eq!(direction_to_marker(MessageDirection::Failed), "[!]");
    }

    #[test]
    fn test_parse_reaction_known_users() {
        let r = parse_reaction_line("    👍 bob, charlie").unwrap();
        assert_eq!(r.emoji, "👍");
        assert_eq!(r.users, vec!["bob", "charlie"]);
        assert_eq!(r.unknown_count, 0);
    }

    #[test]
    fn test_parse_reaction_count_only() {
        let r = parse_reaction_line("    ❤️ 3").unwrap();
        assert_eq!(r.emoji, "❤️");
        assert!(r.users.is_empty());
        assert_eq!(r.unknown_count, 3);
    }

    #[test]
    fn test_parse_reaction_mixed() {
        let r = parse_reaction_line("    🎉 river +2").unwrap();
        assert_eq!(r.emoji, "🎉");
        assert_eq!(r.users, vec!["river"]);
        assert_eq!(r.unknown_count, 2);
    }

    #[test]
    fn test_parse_reaction_not_indented() {
        assert!(parse_reaction_line("👍 bob").is_none());
        assert!(parse_reaction_line("  👍 bob").is_none());
    }

    #[test]
    fn test_format_reaction_roundtrip() {
        let reactions = vec![
            Reaction { emoji: "👍".to_string(), users: vec!["bob".to_string()], unknown_count: 0 },
            Reaction { emoji: "❤️".to_string(), users: vec![], unknown_count: 3 },
            Reaction { emoji: "🎉".to_string(), users: vec!["river".to_string()], unknown_count: 2 },
        ];
        for r in reactions {
            let formatted = format_reaction(&r);
            let parsed = parse_reaction_line(&formatted).unwrap();
            assert_eq!(parsed, r);
        }
    }

    #[test]
    fn test_parse_message_line_unread() {
        let line = "[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey, can you help?";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.direction, MessageDirection::Unread);
        assert_eq!(msg.timestamp, "2026-04-03 14:30:00");
        assert_eq!(msg.id, "msg123");
        assert_eq!(msg.author.name, "alice");
        assert_eq!(msg.author.id, "111");
        assert_eq!(msg.content, "hey, can you help?");
    }

    #[test]
    fn test_parse_message_line_outgoing() {
        let line = "[>] 2026-04-03 14:30:15 msg124 <river:999> Sure! What do you need?";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.direction, MessageDirection::Outgoing);
        assert_eq!(msg.author.name, "river");
    }

    #[test]
    fn test_parse_message_line_failed() {
        let line = "[!] 2026-04-03 14:31:00 - <river:999> (failed: error) Original message";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.direction, MessageDirection::Failed);
        assert_eq!(msg.id, "-");
    }

    #[test]
    fn test_parse_read_receipt_line() {
        let line = "[r] 2026-04-03 14:30:05 msg123";
        let receipt = parse_read_receipt_line(line).unwrap();
        match receipt {
            Line::ReadReceipt { timestamp, message_id } => {
                assert_eq!(timestamp, "2026-04-03 14:30:05");
                assert_eq!(message_id, "msg123");
            }
            _ => panic!("Expected ReadReceipt"),
        }
    }

    #[test]
    fn test_format_message_simple() {
        let msg = Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg123".to_string(),
            author: Author { name: "alice".to_string(), id: "111".to_string(), bot: false },
            content: "hey".to_string(),
            reactions: vec![],
        };
        let formatted = format_message(&msg);
        assert_eq!(formatted, "[ ] 2026-04-03 14:30:00 msg123 <alice:111> hey");
    }

    #[test]
    fn test_format_message_with_reactions() {
        let msg = Message {
            direction: MessageDirection::Read,
            timestamp: "2026-04-03 14:30:00".to_string(),
            id: "msg123".to_string(),
            author: Author { name: "alice".to_string(), id: "111".to_string(), bot: false },
            content: "hey".to_string(),
            reactions: vec![
                Reaction { emoji: "👍".to_string(), users: vec!["bob".to_string()], unknown_count: 0 },
            ],
        };
        let formatted = format_message(&msg);
        assert!(formatted.contains("[x] 2026-04-03 14:30:00 msg123 <alice:111> hey"));
        assert!(formatted.contains("\n    👍 bob"));
    }

    #[test]
    fn test_format_line_read_receipt() {
        let line = Line::ReadReceipt {
            timestamp: "2026-04-03 14:30:05".to_string(),
            message_id: "msg123".to_string(),
        };
        assert_eq!(format_line(&line), "[r] 2026-04-03 14:30:05 msg123");
    }

    #[test]
    fn test_message_roundtrip() {
        let original = Message {
            direction: MessageDirection::Outgoing,
            timestamp: "2026-04-03 14:30:15".to_string(),
            id: "msg124".to_string(),
            author: Author { name: "river".to_string(), id: "999".to_string(), bot: true },
            content: "Sure! What do you need?".to_string(),
            reactions: vec![],
        };
        let formatted = format_message(&original);
        let parsed = parse_message_line(&formatted).unwrap();
        assert_eq!(parsed.direction, original.direction);
        assert_eq!(parsed.timestamp, original.timestamp);
        assert_eq!(parsed.id, original.id);
        assert_eq!(parsed.author.name, original.author.name);
        assert_eq!(parsed.author.id, original.author.id);
        assert_eq!(parsed.content, original.content);
    }
}

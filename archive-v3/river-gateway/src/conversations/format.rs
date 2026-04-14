//! Custom human-readable serialization for conversations
//!
//! Format:
//! - Messages are lines starting with `[marker]`
//! - Reactions are indented lines beneath their message
//!
//! Example:
//! ```text
//! [ ] 2026-03-23 14:30:00 msg123 <alice:111> hey, can you help?
//!     👍 bob, charlie
//!     ❤️ 3
//! [>] 2026-03-23 14:30:15 msg124 <river:999> Sure! What do you need?
//! [x] 2026-03-23 14:30:30 msg125 <alice:111> I'm trying to deploy...
//!     🎉 river +2
//! [!] 2026-03-23 14:31:00 - <river:999> (failed: Connection timeout) Original message
//! ```

use super::{Author, Message, MessageDirection, Reaction};

/// YAML frontmatter delimiter
pub const FRONTMATTER_DELIMITER: &str = "---";

/// Parse a direction marker from a string
pub fn parse_direction_marker(s: &str) -> Option<MessageDirection> {
    match s {
        "[ ]" => Some(MessageDirection::Unread),
        "[x]" => Some(MessageDirection::Read),
        "[>]" => Some(MessageDirection::Outgoing),
        "[!]" => Some(MessageDirection::Failed),
        _ => None,
    }
}

/// Convert a direction to its marker string
pub fn direction_to_marker(d: MessageDirection) -> &'static str {
    match d {
        MessageDirection::Unread => "[ ]",
        MessageDirection::Read => "[x]",
        MessageDirection::Outgoing => "[>]",
        MessageDirection::Failed => "[!]",
    }
}

/// Escape content for multi-line storage
/// Escapes: \ -> \\, newlines are preserved (not escaped like in inbox format)
pub fn escape_content(content: &str) -> String {
    // For now, we preserve newlines in the content.
    // This is different from inbox format which escapes them.
    content.to_string()
}

/// Unescape content from storage format
pub fn unescape_content(content: &str) -> String {
    content.to_string()
}

/// Parse a reaction line (indented with 4 spaces)
/// Formats:
/// - `    👍 bob, charlie` — usernames known
/// - `    👍 3` — count only (no usernames)
/// - `    👍 bob, charlie +1` — 2 known + 1 unknown = 3 total
pub fn parse_reaction_line(line: &str) -> Option<Reaction> {
    // Must start with exactly 4 spaces
    if !line.starts_with("    ") {
        return None;
    }

    let content = line[4..].trim();

    // Find first space to split emoji from rest
    let space_idx = content.find(' ')?;
    let emoji = content[..space_idx].to_string();
    let rest = content[space_idx + 1..].trim();

    // Parse the rest: either "N", "users", or "users +N"
    if let Some(plus_idx) = rest.find(" +") {
        // Format: "users +N"
        let users_part = &rest[..plus_idx];
        let count_part = &rest[plus_idx + 2..];

        let users: Vec<String> = users_part
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let unknown_count = count_part.parse::<usize>().ok()?;

        Some(Reaction {
            emoji,
            users,
            unknown_count,
        })
    } else if rest.chars().all(|c| c.is_ascii_digit()) {
        // Format: "N" (count only)
        let unknown_count = rest.parse::<usize>().ok()?;
        Some(Reaction {
            emoji,
            users: vec![],
            unknown_count,
        })
    } else {
        // Format: "users"
        let users: Vec<String> = rest
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        Some(Reaction {
            emoji,
            users,
            unknown_count: 0,
        })
    }
}

/// Format a reaction as a string
pub fn format_reaction(r: &Reaction) -> String {
    if r.users.is_empty() {
        // Count only: "    👍 3"
        format!("    {} {}", r.emoji, r.unknown_count)
    } else if r.unknown_count > 0 {
        // Mixed: "    👍 bob, charlie +2"
        let users = r.users.join(", ");
        format!("    {} {} +{}", r.emoji, users, r.unknown_count)
    } else {
        // Known users only: "    👍 bob, charlie"
        let users = r.users.join(", ");
        format!("    {} {}", r.emoji, users)
    }
}

/// Parse a message line (without reactions)
/// Format: [marker] timestamp id <author:id> content
/// Example: [ ] 2026-03-23 14:30:00 msg123 <alice:111> hey, can you help?
pub fn parse_message_line(line: &str) -> Option<Message> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Parse direction marker
    let (direction, rest) = if line.starts_with("[!] ") {
        (MessageDirection::Failed, &line[4..])
    } else if line.starts_with("[>] ") {
        (MessageDirection::Outgoing, &line[4..])
    } else if line.starts_with("[x] ") {
        (MessageDirection::Read, &line[4..])
    } else if line.starts_with("[ ] ") {
        (MessageDirection::Unread, &line[4..])
    } else {
        return None;
    };

    // Split into parts: date, time, message_id, <author>, content
    let mut parts = rest.splitn(4, ' ');

    let date = parts.next()?;
    let time = parts.next()?;
    let timestamp = format!("{} {}", date, time);

    let id = parts.next()?.to_string();

    let remainder = parts.next()?;

    // Parse <name:id> and content
    if !remainder.starts_with('<') {
        return None;
    }

    let author_end = remainder.find('>')?;
    let author_part = &remainder[1..author_end];
    let content_start = author_end + 2; // Skip "> "

    let (author_name, author_id) = author_part.rsplit_once(':')?;

    let content = if content_start < remainder.len() {
        unescape_content(&remainder[content_start..])
    } else {
        String::new()
    };

    Some(Message {
        direction,
        timestamp,
        id,
        author: Author {
            name: author_name.to_string(),
            id: author_id.to_string(),
        },
        content,
        reactions: vec![],
    })
}

/// Format a message as a string (including reactions)
pub fn format_message(msg: &Message) -> String {
    let marker = direction_to_marker(msg.direction);
    let mut result = format!(
        "{} {} {} <{}:{}> {}",
        marker,
        msg.timestamp,
        msg.id,
        msg.author.name,
        msg.author.id,
        escape_content(&msg.content)
    );

    for reaction in &msg.reactions {
        result.push('\n');
        result.push_str(&format_reaction(reaction));
    }

    result
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
    fn test_parse_reaction_line() {
        // Known users
        let r = parse_reaction_line("    👍 bob, charlie").unwrap();
        assert_eq!(r.emoji, "👍");
        assert_eq!(r.users, vec!["bob", "charlie"]);
        assert_eq!(r.unknown_count, 0);

        // Count only
        let r = parse_reaction_line("    ❤️ 3").unwrap();
        assert_eq!(r.emoji, "❤️");
        assert!(r.users.is_empty());
        assert_eq!(r.unknown_count, 3);

        // Mixed
        let r = parse_reaction_line("    🎉 river +2").unwrap();
        assert_eq!(r.emoji, "🎉");
        assert_eq!(r.users, vec!["river"]);
        assert_eq!(r.unknown_count, 2);
    }

    #[test]
    fn test_parse_reaction_line_invalid() {
        // Not indented
        assert!(parse_reaction_line("👍 bob").is_none());

        // Wrong indentation
        assert!(parse_reaction_line("  👍 bob").is_none());

        // No space after emoji
        assert!(parse_reaction_line("    👍").is_none());
    }

    #[test]
    fn test_format_reaction() {
        // Known users only
        let r = Reaction {
            emoji: "👍".to_string(),
            users: vec!["bob".to_string(), "charlie".to_string()],
            unknown_count: 0,
        };
        assert_eq!(format_reaction(&r), "    👍 bob, charlie");

        // Count only
        let r = Reaction {
            emoji: "❤️".to_string(),
            users: vec![],
            unknown_count: 3,
        };
        assert_eq!(format_reaction(&r), "    ❤️ 3");

        // Mixed
        let r = Reaction {
            emoji: "🎉".to_string(),
            users: vec!["river".to_string()],
            unknown_count: 2,
        };
        assert_eq!(format_reaction(&r), "    🎉 river +2");
    }

    #[test]
    fn test_parse_message_line() {
        let line = "[ ] 2026-03-23 14:30:00 msg123 <alice:111> hey, can you help?";
        let msg = parse_message_line(line).unwrap();

        assert_eq!(msg.direction, MessageDirection::Unread);
        assert_eq!(msg.timestamp, "2026-03-23 14:30:00");
        assert_eq!(msg.id, "msg123");
        assert_eq!(msg.author.name, "alice");
        assert_eq!(msg.author.id, "111");
        assert_eq!(msg.content, "hey, can you help?");
        assert!(msg.reactions.is_empty());
    }

    #[test]
    fn test_parse_message_line_all_directions() {
        let msg = parse_message_line("[ ] 2026-03-23 14:30:00 msg123 <alice:111> test").unwrap();
        assert_eq!(msg.direction, MessageDirection::Unread);

        let msg = parse_message_line("[x] 2026-03-23 14:30:00 msg123 <alice:111> test").unwrap();
        assert_eq!(msg.direction, MessageDirection::Read);

        let msg = parse_message_line("[>] 2026-03-23 14:30:00 msg123 <alice:111> test").unwrap();
        assert_eq!(msg.direction, MessageDirection::Outgoing);

        let msg = parse_message_line("[!] 2026-03-23 14:30:00 - <river:999> (failed: error) test").unwrap();
        assert_eq!(msg.direction, MessageDirection::Failed);
        assert_eq!(msg.id, "-");
    }

    #[test]
    fn test_format_message_simple() {
        let msg = Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-03-23 14:30:00".to_string(),
            id: "msg123".to_string(),
            author: Author {
                name: "alice".to_string(),
                id: "111".to_string(),
            },
            content: "hey, can you help?".to_string(),
            reactions: vec![],
        };

        let formatted = format_message(&msg);
        assert_eq!(formatted, "[ ] 2026-03-23 14:30:00 msg123 <alice:111> hey, can you help?");
    }

    #[test]
    fn test_format_message_with_reactions() {
        let msg = Message {
            direction: MessageDirection::Read,
            timestamp: "2026-03-23 14:30:00".to_string(),
            id: "msg123".to_string(),
            author: Author {
                name: "alice".to_string(),
                id: "111".to_string(),
            },
            content: "hey, can you help?".to_string(),
            reactions: vec![
                Reaction {
                    emoji: "👍".to_string(),
                    users: vec!["bob".to_string(), "charlie".to_string()],
                    unknown_count: 0,
                },
                Reaction {
                    emoji: "❤️".to_string(),
                    users: vec![],
                    unknown_count: 3,
                },
            ],
        };

        let formatted = format_message(&msg);
        let expected = "[x] 2026-03-23 14:30:00 msg123 <alice:111> hey, can you help?\n    👍 bob, charlie\n    ❤️ 3";
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_format_parse_message_roundtrip() {
        let original = Message {
            direction: MessageDirection::Outgoing,
            timestamp: "2026-03-23 14:30:15".to_string(),
            id: "msg124".to_string(),
            author: Author {
                name: "river".to_string(),
                id: "999".to_string(),
            },
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

    #[test]
    fn test_format_parse_reaction_roundtrip() {
        let reactions = vec![
            Reaction {
                emoji: "👍".to_string(),
                users: vec!["bob".to_string(), "charlie".to_string()],
                unknown_count: 0,
            },
            Reaction {
                emoji: "❤️".to_string(),
                users: vec![],
                unknown_count: 3,
            },
            Reaction {
                emoji: "🎉".to_string(),
                users: vec!["river".to_string()],
                unknown_count: 2,
            },
        ];

        for original in reactions {
            let formatted = format_reaction(&original);
            let parsed = parse_reaction_line(&formatted).unwrap();
            assert_eq!(parsed.emoji, original.emoji);
            assert_eq!(parsed.users, original.users);
            assert_eq!(parsed.unknown_count, original.unknown_count);
        }
    }
}

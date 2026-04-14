//! Inbox/chat line formatting and parsing
//!
//! Supports bidirectional messages:
//! - `[ ]` incoming unread
//! - `[x]` incoming read
//! - `[>]` outgoing (sent by agent)
//! - `[!]` failed to send

/// Message direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDirection {
    /// Incoming message (from external user)
    Incoming { read: bool },
    /// Outgoing message (sent by agent)
    Outgoing,
    /// Failed to send
    Failed,
}

/// Escape content for single-line storage
/// Escapes: \ -> \\, \n -> \\n, \r -> \\r
pub fn escape_content(content: &str) -> String {
    content
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Unescape content from storage format
/// Unescapes: \\n -> \n, \\r -> \r, \\\\ -> \
pub fn unescape_content(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    result.push('\n');
                }
                Some('r') => {
                    chars.next();
                    result.push('\r');
                }
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                _ => result.push(c),
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Parsed inbox message
#[derive(Debug, Clone, PartialEq)]
pub struct InboxMessage {
    pub read: bool,
    pub timestamp: String,
    pub message_id: String,
    pub author_name: String,
    pub author_id: String,
    pub content: String,
    pub line_number: usize,
}

/// Format a message as an inbox line
pub fn format_inbox_line(
    timestamp: &str,
    message_id: &str,
    author_name: &str,
    author_id: &str,
    content: &str,
) -> String {
    format!(
        "[ ] {} {} <{}:{}> {}",
        timestamp,
        message_id,
        author_name,
        author_id,
        escape_content(content)
    )
}

/// Parse an inbox line into its components
/// Returns None if line is malformed
pub fn parse_inbox_line(line: &str, line_number: usize) -> Option<InboxMessage> {
    // Format: [status] timestamp messageId <name:id> content
    // Example: [ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there

    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Parse read status
    let (read, rest) = if line.starts_with("[x] ") {
        (true, &line[4..])
    } else if line.starts_with("[ ] ") {
        (false, &line[4..])
    } else {
        return None;
    };

    // Split into parts: timestamp (2 parts), message_id, <author>, content
    let mut parts = rest.splitn(4, ' ');

    let date = parts.next()?;
    let time = parts.next()?;
    let timestamp = format!("{} {}", date, time);

    let message_id = parts.next()?.to_string();

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

    Some(InboxMessage {
        read,
        timestamp,
        message_id,
        author_name: author_name.to_string(),
        author_id: author_id.to_string(),
        content,
        line_number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_content() {
        assert_eq!(escape_content("hello"), "hello");
        assert_eq!(escape_content("hello\nworld"), "hello\\nworld");
        assert_eq!(escape_content("a\\b"), "a\\\\b");
        assert_eq!(escape_content("a\r\nb"), "a\\r\\nb");
    }

    #[test]
    fn test_unescape_content() {
        assert_eq!(unescape_content("hello"), "hello");
        assert_eq!(unescape_content("hello\\nworld"), "hello\nworld");
        assert_eq!(unescape_content("a\\\\b"), "a\\b");
        assert_eq!(unescape_content("a\\r\\nb"), "a\r\nb");
    }

    #[test]
    fn test_escape_unescape_roundtrip() {
        let original = "hello\nworld\r\nwith\\backslash";
        let escaped = escape_content(original);
        let unescaped = unescape_content(&escaped);
        assert_eq!(unescaped, original);
    }

    #[test]
    fn test_format_inbox_line() {
        let line = format_inbox_line(
            "2026-03-18 22:15:32",
            "abc123",
            "alice",
            "123456789",
            "hello there",
        );
        assert_eq!(line, "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there");
    }

    #[test]
    fn test_format_inbox_line_with_newline() {
        let line = format_inbox_line(
            "2026-03-18 22:15:32",
            "abc123",
            "alice",
            "123456789",
            "hello\nthere",
        );
        assert_eq!(line, "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello\\nthere");
    }

    #[test]
    fn test_parse_inbox_line_unread() {
        let msg = parse_inbox_line(
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there",
            0,
        ).unwrap();

        assert!(!msg.read);
        assert_eq!(msg.timestamp, "2026-03-18 22:15:32");
        assert_eq!(msg.message_id, "abc123");
        assert_eq!(msg.author_name, "alice");
        assert_eq!(msg.author_id, "123456789");
        assert_eq!(msg.content, "hello there");
        assert_eq!(msg.line_number, 0);
    }

    #[test]
    fn test_parse_inbox_line_read() {
        let msg = parse_inbox_line(
            "[x] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there",
            5,
        ).unwrap();

        assert!(msg.read);
        assert_eq!(msg.line_number, 5);
    }

    #[test]
    fn test_parse_inbox_line_with_escaped_newline() {
        let msg = parse_inbox_line(
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello\\nthere",
            0,
        ).unwrap();

        assert_eq!(msg.content, "hello\nthere");
    }

    #[test]
    fn test_parse_inbox_line_empty_content() {
        let msg = parse_inbox_line(
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> ",
            0,
        ).unwrap();

        assert_eq!(msg.content, "");
    }

    #[test]
    fn test_parse_inbox_line_malformed() {
        assert!(parse_inbox_line("not a valid line", 0).is_none());
        assert!(parse_inbox_line("", 0).is_none());
        assert!(parse_inbox_line("[] 2026-03-18 abc", 0).is_none());
    }

    #[test]
    fn test_format_parse_roundtrip() {
        let line = format_inbox_line(
            "2026-03-18 22:15:32",
            "msg123",
            "bob",
            "987654321",
            "hello\nworld",
        );
        let msg = parse_inbox_line(&line, 0).unwrap();

        assert!(!msg.read);
        assert_eq!(msg.timestamp, "2026-03-18 22:15:32");
        assert_eq!(msg.message_id, "msg123");
        assert_eq!(msg.author_name, "bob");
        assert_eq!(msg.author_id, "987654321");
        assert_eq!(msg.content, "hello\nworld");
    }
}

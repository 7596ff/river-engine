//! Inbox file reading operations

use crate::inbox::format::{parse_inbox_line, InboxMessage};
use river_core::RiverResult;
use std::fs;
use std::path::Path;

/// Read all messages from an inbox file
pub fn read_all_messages(path: &Path) -> RiverResult<Vec<InboxMessage>> {
    let content = fs::read_to_string(path)?;

    Ok(content
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            match parse_inbox_line(line, i) {
                Some(msg) => Some(msg),
                None => {
                    if !line.trim().is_empty() {
                        tracing::warn!(
                            path = %path.display(),
                            line_number = i + 1,
                            "Malformed line in inbox file, skipping"
                        );
                    }
                    None
                }
            }
        })
        .collect())
}

/// Read only unread messages from an inbox file
pub fn read_unread_messages(path: &Path) -> RiverResult<Vec<InboxMessage>> {
    let messages = read_all_messages(path)?;
    Ok(messages.into_iter().filter(|m| !m.read).collect())
}

/// Check if an inbox file has any unread messages
pub fn has_unread_messages(path: &Path) -> RiverResult<bool> {
    let content = fs::read_to_string(path)?;
    Ok(content.lines().any(|line| line.starts_with("[ ] ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_test_file(path: &Path, lines: &[&str]) {
        let mut file = fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{}", line).unwrap();
        }
    }

    #[test]
    fn test_read_all_messages() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "[x] 2026-03-18 22:15:45 def456 <bob:456> world",
        ]);

        let messages = read_all_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(!messages[0].read);
        assert!(messages[1].read);
    }

    #[test]
    fn test_read_unread_messages() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "[x] 2026-03-18 22:15:45 def456 <bob:456> world",
            "[ ] 2026-03-18 22:16:00 ghi789 <alice:123> another",
        ]);

        let messages = read_unread_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message_id, "abc123");
        assert_eq!(messages[1].message_id, "ghi789");
    }

    #[test]
    fn test_has_unread_messages_true() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[x] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "[ ] 2026-03-18 22:15:45 def456 <bob:456> world",
        ]);

        assert!(has_unread_messages(&path).unwrap());
    }

    #[test]
    fn test_has_unread_messages_false() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[x] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "[x] 2026-03-18 22:15:45 def456 <bob:456> world",
        ]);

        assert!(!has_unread_messages(&path).unwrap());
    }

    #[test]
    fn test_read_empty_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[]);

        let messages = read_all_messages(&path).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_read_skips_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello",
            "this is not valid",
            "[ ] 2026-03-18 22:15:45 def456 <bob:456> world",
        ]);

        let messages = read_all_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_read_preserves_line_numbers() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_test_file(&path, &[
            "[ ] 2026-03-18 22:15:32 abc123 <alice:123> first",
            "",
            "[ ] 2026-03-18 22:15:45 def456 <bob:456> third",
        ]);

        let messages = read_all_messages(&path).unwrap();
        assert_eq!(messages[0].line_number, 0);
        assert_eq!(messages[1].line_number, 2);
    }
}

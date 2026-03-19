//! File-based message inbox
//!
//! Messages are stored as human-readable text files with one message per line.
//! Format: `[status] timestamp messageId <authorName:authorId> content`

pub mod format;
pub mod reader;
pub mod writer;

pub use format::{escape_content, unescape_content, format_inbox_line, parse_inbox_line, InboxMessage};
pub use reader::{read_all_messages, read_unread_messages, has_unread_messages};
pub use writer::{sanitize_name, build_discord_path, append_line};

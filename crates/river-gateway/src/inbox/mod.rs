//! File-based message inbox
//!
//! Messages are stored as human-readable text files with one message per line.
//! Format: `[status] timestamp messageId <authorName:authorId> content`

pub mod format;

pub use format::{escape_content, unescape_content, format_inbox_line, parse_inbox_line, InboxMessage};

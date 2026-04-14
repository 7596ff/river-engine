//! Load workspace data into river-context types.
//!
//! Reads moves, moments, and conversations from workspace directories
//! and converts them to `ChannelContext` for use with `build_context`.

use river_context::{Author, Channel, ChannelContext, ChatMessage, Moment, Move};
use std::path::Path;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Load a channel's context from workspace files.
///
/// Reads:
/// - `workspace/moves/{adapter}_{channel_id}.jsonl`
/// - `workspace/moments/{adapter}_{channel_id}.jsonl`
/// - `workspace/conversations/{adapter}_{channel_id}.txt`
pub async fn load_channel_context(
    workspace: &Path,
    channel: &Channel,
) -> Result<ChannelContext, LoadError> {
    let adapter = &channel.adapter;
    let channel_id = &channel.id;
    let file_stem = format!("{}_{}", adapter, channel_id);

    // Load moves
    let moves_path = workspace.join("moves").join(format!("{}.jsonl", file_stem));
    let moves = load_moves(&moves_path).await.unwrap_or_default();

    // Load moments
    let moments_path = workspace.join("moments").join(format!("{}.jsonl", file_stem));
    let moments = load_moments(&moments_path).await.unwrap_or_default();

    // Load messages from conversation file
    let conv_path = workspace.join("conversations").join(format!("{}.txt", file_stem));
    let messages = load_conversation(&conv_path).await.unwrap_or_default();

    // Load inbox items
    let inbox = crate::inbox::load_inbox_items(workspace, adapter, channel_id).await;

    Ok(ChannelContext {
        channel: channel.clone(),
        moments,
        moves,
        messages,
        embeddings: vec![], // Loaded separately via embed server
        inbox,
    })
}

/// Load multiple channels' contexts.
pub async fn load_channels(
    workspace: &Path,
    channels: &[Channel],
) -> Vec<ChannelContext> {
    let mut contexts = Vec::with_capacity(channels.len());
    for channel in channels {
        match load_channel_context(workspace, channel).await {
            Ok(ctx) => contexts.push(ctx),
            Err(e) => {
                tracing::warn!("Failed to load channel {}: {:?}", channel.id, e);
                // Push empty context so channel order is preserved
                contexts.push(ChannelContext {
                    channel: channel.clone(),
                    moments: vec![],
                    moves: vec![],
                    messages: vec![],
                    embeddings: vec![],
                    inbox: vec![],
                });
            }
        }
    }
    contexts
}

/// Load moves from a JSONL file.
async fn load_moves(path: &Path) -> Result<Vec<Move>, LoadError> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let file = fs::File::open(path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut moves = Vec::new();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<MoveEntry>(&line) {
            Ok(entry) => {
                moves.push(Move {
                    id: entry.id,
                    content: entry.content,
                    message_range: (entry.start_message_id, entry.end_message_id),
                });
            }
            Err(e) => {
                tracing::debug!("Skipping malformed move entry: {}", e);
            }
        }
    }

    Ok(moves)
}

/// Load moments from a JSONL file.
async fn load_moments(path: &Path) -> Result<Vec<Moment>, LoadError> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let file = fs::File::open(path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut moments = Vec::new();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<MomentEntry>(&line) {
            Ok(entry) => {
                moments.push(Moment {
                    id: entry.id,
                    content: entry.content,
                    move_range: (entry.start_move_id, entry.end_move_id),
                });
            }
            Err(e) => {
                tracing::debug!("Skipping malformed moment entry: {}", e);
            }
        }
    }

    Ok(moments)
}

/// Load messages from a conversation file.
///
/// Format:
/// ```text
/// ---
/// adapter: discord
/// channel_id: "chan_123"
/// channel_name: "general"
/// ---
/// [ ] 2026-04-01 00:00:00 msg1000 <alice:6221> hello
/// [>] 2026-04-01 00:02:00 msg1001 <River:999> hi there
/// [x] 2026-04-01 00:04:00 msg1002 <bob:5885> hey
/// ```
async fn load_conversation(path: &Path) -> Result<Vec<ChatMessage>, LoadError> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(path).await?;
    let mut messages = Vec::new();
    let mut in_frontmatter = false;
    let mut past_frontmatter = false;

    for line in content.lines() {
        // Handle YAML frontmatter
        if line == "---" {
            if !past_frontmatter {
                in_frontmatter = !in_frontmatter;
                if !in_frontmatter {
                    past_frontmatter = true;
                }
                continue;
            }
        }

        if in_frontmatter || !past_frontmatter {
            continue;
        }

        // Parse message line
        // Format: [status] timestamp msg_id <author:id> content
        if let Some(msg) = parse_message_line(line) {
            messages.push(msg);
        }
    }

    Ok(messages)
}

/// Parse a single message line.
///
/// Format: `[status] 2026-04-01 00:00:00 msg1000 <author:id> content`
/// Status: `[ ]` = unread, `[x]` = read, `[>]` = outgoing (bot)
fn parse_message_line(line: &str) -> Option<ChatMessage> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Check status marker
    let (is_bot, rest) = if line.starts_with("[>]") {
        (true, &line[3..])
    } else if line.starts_with("[ ]") || line.starts_with("[x]") {
        (false, &line[3..])
    } else {
        return None;
    };

    let rest = rest.trim();

    // Parse: date time msg_id <author:id> content
    let parts: Vec<&str> = rest.splitn(4, ' ').collect();
    if parts.len() < 4 {
        return None;
    }

    let date = parts[0];
    let time = parts[1];
    let msg_id = parts[2];
    let remainder = parts[3];

    // Parse <author:id> content
    let author_end = remainder.find('>')?;
    let author_part = &remainder[1..author_end]; // Skip '<'
    let content = remainder[author_end + 1..].trim();

    // Parse author:id
    let (author_name, author_id) = if let Some(colon_pos) = author_part.find(':') {
        (&author_part[..colon_pos], &author_part[colon_pos + 1..])
    } else {
        (author_part, "0")
    };

    // Generate snowflake-style ID from message ID
    // msg1000 → extract number, convert to timestamp-like ID
    let id = msg_id_to_snowflake(msg_id);

    Some(ChatMessage {
        id,
        timestamp: format!("{}T{}Z", date, time),
        author: Author {
            id: author_id.to_string(),
            name: author_name.to_string(),
            bot: is_bot,
        },
        content: content.to_string(),
    })
}

/// Convert a message ID like "msg1000" to a snowflake-style string.
fn msg_id_to_snowflake(msg_id: &str) -> String {
    // If already looks like a snowflake (all digits), use as-is
    if msg_id.chars().all(|c| c.is_ascii_digit()) {
        return msg_id.to_string();
    }

    // Extract number from "msg1000" format
    let num: u64 = msg_id
        .strip_prefix("msg")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Convert to fake snowflake (minutes from base * 60 * 1_000_000 in high bits)
    let minutes = num.saturating_sub(1000) * 2;
    let micros = minutes * 60 * 1_000_000;
    let snowflake: u128 = (micros as u128) << 64;
    snowflake.to_string()
}

/// JSONL entry for a move.
#[derive(Debug, serde::Deserialize)]
struct MoveEntry {
    id: String,
    content: String,
    start_message_id: String,
    end_message_id: String,
    #[allow(dead_code)]
    channel: Option<ChannelRef>,
    #[allow(dead_code)]
    created_at: Option<String>,
}

/// JSONL entry for a moment.
#[derive(Debug, serde::Deserialize)]
struct MomentEntry {
    id: String,
    content: String,
    start_move_id: String,
    end_move_id: String,
    #[allow(dead_code)]
    channel: Option<ChannelRef>,
    #[allow(dead_code)]
    created_at: Option<String>,
}

/// Channel reference in JSONL entries.
#[derive(Debug, serde::Deserialize)]
struct ChannelRef {
    #[allow(dead_code)]
    adapter: String,
    #[allow(dead_code)]
    id: String,
}

/// Error loading workspace data.
#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl From<serde_json::Error> for LoadError {
    fn from(e: serde_json::Error) -> Self {
        LoadError::Json(e)
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "IO error: {}", e),
            LoadError::Json(e) => write!(f, "JSON parse error: {}", e),
        }
    }
}

impl std::error::Error for LoadError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_line_unread() {
        let line = "[ ] 2026-04-01 00:00:00 msg1000 <alice:6221> hello world";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.author.name, "alice");
        assert_eq!(msg.author.id, "6221");
        assert!(!msg.author.bot);
        assert_eq!(msg.content, "hello world");
        assert_eq!(msg.timestamp, "2026-04-01T00:00:00Z");
    }

    #[test]
    fn test_parse_message_line_outgoing() {
        let line = "[>] 2026-04-01 00:02:00 msg1001 <River:999> hi there";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.author.name, "River");
        assert!(msg.author.bot);
        assert_eq!(msg.content, "hi there");
    }

    #[test]
    fn test_parse_message_line_read() {
        let line = "[x] 2026-04-01 00:04:00 msg1002 <bob:5885> hey";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.author.name, "bob");
        assert!(!msg.author.bot);
    }

    #[test]
    fn test_msg_id_to_snowflake() {
        // msg1000 should be base (0 minutes offset)
        let id = msg_id_to_snowflake("msg1000");
        assert_eq!(id, "0");

        // msg1001 should be 2 minutes = 120 seconds = 120_000_000 microseconds
        let id = msg_id_to_snowflake("msg1001");
        let expected: u128 = (2 * 60 * 1_000_000_u128) << 64;
        assert_eq!(id, expected.to_string());
    }

    #[test]
    fn test_parse_empty_line() {
        assert!(parse_message_line("").is_none());
        assert!(parse_message_line("   ").is_none());
    }

    #[test]
    fn test_parse_invalid_format() {
        assert!(parse_message_line("not a valid line").is_none());
        assert!(parse_message_line("[?] invalid status").is_none());
    }
}

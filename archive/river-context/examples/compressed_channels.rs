//! Parse compressed channel files (.moments.txt) and run build_context.
//!
//! These files contain:
//! - Moments [~] (arc-level summaries)
//! - Raw messages (last 20, uncompressed)
//!
//! Run with: cargo run --example compressed_channels

use river_context::{
    build_context, Author, Channel, ChannelContext, ChatMessage, ContextRequest, Moment,
};
use std::fs;
use std::path::Path;

const CONVERSATIONS_DIR: &str = "tests/fixtures/conversations";

/// Parse a timestamp like "2026-04-01 06:50:00" into microseconds for snowflake generation.
fn parse_timestamp_micros(ts: &str) -> u64 {
    // Extract hours, minutes from "2026-04-01 HH:MM:SS"
    let parts: Vec<&str> = ts.split_whitespace().collect();
    if parts.len() < 2 {
        return 0;
    }
    let time_parts: Vec<&str> = parts[1].split(':').collect();
    if time_parts.len() < 2 {
        return 0;
    }
    let hours: u64 = time_parts[0].parse().unwrap_or(0);
    let minutes: u64 = time_parts[1].parse().unwrap_or(0);
    let seconds: u64 = time_parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    // Convert to microseconds from midnight
    (hours * 3600 + minutes * 60 + seconds) * 1_000_000
}

/// Create a snowflake ID from microseconds.
fn make_snowflake(micros: u64) -> String {
    let snowflake: u128 = (micros as u128) << 64;
    snowflake.to_string()
}

/// Parse a message ID like "msg1205" into a number.
fn parse_msg_id(id: &str) -> u64 {
    id.strip_prefix("msg").and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Generate a snowflake for a message ID (assumes 2 minute intervals from msg1000).
fn msg_id_to_snowflake(msg_id: &str) -> String {
    let num = parse_msg_id(msg_id);
    let minutes = (num.saturating_sub(1000)) * 2;
    let micros = minutes * 60 * 1_000_000;
    make_snowflake(micros)
}

#[derive(Debug)]
struct ParsedChannel {
    channel: Channel,
    moments: Vec<Moment>,
    messages: Vec<ChatMessage>,
}

fn parse_compressed_channel(name: &str) -> ParsedChannel {
    let path = Path::new(CONVERSATIONS_DIR).join(format!("{}.moments.txt", name));
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));

    let mut lines = content.lines().peekable();

    // Skip YAML frontmatter
    let mut in_frontmatter = false;
    let mut channel_id = format!("chan_{}", name);
    let mut channel_name = name.to_string();

    while let Some(line) = lines.peek() {
        if *line == "---" {
            if in_frontmatter {
                lines.next(); // consume closing ---
                break;
            } else {
                in_frontmatter = true;
                lines.next();
                continue;
            }
        }
        if in_frontmatter {
            if let Some(rest) = line.strip_prefix("channel_id: ") {
                channel_id = rest.trim_matches('"').to_string();
            }
            if let Some(rest) = line.strip_prefix("channel_name: ") {
                channel_name = rest.trim_matches('"').to_string();
            }
        }
        lines.next();
    }

    let channel = Channel {
        adapter: "discord".into(),
        id: channel_id,
        name: Some(channel_name),
    };

    let mut moments = Vec::new();
    let mut messages = Vec::new();
    let mut in_raw_section = false;

    for line in lines {
        let line = line.trim();

        // Section markers
        if line.starts_with("# Moments") {
            in_raw_section = false;
            continue;
        }
        if line.starts_with("# Raw messages") {
            in_raw_section = true;
            continue;
        }

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse [~] moment lines
        if line.starts_with("[~]") {
            let rest = line.strip_prefix("[~]").unwrap().trim();
            // Format: msg1000-msg1133 Summary text...
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() >= 2 {
                let range = parts[0];
                let content = parts[1];

                // Parse range like "msg1000-msg1133"
                let range_parts: Vec<&str> = range.split('-').collect();
                if range_parts.len() == 2 {
                    let start_id = range_parts[0];
                    let end_id = range_parts[1];

                    moments.push(Moment {
                        id: msg_id_to_snowflake(end_id), // Use end of range for timestamp
                        content: content.to_string(),
                        move_range: (start_id.to_string(), end_id.to_string()),
                    });
                }
            }
            continue;
        }

        // Parse raw messages
        // Format: [ ] 2026-04-01 06:50:00 msg1205 <dan:7668> content
        // or:     [>] 2026-04-01 06:50:00 msg1205 <River:999> content
        if in_raw_section && (line.starts_with("[ ]") || line.starts_with("[>]") || line.starts_with("[x]")) {
            let is_bot = line.starts_with("[>]");
            let rest = &line[3..].trim();

            // Parse: 2026-04-01 06:50:00 msg1205 <name:id> content
            let parts: Vec<&str> = rest.splitn(4, ' ').collect();
            if parts.len() >= 4 {
                let date = parts[0];
                let time = parts[1];
                let _msg_id = parts[2];
                let rest = parts[3];

                // Parse <name:id> content
                if let Some(end_bracket) = rest.find('>') {
                    let author_part = &rest[1..end_bracket]; // Remove < and >
                    let content = rest[end_bracket + 1..].trim();

                    let author_parts: Vec<&str> = author_part.split(':').collect();
                    let author_name = author_parts[0];
                    let author_id = author_parts.get(1).unwrap_or(&"0");

                    let timestamp = format!("{} {}", date, time);
                    let micros = parse_timestamp_micros(&timestamp);

                    messages.push(ChatMessage {
                        id: make_snowflake(micros),
                        timestamp: format!("{}T{}Z", date, time),
                        author: Author {
                            id: format!("user_{}", author_id),
                            name: author_name.to_string(),
                            bot: is_bot,
                        },
                        content: content.to_string(),
                    });
                }
            }
        }
    }

    ParsedChannel {
        channel,
        moments,
        messages,
    }
}

fn main() {
    let channel_names = ["development", "philosophy", "random", "memes", "politics"];

    println!("Parsing compressed channel files...\n");

    let channels: Vec<ChannelContext> = channel_names
        .iter()
        .map(|name| {
            let parsed = parse_compressed_channel(name);
            println!(
                "  #{}: {} moments, {} raw messages",
                name,
                parsed.moments.len(),
                parsed.messages.len()
            );
            ChannelContext {
                channel: parsed.channel,
                moments: parsed.moments,
                moves: vec![],
                messages: parsed.messages,
                embeddings: vec![],
                inbox: vec![],
            }
        })
        .collect();

    println!("\n{}", "=".repeat(60));
    println!("Running build_context with #development as current channel");
    println!("{}\n", "=".repeat(60));

    let request = ContextRequest {
        channels,
        flashes: vec![],
        history: vec![],
        max_tokens: 50000,
        now: "2026-04-01T12:00:00Z".into(),
    };

    match build_context(request) {
        Ok(response) => {
            println!("Estimated tokens: {}", response.estimated_tokens);
            println!("Messages generated: {}\n", response.messages.len());

            // Write full output to file
            let output_path = Path::new(CONVERSATIONS_DIR).join("compressed_output.txt");
            let mut output = String::new();

            for (i, msg) in response.messages.iter().enumerate() {
                let content = msg.content.as_deref().unwrap_or("[no content]");
                let tool_info = if msg.tool_calls.is_some() {
                    " [tool_calls]"
                } else if msg.tool_call_id.is_some() {
                    " [tool_result]"
                } else {
                    ""
                };

                output.push_str(&format!(
                    "--- Message {} [{}]{} ---\n{}\n\n",
                    i + 1,
                    msg.role,
                    tool_info,
                    content
                ));

                // Preview in terminal
                let preview: String = content.chars().take(200).collect();
                let truncated = if content.len() > 200 { "..." } else { "" };
                println!(
                    "{:>3}. [{:>9}]{} {}{}",
                    i + 1,
                    msg.role,
                    tool_info,
                    preview.replace('\n', "\\n"),
                    truncated
                );
            }

            fs::write(&output_path, &output).expect("Failed to write output");
            println!("\nFull output written to: {}", output_path.display());
        }
        Err(e) => {
            eprintln!("Error: {:?}", e);
        }
    }
}

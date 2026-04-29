//! Parse channel text files and run build_context on them.
//!
//! Run with: cargo run --example five_channels

use river_context::{
    build_context, Author, Channel, ChannelContext, ChatMessage, ContextRequest,
};
use std::fs;
use std::path::Path;

const CHANNELS_DIR: &str = "tests/fixtures/channels";

fn parse_channel(name: &str) -> ChannelContext {
    let path = Path::new(CHANNELS_DIR).join(format!("{}.txt", name));
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));

    let mut messages = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if line.is_empty() {
            continue;
        }

        let (author, bot, text) = if let Some(rest) = line.strip_prefix('*') {
            // Bot message: *River: content
            let (name, content) = rest.split_once(": ").unwrap_or((rest, ""));
            (name.to_string(), true, content.to_string())
        } else {
            // Human message: alice: content
            let (name, content) = line.split_once(": ").unwrap_or((line, ""));
            (name.to_string(), false, content.to_string())
        };

        // Generate snowflake ID from line number (simulate time progression)
        let minutes = (i as u64) * 2; // 2 minutes apart
        let micros = minutes * 60 * 1_000_000;
        let snowflake: u128 = (micros as u128) << 64;

        messages.push(ChatMessage {
            id: snowflake.to_string(),
            timestamp: format!("2026-04-01T{:02}:{:02}:00Z", (minutes / 60) % 24, minutes % 60),
            author: Author {
                id: format!("user_{}", author.to_lowercase()),
                name: author,
                bot,
            },
            content: text,
        });
    }

    ChannelContext {
        channel: Channel {
            adapter: "discord".into(),
            id: format!("chan_{}", name),
            name: Some(name.into()),
        },
        moments: vec![],
        moves: vec![],
        messages,
        embeddings: vec![],
        inbox: vec![],
    }
}

fn main() {
    let channel_names = ["development", "philosophy", "random", "memes", "politics"];

    println!("Parsing channels...\n");

    let channels: Vec<ChannelContext> = channel_names
        .iter()
        .map(|name| {
            let ctx = parse_channel(name);
            println!("  #{}: {} messages", name, ctx.messages.len());
            ctx
        })
        .collect();

    println!("\n{}", "=".repeat(60));
    println!("Running build_context with #development as current channel");
    println!("{}\n", "=".repeat(60));

    let request = ContextRequest {
        channels,
        flashes: vec![],
        history: vec![],
        max_tokens: 10000,
        now: "2026-04-01T12:00:00Z".into(),
    };

    match build_context(request) {
        Ok(response) => {
            println!("Estimated tokens: {}", response.estimated_tokens);
            println!("Messages generated: {}\n", response.messages.len());

            // Write full output to file
            let output_path = Path::new(CHANNELS_DIR).join("output.txt");
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

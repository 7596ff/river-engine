//! Formatting workspace types to OpenAI messages.

use river_adapter::Channel;

use crate::openai::OpenAIMessage;
use crate::workspace::{ChatMessage, Embedding, Flash, Moment, Move};

/// Format a moment as a system message.
pub fn format_moment(m: &Moment, channel: &Channel) -> OpenAIMessage {
    let channel_name = channel.name.as_deref().unwrap_or(&channel.id);
    OpenAIMessage::system(format!(
        "[Moment: {}] {} (moves {}-{})",
        channel_name, m.content, m.move_range.0, m.move_range.1
    ))
}

/// Format a move as a system message.
pub fn format_move(m: &Move, channel: &Channel) -> OpenAIMessage {
    let channel_name = channel.name.as_deref().unwrap_or(&channel.id);
    OpenAIMessage::system(format!(
        "[Move: {}] {} (messages {}-{})",
        channel_name, m.content, m.message_range.0, m.message_range.1
    ))
}

/// Format a flash as a system message.
pub fn format_flash(f: &Flash) -> OpenAIMessage {
    OpenAIMessage::system(format!("[Flash from {}] {}", f.from, f.content))
}

/// Format an embedding as a system message.
pub fn format_embedding(e: &Embedding) -> OpenAIMessage {
    OpenAIMessage::system(format!("[Reference: {}]\n{}", e.source, e.content))
}

/// Format chat messages as a user message.
pub fn format_chat_messages(msgs: &[ChatMessage], channel: &Channel) -> OpenAIMessage {
    let channel_name = channel.name.as_deref().unwrap_or(&channel.id);
    let formatted = msgs
        .iter()
        .map(|m| format!("[{}] <{}> {}", m.timestamp, m.author.name, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    OpenAIMessage::user(format!("[Chat: {}]\n{}", channel_name, formatted))
}

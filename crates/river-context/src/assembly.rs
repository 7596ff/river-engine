//! Context assembly logic.

use crate::format::{format_chat_messages, format_embedding, format_flash, format_moment, format_move};
use crate::openai::OpenAIMessage;
use crate::request::{ChannelContext, ContextRequest};
use crate::response::{ContextError, ContextResponse};
use crate::tokens::estimate_total_tokens;

/// Build context from request.
pub fn build_context(request: ContextRequest) -> Result<ContextResponse, ContextError> {
    if request.channels.is_empty() {
        return Err(ContextError::EmptyChannels);
    }

    let mut messages = Vec::new();
    let now = &request.now;

    // Filter flashes by TTL
    let valid_flashes: Vec<_> = request
        .flashes
        .into_iter()
        .filter(|f| f.expires_at > *now)
        .collect();

    // Process other channels (not current, not last): moments + moves only
    if request.channels.len() > 2 {
        for channel_ctx in &request.channels[2..] {
            add_channel_summary(&mut messages, channel_ctx);
        }
    }

    // Process last channel (index 1 if exists): moments + moves + embeddings
    if request.channels.len() > 1 {
        let last_ctx = &request.channels[1];
        add_channel_summary(&mut messages, last_ctx);
        add_channel_embeddings(&mut messages, last_ctx, now);
    }

    // Add LLM history block
    messages.extend(request.history);

    // Process current channel (index 0): moments + moves + messages + embeddings
    let current_ctx = &request.channels[0];
    add_channel_summary(&mut messages, current_ctx);
    add_channel_embeddings(&mut messages, current_ctx, now);

    // Add chat messages for current channel
    if !current_ctx.messages.is_empty() {
        messages.push(format_chat_messages(&current_ctx.messages, &current_ctx.channel));
    }

    // Intersperse flashes (add near end for high priority)
    for flash in valid_flashes {
        messages.push(format_flash(&flash));
    }

    // Estimate tokens
    let estimated_tokens = estimate_total_tokens(&messages);

    if estimated_tokens > request.max_tokens {
        return Err(ContextError::OverBudget {
            estimated: estimated_tokens,
            limit: request.max_tokens,
        });
    }

    Ok(ContextResponse {
        messages,
        estimated_tokens,
    })
}

fn add_channel_summary(messages: &mut Vec<OpenAIMessage>, ctx: &ChannelContext) {
    for moment in &ctx.moments {
        messages.push(format_moment(moment, &ctx.channel));
    }
    for mv in &ctx.moves {
        messages.push(format_move(mv, &ctx.channel));
    }
}

fn add_channel_embeddings(messages: &mut Vec<OpenAIMessage>, ctx: &ChannelContext, now: &str) {
    for embedding in &ctx.embeddings {
        if embedding.expires_at.as_str() > now {
            messages.push(format_embedding(embedding));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_adapter::Channel;

    #[test]
    fn test_build_context_empty_channels() {
        let request = ContextRequest {
            channels: vec![],
            flashes: vec![],
            history: vec![],
            max_tokens: 1000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request);
        assert!(matches!(result, Err(ContextError::EmptyChannels)));
    }

    #[test]
    fn test_build_context_single_channel() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "123".into(),
                    name: Some("general".into()),
                },
                moments: vec![],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 1000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();
        assert_eq!(result.messages.len(), 0);
    }
}

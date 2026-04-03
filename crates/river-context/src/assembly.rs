//! Context assembly logic.

use chrono::{DateTime, Utc};

use crate::format::{
    format_chat_messages, format_embedding, format_flash, format_moment, format_move,
};
use crate::id::extract_timestamp;
use crate::openai::OpenAIMessage;
use crate::request::{ChannelContext, ContextRequest};
use crate::response::{ContextError, ContextResponse};
use crate::tokens::estimate_total_tokens;
use crate::workspace::Flash;

/// Item in the timeline with its timestamp for sorting.
#[derive(Debug)]
struct TimelineItem {
    /// Timestamp in microseconds (from snowflake ID).
    timestamp: u64,
    /// The formatted message.
    message: OpenAIMessage,
}

impl TimelineItem {
    fn new(id: &str, message: OpenAIMessage) -> Self {
        let timestamp = extract_timestamp(id).unwrap_or(0);
        Self { timestamp, message }
    }
}

/// Check if an item has expired based on TTL.
/// Uses chrono for robust timestamp comparison.
fn is_expired(expires_at: &str, now: &DateTime<Utc>) -> bool {
    match expires_at.parse::<DateTime<Utc>>() {
        Ok(expiry) => expiry <= *now,
        Err(_) => {
            // Fallback to string comparison for ISO8601 UTC strings
            expires_at <= now.to_rfc3339().as_str()
        }
    }
}

/// Parse the current time string into a DateTime.
fn parse_now(now: &str) -> DateTime<Utc> {
    now.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now())
}

/// Build context from request.
pub fn build_context(request: ContextRequest) -> Result<ContextResponse, ContextError> {
    if request.channels.is_empty() {
        return Err(ContextError::EmptyChannels);
    }

    let now_dt = parse_now(&request.now);

    // Collect all timeline items with timestamps
    let mut timeline: Vec<TimelineItem> = Vec::new();

    // Filter flashes by TTL and collect with timestamps
    let valid_flashes: Vec<&Flash> = request
        .flashes
        .iter()
        .filter(|f| !is_expired(&f.expires_at, &now_dt))
        .collect();

    for flash in &valid_flashes {
        timeline.push(TimelineItem::new(&flash.id, format_flash(flash)));
    }

    // Process other channels (not current, not last): moments + moves only
    if request.channels.len() > 2 {
        for channel_ctx in &request.channels[2..] {
            collect_channel_summary(&mut timeline, channel_ctx);
        }
    }

    // Process last channel (index 1 if exists): moments + moves + embeddings
    if request.channels.len() > 1 {
        let last_ctx = &request.channels[1];
        collect_channel_summary(&mut timeline, last_ctx);
        collect_channel_embeddings(&mut timeline, last_ctx, &now_dt);
    }

    // Process current channel (index 0): moments + moves + embeddings
    let current_ctx = &request.channels[0];
    collect_channel_summary(&mut timeline, current_ctx);
    collect_channel_embeddings(&mut timeline, current_ctx, &now_dt);

    // Sort timeline by timestamp
    timeline.sort_by_key(|item| item.timestamp);

    // Build final message list
    let mut messages: Vec<OpenAIMessage> =
        timeline.into_iter().map(|item| item.message).collect();

    // Add LLM history block (not sorted, keeps its position)
    messages.extend(request.history);

    // Add chat messages for current channel (at the end, most recent)
    if !current_ctx.messages.is_empty() {
        messages.push(format_chat_messages(
            &current_ctx.messages,
            &current_ctx.channel,
        ));
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

fn collect_channel_summary(timeline: &mut Vec<TimelineItem>, ctx: &ChannelContext) {
    for moment in &ctx.moments {
        timeline.push(TimelineItem::new(
            &moment.id,
            format_moment(moment, &ctx.channel),
        ));
    }
    for mv in &ctx.moves {
        timeline.push(TimelineItem::new(&mv.id, format_move(mv, &ctx.channel)));
    }
}

fn collect_channel_embeddings(
    timeline: &mut Vec<TimelineItem>,
    ctx: &ChannelContext,
    now: &DateTime<Utc>,
) {
    for embedding in &ctx.embeddings {
        if !is_expired(&embedding.expires_at, now) {
            timeline.push(TimelineItem::new(
                &embedding.id,
                format_embedding(embedding),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_protocol::Channel;

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

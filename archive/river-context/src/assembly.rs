//! Context assembly logic.

use chrono::{DateTime, Utc};
use tracing;

use crate::format::{
    format_chat_messages, format_embedding, format_flash, format_inbox_item, format_moment,
    format_move,
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
        let timestamp = extract_timestamp(id)
            .map_err(|e| {
                tracing::warn!("{}", e);
                e
            })
            .unwrap_or(0);
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
/// Returns error if parsing fails - invalid timestamps should not be silently accepted.
fn parse_now(now: &str) -> Result<DateTime<Utc>, ContextError> {
    now.parse::<DateTime<Utc>>()
        .map_err(|e| ContextError::TimeParseError(format!("{}: {}", now, e)))
}

/// Build context from request.
pub fn build_context(request: ContextRequest) -> Result<ContextResponse, ContextError> {
    if request.channels.is_empty() {
        return Err(ContextError::EmptyChannels);
    }

    let now_dt = parse_now(&request.now)?;

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

    // Process current channel (index 0): moments + moves + embeddings + inbox
    let current_ctx = &request.channels[0];
    collect_channel_summary(&mut timeline, current_ctx);
    collect_channel_embeddings(&mut timeline, current_ctx, &now_dt);
    collect_channel_inbox(&mut timeline, current_ctx);

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

fn collect_channel_inbox(timeline: &mut Vec<TimelineItem>, ctx: &ChannelContext) {
    for item in &ctx.inbox {
        timeline.push(TimelineItem::new(&item.id, format_inbox_item(item)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{ChatMessage, Embedding, InboxItem, Moment, Move};
    use river_protocol::{Author, Channel};

    /// Create a snowflake ID with a specific timestamp (microseconds).
    fn make_id(timestamp_micros: u64) -> String {
        let snowflake: u128 = (timestamp_micros as u128) << 64;
        snowflake.to_string()
    }

    fn test_channel(name: &str) -> Channel {
        Channel {
            adapter: "discord".into(),
            id: format!("chan_{}", name),
            name: Some(name.into()),
        }
    }

    fn test_moment(id: &str, content: &str) -> Moment {
        Moment {
            id: id.into(),
            content: content.into(),
            move_range: ("0".into(), "0".into()),
        }
    }

    fn test_move(id: &str, content: &str) -> Move {
        Move {
            id: id.into(),
            content: content.into(),
            message_range: ("0".into(), "0".into()),
        }
    }

    fn test_flash(id: &str, from: &str, content: &str, expires_at: &str) -> Flash {
        Flash {
            id: id.into(),
            from: from.into(),
            content: content.into(),
            expires_at: expires_at.into(),
        }
    }

    fn test_embedding(id: &str, content: &str, expires_at: &str) -> Embedding {
        Embedding {
            id: id.into(),
            content: content.into(),
            source: "test".into(),
            expires_at: expires_at.into(),
        }
    }

    fn test_message(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            timestamp: "2026-04-01T12:00:00Z".into(),
            author: Author {
                id: "user1".into(),
                name: "TestUser".into(),
                bot: false,
            },
            content: content.into(),
        }
    }

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
                inbox: vec![],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 1000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();
        assert_eq!(result.messages.len(), 0);
    }

    #[test]
    fn test_build_context_multi_channel() {
        let request = ContextRequest {
            channels: vec![
                ChannelContext {
                    channel: test_channel("current"),
                    moments: vec![test_moment(&make_id(3000), "Current moment")],
                    moves: vec![],
                    messages: vec![test_message("msg1", "Hello")],
                    embeddings: vec![],
                inbox: vec![],
                },
                ChannelContext {
                    channel: test_channel("last"),
                    moments: vec![test_moment(&make_id(2000), "Last moment")],
                    moves: vec![],
                    messages: vec![],
                    embeddings: vec![],
                inbox: vec![],
                },
                ChannelContext {
                    channel: test_channel("other"),
                    moments: vec![test_moment(&make_id(1000), "Other moment")],
                    moves: vec![],
                    messages: vec![],
                    embeddings: vec![],
                inbox: vec![],
                },
            ],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        // Should have 3 moments + 1 chat message
        assert_eq!(result.messages.len(), 4);

        // Verify ordering by timestamp (earliest first)
        let content_0 = result.messages[0].content.as_ref().unwrap();
        let content_1 = result.messages[1].content.as_ref().unwrap();
        let content_2 = result.messages[2].content.as_ref().unwrap();

        assert!(content_0.contains("Other moment")); // timestamp 1000
        assert!(content_1.contains("Last moment")); // timestamp 2000
        assert!(content_2.contains("Current moment")); // timestamp 3000
    }

    #[test]
    fn test_flashes_interspersed_by_timestamp() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![
                    test_moment(&make_id(1000), "Early moment"),
                    test_moment(&make_id(3000), "Late moment"),
                ],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![test_flash(
                &make_id(2000),
                "worker1",
                "Middle flash",
                "2026-04-02T00:00:00Z",
            )],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        assert_eq!(result.messages.len(), 3);

        // Flash should be between the two moments
        let content_0 = result.messages[0].content.as_ref().unwrap();
        let content_1 = result.messages[1].content.as_ref().unwrap();
        let content_2 = result.messages[2].content.as_ref().unwrap();

        assert!(content_0.contains("Early moment")); // timestamp 1000
        assert!(content_1.contains("Middle flash")); // timestamp 2000
        assert!(content_2.contains("Late moment")); // timestamp 3000
    }

    #[test]
    fn test_flash_ttl_filtering() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![
                test_flash(
                    &make_id(1000),
                    "worker1",
                    "Expired flash",
                    "2026-04-01T11:00:00Z",
                ),
                test_flash(
                    &make_id(2000),
                    "worker2",
                    "Valid flash",
                    "2026-04-01T13:00:00Z",
                ),
            ],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        // Only the valid flash should be included
        assert_eq!(result.messages.len(), 1);
        assert!(result.messages[0]
            .content
            .as_ref()
            .unwrap()
            .contains("Valid flash"));
    }

    #[test]
    fn test_embeddings_interspersed_by_timestamp() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![
                    test_moment(&make_id(1000), "Moment 1"),
                    test_moment(&make_id(3000), "Moment 2"),
                ],
                moves: vec![],
                messages: vec![],
                embeddings: vec![test_embedding(
                    &make_id(2000),
                    "Embedding content",
                    "2026-04-02T00:00:00Z",
                )],
                inbox: vec![],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        assert_eq!(result.messages.len(), 3);

        // Embedding should be between the two moments
        let content_0 = result.messages[0].content.as_ref().unwrap();
        let content_1 = result.messages[1].content.as_ref().unwrap();
        let content_2 = result.messages[2].content.as_ref().unwrap();

        assert!(content_0.contains("Moment 1")); // timestamp 1000
        assert!(content_1.contains("Embedding content")); // timestamp 2000
        assert!(content_2.contains("Moment 2")); // timestamp 3000
    }

    #[test]
    fn test_embedding_ttl_filtering() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![],
                moves: vec![],
                messages: vec![],
                embeddings: vec![
                    test_embedding(
                        &make_id(1000),
                        "Expired embedding",
                        "2026-04-01T11:00:00Z",
                    ),
                    test_embedding(&make_id(2000), "Valid embedding", "2026-04-01T13:00:00Z"),
                ],
                inbox: vec![],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        // Only the valid embedding should be included
        assert_eq!(result.messages.len(), 1);
        assert!(result.messages[0]
            .content
            .as_ref()
            .unwrap()
            .contains("Valid embedding"));
    }

    #[test]
    fn test_inbox_items_interspersed_by_timestamp() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![
                    test_moment(&make_id(1000), "Moment 1"),
                    test_moment(&make_id(3000), "Moment 2"),
                ],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
                inbox: vec![InboxItem {
                    id: make_id(2000),
                    timestamp: "2026-04-01T07:28:00Z".into(),
                    tool: "read_channel".into(),
                    channel_adapter: "discord".into(),
                    channel_id: "main".into(),
                    summary: "msg1150-msg1200".into(),
                }],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        assert_eq!(result.messages.len(), 3);

        // Inbox item should be between the two moments
        let content_0 = result.messages[0].content.as_ref().unwrap();
        let content_1 = result.messages[1].content.as_ref().unwrap();
        let content_2 = result.messages[2].content.as_ref().unwrap();

        assert!(content_0.contains("Moment 1")); // timestamp 1000
        assert!(content_1.contains("[inbox]")); // timestamp 2000
        assert!(content_2.contains("Moment 2")); // timestamp 3000
    }

    #[test]
    fn test_over_budget_error() {
        let long_content = "x".repeat(10000);
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![test_moment(&make_id(1000), &long_content)],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 100, // Very low limit
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request);

        match result {
            Err(ContextError::OverBudget { estimated, limit }) => {
                assert!(estimated > limit);
                assert_eq!(limit, 100);
            }
            _ => panic!("Expected OverBudget error"),
        }
    }

    #[test]
    fn test_history_placement() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![test_moment(&make_id(1000), "A moment")],
                moves: vec![],
                messages: vec![test_message("msg1", "User message")],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![],
            history: vec![
                OpenAIMessage::user("Previous user message"),
                OpenAIMessage::assistant("Previous assistant response"),
            ],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        // Order: sorted timeline items, then history, then chat messages
        assert_eq!(result.messages.len(), 4);

        // Moment comes first (sorted timeline)
        assert!(result.messages[0]
            .content
            .as_ref()
            .unwrap()
            .contains("A moment"));
        // History comes after timeline
        assert!(result.messages[1]
            .content
            .as_ref()
            .unwrap()
            .contains("Previous user message"));
        assert!(result.messages[2]
            .content
            .as_ref()
            .unwrap()
            .contains("Previous assistant response"));
        // Chat messages come last
        assert!(result.messages[3]
            .content
            .as_ref()
            .unwrap()
            .contains("User message"));
    }

    #[test]
    fn test_moves_sorted_by_timestamp() {
        let request = ContextRequest {
            channels: vec![ChannelContext {
                channel: test_channel("main"),
                moments: vec![],
                moves: vec![
                    test_move(&make_id(2000), "Move B"),
                    test_move(&make_id(1000), "Move A"),
                    test_move(&make_id(3000), "Move C"),
                ],
                messages: vec![],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        };

        let result = build_context(request).unwrap();

        assert_eq!(result.messages.len(), 3);

        // Verify ordering by timestamp
        assert!(result.messages[0]
            .content
            .as_ref()
            .unwrap()
            .contains("Move A"));
        assert!(result.messages[1]
            .content
            .as_ref()
            .unwrap()
            .contains("Move B"));
        assert!(result.messages[2]
            .content
            .as_ref()
            .unwrap()
            .contains("Move C"));
    }
}

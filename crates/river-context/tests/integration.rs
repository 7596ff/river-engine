//! Integration tests for river-context.

use river_context::{
    build_context, ChannelContext, ChatMessage, ContextRequest, Embedding, Flash, Moment, Move,
    OpenAIMessage,
};
use river_protocol::{Author, Channel};

/// Create a snowflake ID with a specific timestamp (microseconds).
fn make_id(timestamp_micros: u64) -> String {
    let snowflake: u128 = (timestamp_micros as u128) << 64;
    snowflake.to_string()
}

#[test]
fn test_full_context_assembly() {
    // Simulate a realistic scenario with multiple channels, flashes, embeddings
    let request = ContextRequest {
        channels: vec![
            // Current channel (index 0)
            ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "current_123".into(),
                    name: Some("dev-chat".into()),
                },
                moments: vec![Moment {
                    id: make_id(1_000_000),
                    content: "Team discussed deployment strategy".into(),
                    move_range: ("m1".into(), "m10".into()),
                }],
                moves: vec![Move {
                    id: make_id(2_000_000),
                    content: "Reviewed CI pipeline changes".into(),
                    message_range: ("msg1".into(), "msg20".into()),
                }],
                messages: vec![
                    ChatMessage {
                        id: "msg21".into(),
                        timestamp: "2026-04-01T12:00:00Z".into(),
                        author: Author {
                            id: "user1".into(),
                            name: "Alice".into(),
                            bot: false,
                        },
                        content: "Can you help with the API?".into(),
                    },
                    ChatMessage {
                        id: "msg22".into(),
                        timestamp: "2026-04-01T12:01:00Z".into(),
                        author: Author {
                            id: "user2".into(),
                            name: "Bob".into(),
                            bot: false,
                        },
                        content: "Sure, what do you need?".into(),
                    },
                ],
                embeddings: vec![Embedding {
                    id: make_id(1_500_000),
                    content: "API documentation for /users endpoint".into(),
                    source: "docs/api.md:15-42".into(),
                    expires_at: "2026-04-01T18:00:00Z".into(),
                }],
            },
            // Last active channel (index 1)
            ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "last_456".into(),
                    name: Some("general".into()),
                },
                moments: vec![Moment {
                    id: make_id(500_000),
                    content: "Standup meeting notes".into(),
                    move_range: ("m0".into(), "m5".into()),
                }],
                moves: vec![],
                messages: vec![],
                embeddings: vec![Embedding {
                    id: make_id(600_000),
                    content: "Project timeline".into(),
                    source: "notes/timeline.md".into(),
                    expires_at: "2026-04-01T18:00:00Z".into(),
                }],
            },
        ],
        flashes: vec![
            // Expired flash (should be filtered)
            Flash {
                id: make_id(100_000),
                from: "worker-old".into(),
                content: "This is expired".into(),
                expires_at: "2026-04-01T10:00:00Z".into(),
            },
            // Valid flash (should be included)
            Flash {
                id: make_id(1_200_000),
                from: "worker-alert".into(),
                content: "Build succeeded".into(),
                expires_at: "2026-04-01T18:00:00Z".into(),
            },
        ],
        history: vec![
            OpenAIMessage::user("What's the status?"),
            OpenAIMessage::assistant("Everything is running smoothly."),
        ],
        max_tokens: 50000,
        now: "2026-04-01T12:00:00Z".into(),
    };

    let result = build_context(request).unwrap();

    // Verify we got messages
    assert!(!result.messages.is_empty());

    // Verify token estimation is reasonable
    assert!(result.estimated_tokens > 0);
    assert!(result.estimated_tokens < 50000);

    // Verify the order:
    // 1. Timeline items sorted by timestamp
    // 2. History
    // 3. Current channel chat messages

    // Find the chat message (last item)
    let last_msg = result.messages.last().unwrap();
    assert_eq!(last_msg.role, "user");
    assert!(last_msg
        .content
        .as_ref()
        .unwrap()
        .contains("[Chat: dev-chat]"));

    // Find history messages (before chat)
    let history_start = result.messages.len() - 3; // 2 history + 1 chat
    assert_eq!(result.messages[history_start].role, "user");
    assert!(result.messages[history_start]
        .content
        .as_ref()
        .unwrap()
        .contains("What's the status?"));

    // Verify expired flash is not included
    let all_content: String = result
        .messages
        .iter()
        .filter_map(|m| m.content.as_ref())
        .cloned()
        .collect();

    assert!(!all_content.contains("This is expired"));
    assert!(all_content.contains("Build succeeded"));
}

#[test]
fn test_context_with_default_request() {
    // Using Default trait for cleaner test setup
    let mut request = ContextRequest::default();
    request.channels.push(ChannelContext {
        channel: Channel {
            adapter: "test".into(),
            id: "1".into(),
            name: Some("test-channel".into()),
        },
        ..Default::default()
    });
    request.max_tokens = 10000;
    request.now = "2026-04-01T12:00:00Z".into();

    let result = build_context(request).unwrap();

    // Should succeed with empty content
    assert_eq!(result.messages.len(), 0);
    assert_eq!(result.estimated_tokens, 0);
}

#[test]
fn test_timestamp_ordering_across_channels() {
    // Test that items from different channels are properly ordered by timestamp
    let request = ContextRequest {
        channels: vec![
            ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "chan1".into(),
                    name: Some("channel-1".into()),
                },
                moments: vec![Moment {
                    id: make_id(3_000_000), // Third
                    content: "Third item from channel 1".into(),
                    move_range: ("a".into(), "b".into()),
                }],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
            },
            ChannelContext {
                channel: Channel {
                    adapter: "discord".into(),
                    id: "chan2".into(),
                    name: Some("channel-2".into()),
                },
                moments: vec![Moment {
                    id: make_id(1_000_000), // First
                    content: "First item from channel 2".into(),
                    move_range: ("c".into(), "d".into()),
                }],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
            },
        ],
        flashes: vec![Flash {
            id: make_id(2_000_000), // Second
            from: "worker".into(),
            content: "Second item (flash)".into(),
            expires_at: "2026-04-02T00:00:00Z".into(),
        }],
        history: vec![],
        max_tokens: 10000,
        now: "2026-04-01T12:00:00Z".into(),
    };

    let result = build_context(request).unwrap();

    assert_eq!(result.messages.len(), 3);

    // Verify ordering
    assert!(result.messages[0]
        .content
        .as_ref()
        .unwrap()
        .contains("First item from channel 2"));
    assert!(result.messages[1]
        .content
        .as_ref()
        .unwrap()
        .contains("Second item (flash)"));
    assert!(result.messages[2]
        .content
        .as_ref()
        .unwrap()
        .contains("Third item from channel 1"));
}

#[test]
fn test_extract_timestamp_function() {
    // Test the public extract_timestamp function
    use river_context::extract_timestamp;

    // Create a snowflake with known timestamp
    let timestamp_micros: u64 = 1_000_000;
    let snowflake: u128 = (timestamp_micros as u128) << 64;
    let id = snowflake.to_string();

    let extracted = extract_timestamp(&id).unwrap();
    assert_eq!(extracted, timestamp_micros);
}

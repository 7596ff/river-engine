//! CLI to test build_context with various scenarios.
//!
//! Run with: cargo run --example test_build_context

use river_context::{
    build_context, Author, Channel, ChannelContext, ChatMessage, ContextRequest, Embedding, Flash,
    Moment, Move, OpenAIMessage,
};

/// Create a snowflake ID with a specific timestamp (microseconds since epoch).
/// The timestamp encodes into the high 64 bits of a 128-bit snowflake.
fn make_id(timestamp_micros: u64) -> String {
    let snowflake: u128 = (timestamp_micros as u128) << 64;
    snowflake.to_string()
}

fn channel(name: &str) -> Channel {
    Channel {
        adapter: "discord".into(),
        id: format!("chan_{}", name),
        name: Some(name.into()),
    }
}

fn moment(id: &str, content: &str) -> Moment {
    Moment {
        id: id.into(),
        content: content.into(),
        move_range: ("0".into(), "0".into()),
    }
}

fn mv(id: &str, content: &str) -> Move {
    Move {
        id: id.into(),
        content: content.into(),
        message_range: ("0".into(), "0".into()),
    }
}

fn flash(id: &str, from: &str, content: &str) -> Flash {
    Flash {
        id: id.into(),
        from: from.into(),
        content: content.into(),
        expires_at: "2099-01-01T00:00:00Z".into(), // Far future, won't expire
    }
}

fn embedding(id: &str, content: &str, source: &str) -> Embedding {
    Embedding {
        id: id.into(),
        content: content.into(),
        source: source.into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
    }
}

fn chat_msg(id: &str, author: &str, content: &str) -> ChatMessage {
    ChatMessage {
        id: id.into(),
        timestamp: "2026-04-01T12:00:00Z".into(),
        author: Author {
            id: format!("user_{}", author),
            name: author.into(),
            bot: false,
        },
        content: content.into(),
    }
}

fn print_result(name: &str, request: ContextRequest) {
    println!("\n{}", "=".repeat(60));
    println!("SCENARIO: {}", name);
    println!("{}", "=".repeat(60));

    // Print input summary
    println!("\n--- INPUT ---");
    println!("Channels: {:?}", request.channels.iter().map(|c| c.channel.name.as_ref().unwrap()).collect::<Vec<_>>());
    println!("Flashes: {}", request.flashes.len());
    println!("History items: {}", request.history.len());

    match build_context(request) {
        Ok(response) => {
            println!("\n--- OUTPUT ({} tokens est.) ---", response.estimated_tokens);
            for (i, msg) in response.messages.iter().enumerate() {
                let content_preview = msg.content.as_ref()
                    .map(|c| if c.len() > 80 { format!("{}...", &c[..80]) } else { c.clone() })
                    .unwrap_or_else(|| "[no content]".into());

                let tool_info = if msg.tool_calls.is_some() {
                    " [has tool_calls]"
                } else if msg.tool_call_id.is_some() {
                    " [tool_result]"
                } else {
                    ""
                };

                println!("{:>3}. [{:>9}]{} {}", i + 1, msg.role, tool_info, content_preview);
            }
        }
        Err(e) => {
            println!("\n--- ERROR ---");
            println!("{:?}", e);
        }
    }
}

fn main() {
    // Scenario 1: Single channel with moments and moves
    print_result(
        "Single channel with timeline items",
        ContextRequest {
            channels: vec![ChannelContext {
                channel: channel("general"),
                moments: vec![
                    moment(&make_id(1000), "Early discussion about project setup"),
                    moment(&make_id(3000), "Later discussion about API design"),
                ],
                moves: vec![
                    mv(&make_id(2000), "Decided to use REST over GraphQL"),
                ],
                messages: vec![
                    chat_msg(&make_id(4000), "alice", "What about authentication?"),
                    chat_msg(&make_id(4100), "bob", "Let's use JWT"),
                ],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![],
            history: vec![
                OpenAIMessage::system("You are a helpful assistant."),
            ],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        },
    );

    // Scenario 2: Flash interspersed in timeline
    print_result(
        "Flash interspersed with moments",
        ContextRequest {
            channels: vec![ChannelContext {
                channel: channel("general"),
                moments: vec![
                    moment(&make_id(1000), "Moment BEFORE flash"),
                    moment(&make_id(3000), "Moment AFTER flash"),
                ],
                moves: vec![],
                messages: vec![
                    chat_msg(&make_id(4000), "alice", "Current message"),
                ],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![
                flash(&make_id(2000), "worker-b", "FLASH: Check PR #42"),
            ],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        },
    );

    // Scenario 3: Multiple channels - current vs last vs other
    print_result(
        "Three channels: current=#dev, last=#general, other=#random",
        ContextRequest {
            channels: vec![
                // Index 0: CURRENT channel
                ChannelContext {
                    channel: channel("dev"),
                    moments: vec![moment(&make_id(5000), "[dev] Code review discussion")],
                    moves: vec![mv(&make_id(5500), "[dev] Approved PR")],
                    messages: vec![
                        chat_msg(&make_id(6000), "alice", "[dev] @bot check the build"),
                    ],
                    embeddings: vec![
                        embedding(&make_id(5200), "[dev] Embedding about testing", "docs/testing.md"),
                    ],
                    inbox: vec![],
                },
                // Index 1: LAST channel
                ChannelContext {
                    channel: channel("general"),
                    moments: vec![moment(&make_id(3000), "[general] Sprint planning")],
                    moves: vec![mv(&make_id(3500), "[general] Assigned tasks")],
                    messages: vec![
                        chat_msg(&make_id(4000), "bob", "[general] old message - should NOT appear"),
                    ],
                    embeddings: vec![
                        embedding(&make_id(3200), "[general] Embedding from last channel", "docs/sprint.md"),
                    ],
                    inbox: vec![],
                },
                // Index 2+: OTHER channels
                ChannelContext {
                    channel: channel("random"),
                    moments: vec![moment(&make_id(1000), "[random] Water cooler chat")],
                    moves: vec![mv(&make_id(1500), "[random] Discussed lunch")],
                    messages: vec![
                        chat_msg(&make_id(2000), "charlie", "[random] old message - should NOT appear"),
                    ],
                    embeddings: vec![
                        embedding(&make_id(1200), "[random] Embedding - should NOT appear (other channel)", "docs/fun.md"),
                    ],
                    inbox: vec![],
                },
            ],
            flashes: vec![
                flash(&make_id(2500), "worker-c", "FLASH between channels"),
            ],
            history: vec![
                OpenAIMessage::system("System prompt"),
                OpenAIMessage::assistant("Previous assistant response"),
            ],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        },
    );

    // Scenario 4: Channel switch simulation
    // What the context looks like when you've just switched from #general to #dev
    print_result(
        "Just switched from #general to #dev (no history in new channel yet)",
        ContextRequest {
            channels: vec![
                // Current: #dev (just switched here, no messages yet)
                ChannelContext {
                    channel: channel("dev"),
                    moments: vec![],
                    moves: vec![],
                    messages: vec![], // No messages yet in new channel
                    embeddings: vec![],
                inbox: vec![],
                },
                // Last: #general (where we came from)
                ChannelContext {
                    channel: channel("general"),
                    moments: vec![moment(&make_id(1000), "[general] Previous conversation summary")],
                    moves: vec![],
                    messages: vec![], // Raw messages not shown for non-current
                    embeddings: vec![],
                inbox: vec![],
                },
            ],
            flashes: vec![],
            history: vec![
                OpenAIMessage::system("You are a helpful assistant."),
                OpenAIMessage::user("(from #general) What's the status?"),
                OpenAIMessage::assistant("(from #general) Everything looks good."),
            ],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        },
    );

    // Scenario 5: Expired flash (should be filtered)
    print_result(
        "Expired flash should be filtered out",
        ContextRequest {
            channels: vec![ChannelContext {
                channel: channel("general"),
                moments: vec![moment(&make_id(1000), "A moment")],
                moves: vec![],
                messages: vec![],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![
                Flash {
                    id: make_id(500),
                    from: "worker-x".into(),
                    content: "EXPIRED FLASH - should NOT appear".into(),
                    expires_at: "2020-01-01T00:00:00Z".into(), // In the past
                },
                flash(&make_id(1500), "worker-y", "VALID FLASH - should appear"),
            ],
            history: vec![],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        },
    );

    // Scenario 6: Multiple flashes at different times
    print_result(
        "Multiple flashes interspersed throughout timeline",
        ContextRequest {
            channels: vec![ChannelContext {
                channel: channel("general"),
                moments: vec![
                    moment(&make_id(1000), "Moment at t=1000"),
                    moment(&make_id(3000), "Moment at t=3000"),
                    moment(&make_id(5000), "Moment at t=5000"),
                ],
                moves: vec![],
                messages: vec![
                    chat_msg(&make_id(6000), "alice", "Current message"),
                ],
                embeddings: vec![],
                inbox: vec![],
            }],
            flashes: vec![
                flash(&make_id(500), "worker-a", "Flash at t=500 (earliest)"),
                flash(&make_id(2000), "worker-b", "Flash at t=2000 (middle)"),
                flash(&make_id(4000), "worker-c", "Flash at t=4000 (later)"),
                flash(&make_id(5500), "worker-d", "Flash at t=5500 (latest)"),
            ],
            history: vec![
                OpenAIMessage::system("System prompt"),
            ],
            max_tokens: 10000,
            now: "2026-04-01T12:00:00Z".into(),
        },
    );

    println!("\n");
}

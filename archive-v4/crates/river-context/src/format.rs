//! Formatting workspace types to OpenAI messages.

use river_protocol::Channel;

use crate::openai::OpenAIMessage;
use crate::workspace::{ChatMessage, Embedding, Flash, InboxItem, Moment, Move};

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

/// Format an inbox item as a system message.
pub fn format_inbox_item(item: &InboxItem) -> OpenAIMessage {
    // Extract time portion from timestamp (e.g., "07:28" from "2026-04-01T07:28:00Z")
    let time = item
        .timestamp
        .split('T')
        .nth(1)
        .map(|t| {
            let parts: Vec<&str> = t.split(':').take(2).collect();
            parts.join(":")
        })
        .unwrap_or_else(|| item.timestamp.clone());

    OpenAIMessage::system(format!("[inbox] {} {}: {}", time, item.tool, item.summary))
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

#[cfg(test)]
mod tests {
    use super::*;
    use river_protocol::Author;

    fn test_channel() -> Channel {
        Channel {
            adapter: "discord".into(),
            id: "123456".into(),
            name: Some("general".into()),
        }
    }

    fn test_channel_no_name() -> Channel {
        Channel {
            adapter: "slack".into(),
            id: "789".into(),
            name: None,
        }
    }

    #[test]
    fn test_format_moment_with_channel_name() {
        let moment = Moment {
            id: "1".into(),
            content: "Discussion about API design".into(),
            move_range: ("100".into(), "150".into()),
        };
        let channel = test_channel();

        let msg = format_moment(&moment, &channel);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Moment: general] Discussion about API design (moves 100-150)"
        );
    }

    #[test]
    fn test_format_moment_without_channel_name() {
        let moment = Moment {
            id: "1".into(),
            content: "Team sync".into(),
            move_range: ("50".into(), "75".into()),
        };
        let channel = test_channel_no_name();

        let msg = format_moment(&moment, &channel);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Moment: 789] Team sync (moves 50-75)"
        );
    }

    #[test]
    fn test_format_move_with_channel_name() {
        let mv = Move {
            id: "2".into(),
            content: "Reviewed PR #42".into(),
            message_range: ("200".into(), "210".into()),
        };
        let channel = test_channel();

        let msg = format_move(&mv, &channel);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Move: general] Reviewed PR #42 (messages 200-210)"
        );
    }

    #[test]
    fn test_format_move_without_channel_name() {
        let mv = Move {
            id: "2".into(),
            content: "Bug triage".into(),
            message_range: ("300".into(), "320".into()),
        };
        let channel = test_channel_no_name();

        let msg = format_move(&mv, &channel);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Move: 789] Bug triage (messages 300-320)"
        );
    }

    #[test]
    fn test_format_flash() {
        let flash = Flash {
            id: "3".into(),
            from: "worker-alpha".into(),
            content: "Urgent: deploy blocked".into(),
            expires_at: "2026-04-01T15:00:00Z".into(),
        };

        let msg = format_flash(&flash);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Flash from worker-alpha] Urgent: deploy blocked"
        );
    }

    #[test]
    fn test_format_embedding() {
        let embedding = Embedding {
            id: "4".into(),
            content: "API documentation for /users endpoint".into(),
            source: "docs/api.md:15-42".into(),
            expires_at: "2026-04-01T18:00:00Z".into(),
        };

        let msg = format_embedding(&embedding);

        assert_eq!(msg.role, "system");
        assert_eq!(
            msg.content.unwrap(),
            "[Reference: docs/api.md:15-42]\nAPI documentation for /users endpoint"
        );
    }

    #[test]
    fn test_format_chat_messages_single() {
        let messages = vec![ChatMessage {
            id: "5".into(),
            timestamp: "2026-04-01T12:00:00Z".into(),
            author: Author {
                id: "user1".into(),
                name: "Alice".into(),
                bot: false,
            },
            content: "Hello world!".into(),
        }];
        let channel = test_channel();

        let msg = format_chat_messages(&messages, &channel);

        assert_eq!(msg.role, "user");
        assert_eq!(
            msg.content.unwrap(),
            "[Chat: general]\n[2026-04-01T12:00:00Z] <Alice> Hello world!"
        );
    }

    #[test]
    fn test_format_chat_messages_multiple() {
        let messages = vec![
            ChatMessage {
                id: "5".into(),
                timestamp: "2026-04-01T12:00:00Z".into(),
                author: Author {
                    id: "user1".into(),
                    name: "Alice".into(),
                    bot: false,
                },
                content: "Hello!".into(),
            },
            ChatMessage {
                id: "6".into(),
                timestamp: "2026-04-01T12:01:00Z".into(),
                author: Author {
                    id: "user2".into(),
                    name: "Bob".into(),
                    bot: false,
                },
                content: "Hi Alice!".into(),
            },
            ChatMessage {
                id: "7".into(),
                timestamp: "2026-04-01T12:02:00Z".into(),
                author: Author {
                    id: "user1".into(),
                    name: "Alice".into(),
                    bot: false,
                },
                content: "How are you?".into(),
            },
        ];
        let channel = test_channel();

        let msg = format_chat_messages(&messages, &channel);

        assert_eq!(msg.role, "user");
        let content = msg.content.unwrap();
        assert!(content.starts_with("[Chat: general]"));
        assert!(content.contains("<Alice> Hello!"));
        assert!(content.contains("<Bob> Hi Alice!"));
        assert!(content.contains("<Alice> How are you?"));
    }

    #[test]
    fn test_format_inbox_item() {
        let item = InboxItem {
            id: "discord_chan123_2026-04-01T07-28-00Z_read_channel".into(),
            timestamp: "2026-04-01T07:28:00Z".into(),
            tool: "read_channel".into(),
            channel_adapter: "discord".into(),
            channel_id: "chan123".into(),
            summary: "msg1150-msg1200".into(),
        };

        let msg = format_inbox_item(&item);

        assert_eq!(msg.role, "system");
        assert!(msg.content.as_ref().unwrap().contains("[inbox]"));
        assert!(msg.content.as_ref().unwrap().contains("07:28"));
        assert!(msg.content.as_ref().unwrap().contains("read_channel"));
        assert!(msg.content.as_ref().unwrap().contains("msg1150-msg1200"));
    }
}

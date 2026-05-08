//! Formatting utilities for spectator handlers

use river_db::{Message, Move};

/// Format a list of messages into a readable transcript for the LLM.
///
/// Output format:
/// ```text
/// [user] What is X?
/// [agent] X is Y.
/// [agent/tool_call] {"name":"read","arguments":"{}"}
/// [tool] Contents of file...
/// ```
///
/// Note: "assistant" role is displayed as "agent" in transcripts.
/// The internal role stays "assistant" for API compatibility.
pub fn format_transcript(messages: &[Message]) -> String {
    let mut lines = Vec::new();
    for msg in messages {
        let role = match msg.role {
            river_db::MessageRole::Assistant => "agent",
            other => other.as_str(),
        };
        if let Some(ref content) = msg.content {
            lines.push(format!("[{}] {}", role, content));
        }
        if let Some(ref tool_calls) = msg.tool_calls {
            lines.push(format!("[{}/tool_call] {}", role, tool_calls));
        }
    }
    lines.join("\n")
}

/// Format a list of moves for the compression prompt.
///
/// Output format:
/// ```text
/// Turn 1: User asked about X, agent explored files
/// Turn 2: Agent wrote implementation based on findings
/// ```
pub fn format_moves(moves: &[Move]) -> String {
    moves
        .iter()
        .map(|m| format!("Turn {}: {}", m.turn_number, m.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build a fallback move summary from messages when the LLM fails.
///
/// Format: "User message -> assistant response with tools: read, write"
pub fn fallback_summary(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    let mut tool_names: Vec<String> = Vec::new();

    for msg in messages {
        match msg.role {
            river_db::MessageRole::User => parts.push("User message"),
            river_db::MessageRole::Assistant => parts.push("agent response"),
            river_db::MessageRole::Tool => {
                if let Some(ref name) = msg.name {
                    if !tool_names.contains(name) {
                        tool_names.push(name.clone());
                    }
                }
            }
            river_db::MessageRole::System => {}
        }
    }

    let mut result = parts.join(" -> ");
    if !tool_names.is_empty() {
        result.push_str(&format!(" with tools: {}", tool_names.join(", ")));
    }
    if result.is_empty() {
        result = "Empty turn".to_string();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};
    use river_db::MessageRole;

    fn test_gen() -> SnowflakeGenerator {
        SnowflakeGenerator::new(AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap())
    }

    fn make_msg(gen: &SnowflakeGenerator, role: MessageRole, content: &str) -> Message {
        Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "sess".to_string(),
            role,
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            turn_number: 1,
            created_at: 1000,
            metadata: None,
        }
    }

    #[test]
    fn test_format_transcript() {
        let gen = test_gen();
        let messages = vec![
            make_msg(&gen, MessageRole::User, "What is X?"),
            make_msg(&gen, MessageRole::Assistant, "X is Y."),
        ];
        let result = format_transcript(&messages);
        assert!(result.contains("[user] What is X?"));
        assert!(result.contains("[agent] X is Y."));
    }

    #[test]
    fn test_format_moves() {
        let gen = test_gen();
        let moves = vec![
            Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: 1,
                summary: "User asked about X".to_string(),
                tool_calls: None,
                created_at: 1000,
            },
            Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: 2,
                summary: "Agent wrote response".to_string(),
                tool_calls: None,
                created_at: 2000,
            },
        ];
        let result = format_moves(&moves);
        assert_eq!(result, "Turn 1: User asked about X\nTurn 2: Agent wrote response");
    }

    #[test]
    fn test_fallback_summary() {
        let gen = test_gen();
        let mut tool_msg = make_msg(&gen, MessageRole::Tool, "file contents");
        tool_msg.name = Some("read".to_string());

        let messages = vec![
            make_msg(&gen, MessageRole::User, "Read the file"),
            make_msg(&gen, MessageRole::Assistant, "Let me read it"),
            tool_msg,
        ];
        let result = fallback_summary(&messages);
        assert_eq!(result, "User message -> agent response with tools: read");
    }

    #[test]
    fn test_fallback_summary_empty() {
        let result = fallback_summary(&[]);
        assert_eq!(result, "Empty turn");
    }
}

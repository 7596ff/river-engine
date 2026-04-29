//! Token estimation.

use crate::openai::OpenAIMessage;

/// Estimate tokens for a string (~4 characters per token).
pub fn estimate_tokens(s: &str) -> usize {
    (s.len() + 3) / 4
}

/// Estimate tokens for an OpenAI message.
pub fn estimate_message_tokens(msg: &OpenAIMessage) -> usize {
    // Base overhead for message structure
    let mut tokens = 4;

    if let Some(content) = &msg.content {
        tokens += estimate_tokens(content);
    }

    if let Some(tool_calls) = &msg.tool_calls {
        for call in tool_calls {
            tokens += 4; // overhead
            tokens += estimate_tokens(&call.id);
            tokens += estimate_tokens(&call.function.name);
            tokens += estimate_tokens(&call.function.arguments);
        }
    }

    if let Some(tool_call_id) = &msg.tool_call_id {
        tokens += estimate_tokens(tool_call_id);
    }

    tokens
}

/// Estimate total tokens for a list of messages.
pub fn estimate_total_tokens(messages: &[OpenAIMessage]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{FunctionCall, ToolCall};

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello"), 2); // 5 chars -> 2 tokens
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars -> 3 tokens
    }

    #[test]
    fn test_estimate_message_tokens_system() {
        let msg = OpenAIMessage::system("Hello world");
        let tokens = estimate_message_tokens(&msg);

        // 4 (base) + 3 (11 chars / 4) = 7
        assert!(tokens >= 7);
    }

    #[test]
    fn test_estimate_message_tokens_with_tool_calls() {
        let msg = OpenAIMessage {
            role: "assistant".into(),
            content: Some("Let me help".into()),
            tool_calls: Some(vec![ToolCall {
                id: "call_123".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "search".into(),
                    arguments: r#"{"query": "test"}"#.into(),
                },
            }]),
            tool_call_id: None,
        };

        let tokens = estimate_message_tokens(&msg);

        // Should include overhead for tool calls
        assert!(tokens > 10);
    }

    #[test]
    fn test_estimate_message_tokens_tool_result() {
        let msg = OpenAIMessage::tool("call_123", "Result content here");
        let tokens = estimate_message_tokens(&msg);

        // 4 (base) + content tokens + tool_call_id tokens
        assert!(tokens >= 4);
    }

    #[test]
    fn test_estimate_total_tokens_empty() {
        let messages: Vec<OpenAIMessage> = vec![];
        assert_eq!(estimate_total_tokens(&messages), 0);
    }

    #[test]
    fn test_estimate_total_tokens_multiple() {
        let messages = vec![
            OpenAIMessage::system("System prompt"),
            OpenAIMessage::user("User question"),
            OpenAIMessage::assistant("Assistant response"),
        ];

        let total = estimate_total_tokens(&messages);
        let sum: usize = messages.iter().map(estimate_message_tokens).sum();

        assert_eq!(total, sum);
    }

    #[test]
    fn test_estimate_tokens_long_string() {
        let long_string = "x".repeat(1000);
        let tokens = estimate_tokens(&long_string);

        // 1000 chars / 4 = 250 tokens
        assert_eq!(tokens, 250); // (1000 + 3) / 4 = 250 (integer division)
    }
}

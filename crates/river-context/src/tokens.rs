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

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello"), 2); // 5 chars -> 2 tokens
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars -> 3 tokens
    }
}

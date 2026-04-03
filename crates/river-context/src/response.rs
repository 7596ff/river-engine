//! Context response types.

use crate::openai::OpenAIMessage;

/// Response from context assembly.
#[derive(Clone, Debug)]
pub struct ContextResponse {
    /// Flat timeline of OpenAI-compatible messages.
    pub messages: Vec<OpenAIMessage>,
    /// Estimated token count.
    pub estimated_tokens: usize,
}

/// Errors that can occur during context assembly.
#[derive(Debug)]
pub enum ContextError {
    /// Assembled context exceeds max_tokens.
    OverBudget { estimated: usize, limit: usize },
    /// No channels provided.
    EmptyChannels,
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverBudget { estimated, limit } => {
                write!(f, "context over budget: {} tokens (limit {})", estimated, limit)
            }
            Self::EmptyChannels => write!(f, "no channels provided"),
        }
    }
}

impl std::error::Error for ContextError {}

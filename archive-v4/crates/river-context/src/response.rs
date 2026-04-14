//! Context response types.

use thiserror::Error;

use crate::openai::OpenAIMessage;

/// Response from context assembly.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextResponse {
    /// Flat timeline of OpenAI-compatible messages.
    pub messages: Vec<OpenAIMessage>,
    /// Estimated token count.
    pub estimated_tokens: usize,
}

/// Errors that can occur during context assembly.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContextError {
    /// Assembled context exceeds max_tokens.
    #[error("context over budget: {estimated} tokens (limit {limit})")]
    OverBudget { estimated: usize, limit: usize },

    /// No channels provided.
    #[error("no channels provided")]
    EmptyChannels,
}

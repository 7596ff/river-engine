//! LLM provider implementations.

use anyhow::Result;
use async_trait::async_trait;

use crate::types::{LlmResponse, Message, ToolDef};

mod claude;

pub use claude::ClaudeProvider;

/// Trait for LLM providers.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Complete a conversation with optional tool definitions.
    async fn complete(&self, messages: &[Message], tools: &[ToolDef]) -> Result<LlmResponse>;

    /// Get the model name.
    fn model_name(&self) -> &str;
}

// TODO: Implement in Phase 5
// mod openai;
// mod ollama;
//
// pub use openai::OpenAiProvider;
// pub use ollama::OllamaProvider;

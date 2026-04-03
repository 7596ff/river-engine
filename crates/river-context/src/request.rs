//! Context request types.

use river_adapter::Channel;
use serde::{Deserialize, Serialize};

use crate::openai::OpenAIMessage;
use crate::workspace::{ChatMessage, Embedding, Flash, Moment, Move};

/// Context for a single channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelContext {
    pub channel: Channel,
    pub moments: Vec<Moment>,
    pub moves: Vec<Move>,
    pub messages: Vec<ChatMessage>,
    pub embeddings: Vec<Embedding>,
}

/// Request to build context.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextRequest {
    /// Channels: [0] is current, rest are last 4 by recency.
    pub channels: Vec<ChannelContext>,
    /// Global flashes, interspersed by timestamp.
    pub flashes: Vec<Flash>,
    /// LLM conversation history (from context.jsonl, already OpenAI format).
    pub history: Vec<OpenAIMessage>,
    /// Token limit (estimate-based).
    pub max_tokens: usize,
    /// Current time for TTL filtering (ISO8601).
    pub now: String,
}

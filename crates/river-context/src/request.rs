//! Context request types.

use river_protocol::Channel;
use serde::{Deserialize, Serialize};

use crate::openai::OpenAIMessage;
use crate::workspace::{ChatMessage, Embedding, Flash, InboxItem, Moment, Move};

/// Context for a single channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelContext {
    pub channel: Channel,
    pub moments: Vec<Moment>,
    pub moves: Vec<Move>,
    pub messages: Vec<ChatMessage>,
    pub embeddings: Vec<Embedding>,
    pub inbox: Vec<InboxItem>,
}

impl Default for ChannelContext {
    fn default() -> Self {
        Self {
            channel: Channel {
                adapter: String::new(),
                id: String::new(),
                name: None,
            },
            moments: Vec::new(),
            moves: Vec::new(),
            messages: Vec::new(),
            embeddings: Vec::new(),
            inbox: Vec::new(),
        }
    }
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

impl Default for ContextRequest {
    fn default() -> Self {
        Self {
            channels: Vec::new(),
            flashes: Vec::new(),
            history: Vec::new(),
            max_tokens: 8000,
            now: String::new(),
        }
    }
}

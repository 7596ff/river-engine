//! River Context - Context assembly library for River Engine workers.
//!
//! This crate provides a pure function that assembles workspace data into
//! OpenAI-compatible messages for LLM consumption.
//!
//! # Example
//!
//! ```rust
//! use river_context::{build_context, ContextRequest, ChannelContext, OpenAIMessage};
//! use river_adapter::Channel;
//!
//! let request = ContextRequest {
//!     channels: vec![ChannelContext {
//!         channel: Channel {
//!             adapter: "discord".into(),
//!             id: "123".into(),
//!             name: Some("general".into()),
//!         },
//!         moments: vec![],
//!         moves: vec![],
//!         messages: vec![],
//!         embeddings: vec![],
//!     }],
//!     flashes: vec![],
//!     history: vec![],
//!     max_tokens: 8000,
//!     now: "2026-04-01T12:00:00Z".into(),
//! };
//!
//! let response = build_context(request).unwrap();
//! println!("Estimated tokens: {}", response.estimated_tokens);
//! ```

mod assembly;
mod format;
mod openai;
mod request;
mod response;
mod tokens;
mod workspace;

pub use assembly::build_context;
pub use openai::{FunctionCall, OpenAIMessage, ToolCall};
pub use request::{ChannelContext, ContextRequest};
pub use response::{ContextError, ContextResponse};
pub use tokens::{estimate_message_tokens, estimate_tokens, estimate_total_tokens};
pub use workspace::{ChatMessage, Embedding, Flash, Moment, Move};

// Re-export types from river-protocol
pub use river_protocol::{Author, Channel};

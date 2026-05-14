//! Model interaction types and client

pub mod client;
pub mod types;

pub use client::{ModelClient, ModelResponse, Provider, Usage};
pub use types::{ChatMessage, FunctionCall, ToolCallRequest};

//! Model interaction types and client

pub mod types;
pub mod client;

pub use types::{ChatMessage, ToolCallRequest, FunctionCall};
pub use client::{ModelClient, ModelResponse, Usage, Provider};

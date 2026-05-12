//! Agent (I) — the acting self
//!
//! The agent runs as a peer task in the coordinator, managing the wake/think/act/settle
//! turn cycle. Context is built from the home channel — an append-only JSONL log.

pub mod channel; // kept for communication tools — will be removed with channel switching
pub mod context; // kept for spectator compatibility — will be removed
pub mod home_context;
pub mod task;
pub mod tools;

pub use channel::ChannelContext;
pub use home_context::HomeContextConfig;
pub use task::{AgentTask, AgentTaskConfig};

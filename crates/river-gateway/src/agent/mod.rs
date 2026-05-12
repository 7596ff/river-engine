//! Agent (I) — the acting self
//!
//! The agent runs as a peer task in the coordinator, managing the wake/think/act/settle
//! turn cycle. It uses a persistent context that accumulates messages and compacts
//! via spectator cursor coordination.

pub mod channel;
pub mod context;
pub mod home_context;
pub mod task;
pub mod tools;

pub use channel::ChannelContext;
pub use context::{PersistentContext, ContextConfig, ContextMessage, TokenCalibration};
pub use task::{AgentTask, AgentTaskConfig};

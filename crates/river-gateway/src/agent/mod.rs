//! Agent (I) — the acting self
//!
//! The agent runs as a peer task in the coordinator, managing the wake/think/act/settle
//! turn cycle. It uses context assembly with hot/warm/cold layers and emits lifecycle
//! events for the spectator to observe.

pub mod channel;
pub mod context;
pub mod task;
pub mod tools;

pub use channel::ChannelContext;
pub use context::{ContextAssembler, ContextBudget, AssembledContext, LayerStats};
pub use task::{AgentTask, AgentTaskConfig};

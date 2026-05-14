//! Tool system — all agent capabilities

// Core
pub mod executor;
pub mod registry;

// Pure tools
pub mod file;
pub mod logging;
pub mod shell;
pub mod web;

// Stateful tools
pub mod context;
pub mod heartbeat;
pub mod memory;
pub mod model;

// Gateway-integrated tools
pub mod adapters;
pub mod communication;
pub mod subagent;
pub mod sync;

// Re-export core types
pub use executor::{ToolCall, ToolCallResponse, ToolExecutor};
pub use registry::{Tool, ToolRegistry, ToolResult, ToolSchema};

// Re-export tools
pub use adapters::{AdapterConfig, AdapterRegistry, ListAdaptersTool};
pub use communication::{ReadChannelTool, SendMessageTool};
pub use context::{ContextRotation, ContextStatusTool, RotateContextTool};
pub use file::{EditTool, GlobTool, GrepTool, ReadTool, WriteTool};
pub use heartbeat::{HeartbeatScheduler, ScheduleHeartbeatTool};
pub use logging::LogReadTool;
pub use memory::{EmbedTool, MemoryDeleteBySourceTool, MemoryDeleteTool, MemorySearchTool};
pub use model::{
    ModelManagerConfig, ModelManagerState, ReleaseModelTool, RequestModelTool, SwitchModelTool,
};
pub use shell::BashTool;
pub use subagent::{
    InternalReceiveTool, InternalSendTool, ListSubagentsTool, SpawnSubagentTool, StopSubagentTool,
    SubagentStatusTool, WaitForSubagentTool,
};
pub use sync::SyncConversationTool;
pub use web::{WebFetchTool, WebSearchTool};

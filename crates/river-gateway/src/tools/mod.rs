//! Tool system — all agent capabilities

// Core
pub mod registry;
pub mod executor;

// Pure tools
pub mod file;
pub mod shell;
pub mod web;
pub mod logging;

// Stateful tools
pub mod model;
pub mod context;
pub mod heartbeat;
pub mod memory;

// Gateway-integrated tools
pub mod adapters;
pub mod communication;
pub mod subagent;
pub mod sync;

// Re-export core types
pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::{ToolExecutor, ToolCall, ToolCallResponse};

// Re-export tools
pub use file::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool};
pub use shell::BashTool;
pub use web::{WebFetchTool, WebSearchTool};
pub use logging::LogReadTool;
pub use model::{ModelManagerConfig, ModelManagerState, RequestModelTool, ReleaseModelTool, SwitchModelTool};
pub use context::{ContextRotation, RotateContextTool, ContextStatusTool};
pub use heartbeat::{HeartbeatScheduler, ScheduleHeartbeatTool};
pub use adapters::{AdapterConfig, AdapterRegistry, ListAdaptersTool};
pub use communication::{SendMessageTool, ReadChannelTool};
pub use memory::{EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool};
pub use subagent::{
    SpawnSubagentTool, ListSubagentsTool, SubagentStatusTool, StopSubagentTool,
    InternalSendTool, InternalReceiveTool, WaitForSubagentTool
};
pub use sync::SyncConversationTool;

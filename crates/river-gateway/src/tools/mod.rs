//! Tool system

mod registry;
mod executor;
mod file;
mod shell;
mod memory;
mod communication;
mod web;
mod model;
mod scheduling;
mod logging;
mod subagent;

pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::{ToolExecutor, ToolCall, ToolCallResponse};
pub use file::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool};
pub use shell::BashTool;
pub use memory::{EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool};
pub use communication::{
    AdapterConfig, AdapterRegistry, SendMessageTool, ListAdaptersTool, ContextStatusTool,
    ReadChannelTool
};
pub use web::{WebFetchTool, WebSearchTool};
pub use model::{
    ModelManagerConfig, ModelManagerState, RequestModelTool, ReleaseModelTool, SwitchModelTool
};
pub use scheduling::{ContextRotation, HeartbeatScheduler, RotateContextTool, ScheduleHeartbeatTool};
pub use logging::LogReadTool;
pub use subagent::{
    SpawnSubagentTool, ListSubagentsTool, SubagentStatusTool, StopSubagentTool,
    InternalSendTool, InternalReceiveTool, WaitForSubagentTool
};

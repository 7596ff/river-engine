//! Tool system — re-exports from river-tools + gateway-specific tools

// Gateway-specific tools (depend on gateway internals)
mod communication;
mod memory;
mod subagent;
mod sync;

// Re-export everything from river-tools
pub use river_tools::{
    Tool, ToolRegistry, ToolSchema, ToolResult,
    ToolExecutor, ToolCall, ToolCallResponse,
    ReadTool, WriteTool, EditTool, GlobTool, GrepTool,
    BashTool,
    WebFetchTool, WebSearchTool,
    ModelManagerConfig, ModelManagerState, RequestModelTool, ReleaseModelTool, SwitchModelTool,
    ContextRotation, HeartbeatScheduler, RotateContextTool, ScheduleHeartbeatTool,
    LogReadTool,
};

// Re-export gateway-specific tools
pub use communication::{
    AdapterConfig, AdapterRegistry, SendMessageTool, ListAdaptersTool, ContextStatusTool,
    ReadChannelTool
};
pub use sync::SyncConversationTool;
pub use memory::{EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool};
pub use subagent::{
    SpawnSubagentTool, ListSubagentsTool, SubagentStatusTool, StopSubagentTool,
    InternalSendTool, InternalReceiveTool, WaitForSubagentTool
};

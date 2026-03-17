//! Tool system

mod registry;
mod executor;
mod file;
mod shell;
mod memory;
mod communication;
mod web;
mod model;

pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::{ToolExecutor, ToolCall, ToolCallResponse};
pub use file::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool};
pub use shell::BashTool;
pub use memory::{EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool};
pub use communication::{
    AdapterConfig, AdapterRegistry, SendMessageTool, ListAdaptersTool, ContextStatusTool
};
pub use web::WebFetchTool;
pub use model::{
    ModelManagerConfig, ModelManagerState, RequestModelTool, ReleaseModelTool, SwitchModelTool
};

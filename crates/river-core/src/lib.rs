//! River Core - foundational types for River Engine
//!
//! This crate provides the core types, error handling, and configuration
//! structures used throughout River Engine.

pub mod auth;
pub mod config;
pub mod error;
pub mod snowflake;
pub mod types;

// Re-exports from auth module
pub use auth::{build_authed_client, require_auth_token, validate_bearer};

// Re-exports from snowflake module
pub use snowflake::{AgentBirth, Snowflake, SnowflakeGenerator, SnowflakeType};

// Re-exports from types module
pub use types::{ContextStatus, Priority, SubagentType};

// Re-exports from error module
pub use error::{RiverError, RiverResult};

// Re-exports from config module
pub use config::{AgentConfig, EmbeddingConfig, HeartbeatConfig, OrchestratorConfig};

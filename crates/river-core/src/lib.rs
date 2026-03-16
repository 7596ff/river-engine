//! River Core - foundational types for River Engine

pub mod snowflake;
pub mod types;
pub mod config;
pub mod error;

// Re-exports
pub use snowflake::{AgentBirth, Snowflake, SnowflakeGenerator, SnowflakeType};

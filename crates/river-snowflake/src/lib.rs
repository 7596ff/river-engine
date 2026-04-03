//! River Snowflake - 128-bit unique ID generation.
//!
//! This crate provides unique 128-bit snowflake IDs for the River Engine system.
//!
//! # Library Usage
//!
//! ```rust
//! use river_snowflake::{parse, format, timestamp_iso8601, GeneratorCache, AgentBirth, SnowflakeType};
//!
//! // Parsing
//! let id = parse("0000000000123456-1a2b3c4d5e6f7890").unwrap();
//! let timestamp = timestamp_iso8601(&id);
//! let hex = format(&id);
//!
//! // Embedded generation
//! let cache = GeneratorCache::new();
//! let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
//! let id = cache.next_id(birth, SnowflakeType::Message).unwrap();
//! ```

mod cache;
mod extract;
mod parse;
mod snowflake;

#[cfg(feature = "server")]
pub mod server;

pub use cache::GeneratorCache;
pub use extract::timestamp_iso8601;
pub use parse::{format, parse};
pub use snowflake::{AgentBirth, Snowflake, SnowflakeGenerator, SnowflakeType};

/// Errors that can occur in snowflake operations.
#[derive(Debug, thiserror::Error)]
pub enum SnowflakeError {
    #[error("invalid format: {0}")]
    InvalidFormat(String),

    #[error("invalid birth: {0}")]
    InvalidBirth(String),

    #[error("invalid type: {0}")]
    InvalidType(String),

    #[error("sequence overflow: too many IDs generated in same microsecond")]
    SequenceOverflow,
}

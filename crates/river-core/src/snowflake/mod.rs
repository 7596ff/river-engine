//! Snowflake ID generation for River Engine.
//!
//! Implements 128-bit Snowflake IDs per spec Section 6:
//! - 64 bits: timestamp (microseconds since agent birth)
//! - 36 bits: agent birth (yyyymmddhhmmss packed)
//! - 8 bits: type (message=0x01, embedding=0x02, session=0x03, subagent=0x04, tool_call=0x05)
//! - 20 bits: sequence

mod birth;
mod generator;
mod id;
mod types;

pub use birth::AgentBirth;
pub use generator::SnowflakeGenerator;
pub use id::Snowflake;
pub use types::SnowflakeType;

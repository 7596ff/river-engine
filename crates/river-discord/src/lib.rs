//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod channels;
pub mod config;

pub use channels::ChannelState;
pub use config::{Args, DiscordConfig};

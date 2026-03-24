//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod adapter;
pub mod channels;
pub mod client;
pub mod commands;
pub mod config;
pub mod gateway;
pub mod handler;
pub mod outbound;

pub use adapter::{discord_adapter_info, register_with_gateway};
pub use channels::ChannelState;
pub use client::{DiscordClient, DiscordSender};
pub use config::{Args, DiscordConfig};
pub use gateway::{discord_metadata, GatewayClient, GatewayError, IncomingResponse};
pub use river_adapter::{Author, EventType, IncomingEvent};
pub use handler::EventHandler;

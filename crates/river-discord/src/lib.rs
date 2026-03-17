//! River Engine Discord Adapter
//!
//! Routes messages between Discord and river-gateway using Twilight.

pub mod channels;
pub mod config;
pub mod gateway;
pub mod handler;

pub use channels::ChannelState;
pub use config::{Args, DiscordConfig};
pub use gateway::{Author, EventMetadata, GatewayClient, GatewayError, IncomingEvent, IncomingResponse};
pub use handler::EventHandler;

//! River TUI — home channel viewer
//!
//! Reads home channel JSONL from stdin or a file, renders as a chat window,
//! and posts user input to the gateway's bystander endpoint.

pub mod config;
pub mod format;
pub mod input;
pub mod post;
pub mod render;

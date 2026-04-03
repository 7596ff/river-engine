//! River Protocol - Shared types for River Engine.
//!
//! This crate provides foundational types used across all River Engine crates.
//! It has no dependencies on other river-* crates.

mod identity;

pub use identity::{Attachment, Author, Baton, Channel, Ground, Side};

//! River Protocol - Shared types for River Engine.
//!
//! This crate provides foundational types used across all River Engine crates.
//! It has no dependencies on other river-* crates.

mod identity;
mod model;
mod registration;
mod registry;

pub use identity::{Attachment, Author, Baton, Channel, Ground, Side};
pub use model::ModelConfig;
pub use registration::{
    AdapterRegistration, AdapterRegistrationRequest, AdapterRegistrationResponse,
    WorkerRegistration, WorkerRegistrationRequest, WorkerRegistrationResponse,
};
pub use registry::{ProcessEntry, Registry};

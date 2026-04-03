//! River Adapter - Types-only library for adapter ↔ worker communication.
//!
//! This crate defines the interface between Workers and adapter binaries.
//! It exports types, traits, and enums — no HTTP infrastructure.
//!
//! # Feature System
//!
//! Two enums work together:
//! - [`FeatureId`]: Lightweight enum for registration and capability checks
//! - [`OutboundRequest`]: Data-carrying enum with typed payloads
//!
//! # Usage
//!
//! ```rust
//! use river_adapter::{FeatureId, OutboundRequest, Adapter, InboundEvent, EventMetadata, Author};
//!
//! // Check if a feature is required
//! assert!(FeatureId::SendMessage.is_required());
//! assert!(FeatureId::ReceiveMessage.is_required());
//! assert!(!FeatureId::EditMessage.is_required());
//!
//! // Get the feature ID for a request
//! let request = OutboundRequest::SendMessage {
//!     channel: "general".into(),
//!     content: "Hello!".into(),
//!     reply_to: None,
//! };
//! assert_eq!(request.feature_id(), FeatureId::SendMessage);
//! ```

mod author;
mod error;
mod event;
mod feature;
mod response;
mod traits;

pub use author::{Attachment, Author, Baton, Channel, Ground, Side};
pub use error::AdapterError;
pub use event::{EventMetadata, EventType, InboundEvent};
pub use feature::{FeatureId, OutboundRequest};
pub use response::{ErrorCode, HistoryMessage, OutboundResponse, ResponseData, ResponseError};
pub use traits::Adapter;

use utoipa::OpenApi;

/// OpenAPI documentation for adapter types.
#[derive(OpenApi)]
#[openapi(components(schemas(
    // Feature system
    FeatureId,
    OutboundRequest,
    // Inbound events
    InboundEvent,
    EventMetadata,
    EventType,
    // Responses
    OutboundResponse,
    ResponseData,
    ResponseError,
    ErrorCode,
    HistoryMessage,
    // Supporting
    Author,
    Channel,
    Attachment,
    Baton,
    Side,
    Ground,
)))]
pub struct AdapterApiDoc;

/// Generate OpenAPI JSON specification.
pub fn openapi_json() -> String {
    AdapterApiDoc::openapi()
        .to_pretty_json()
        .expect("failed to generate OpenAPI JSON")
}

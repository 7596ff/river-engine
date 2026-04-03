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

mod error;
mod event;
mod feature;
mod response;
mod traits;

// Re-export identity types from river-protocol
pub use river_protocol::{Attachment, Author, Baton, Channel, Ground, Side};

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
    // Supporting (from river-protocol)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_id_serde_roundtrip() {
        let features = [
            FeatureId::SendMessage,
            FeatureId::ReceiveMessage,
            FeatureId::EditMessage,
            FeatureId::DeleteMessage,
            FeatureId::ReadHistory,
            FeatureId::PinMessage,
            FeatureId::UnpinMessage,
            FeatureId::BulkDeleteMessages,
            FeatureId::AddReaction,
            FeatureId::RemoveReaction,
            FeatureId::RemoveAllReactions,
            FeatureId::Attachments,
            FeatureId::TypingIndicator,
            FeatureId::CreateThread,
            FeatureId::ThreadEvents,
            FeatureId::CreatePoll,
            FeatureId::PollVote,
            FeatureId::PollEvents,
            FeatureId::VoiceStateEvents,
            FeatureId::PresenceEvents,
            FeatureId::MemberEvents,
            FeatureId::ScheduledEvents,
            FeatureId::ChannelEvents,
            FeatureId::ConnectionEvents,
        ];
        for feature in features {
            let json = serde_json::to_string(&feature).unwrap();
            let parsed: FeatureId = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, feature, "Failed roundtrip for {:?}", feature);
        }
    }

    #[test]
    fn test_feature_id_is_required() {
        assert!(FeatureId::SendMessage.is_required());
        assert!(FeatureId::ReceiveMessage.is_required());
        assert!(!FeatureId::EditMessage.is_required());
        assert!(!FeatureId::DeleteMessage.is_required());
        assert!(!FeatureId::AddReaction.is_required());
        assert!(!FeatureId::ConnectionEvents.is_required());
    }

    #[test]
    fn test_feature_id_try_from_valid() {
        assert_eq!(FeatureId::try_from(0u16), Ok(FeatureId::SendMessage));
        assert_eq!(FeatureId::try_from(1u16), Ok(FeatureId::ReceiveMessage));
        assert_eq!(FeatureId::try_from(10u16), Ok(FeatureId::EditMessage));
        assert_eq!(FeatureId::try_from(20u16), Ok(FeatureId::AddReaction));
        assert_eq!(FeatureId::try_from(100u16), Ok(FeatureId::VoiceStateEvents));
        assert_eq!(FeatureId::try_from(900u16), Ok(FeatureId::ConnectionEvents));
    }

    #[test]
    fn test_feature_id_try_from_invalid() {
        assert_eq!(FeatureId::try_from(2u16), Err(2u16));
        assert_eq!(FeatureId::try_from(99u16), Err(99u16));
        assert_eq!(FeatureId::try_from(9999u16), Err(9999u16));
    }

    #[test]
    fn test_feature_id_u16_values() {
        assert_eq!(FeatureId::SendMessage as u16, 0);
        assert_eq!(FeatureId::ReceiveMessage as u16, 1);
        assert_eq!(FeatureId::EditMessage as u16, 10);
        assert_eq!(FeatureId::ConnectionEvents as u16, 900);
    }
}

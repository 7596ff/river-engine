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

    #[test]
    fn test_outbound_request_feature_id_mapping() {
        let cases = [
            (OutboundRequest::SendMessage { channel: "ch".into(), content: "hi".into(), reply_to: None }, FeatureId::SendMessage),
            (OutboundRequest::EditMessage { channel: "ch".into(), message_id: "m1".into(), content: "edited".into() }, FeatureId::EditMessage),
            (OutboundRequest::DeleteMessage { channel: "ch".into(), message_id: "m1".into() }, FeatureId::DeleteMessage),
            (OutboundRequest::ReadHistory { channel: "ch".into(), limit: Some(10), before: None }, FeatureId::ReadHistory),
            (OutboundRequest::PinMessage { channel: "ch".into(), message_id: "m1".into() }, FeatureId::PinMessage),
            (OutboundRequest::UnpinMessage { channel: "ch".into(), message_id: "m1".into() }, FeatureId::UnpinMessage),
            (OutboundRequest::BulkDeleteMessages { channel: "ch".into(), message_ids: vec!["m1".into(), "m2".into()] }, FeatureId::BulkDeleteMessages),
            (OutboundRequest::AddReaction { channel: "ch".into(), message_id: "m1".into(), emoji: "👍".into() }, FeatureId::AddReaction),
            (OutboundRequest::RemoveReaction { channel: "ch".into(), message_id: "m1".into(), emoji: "👍".into() }, FeatureId::RemoveReaction),
            (OutboundRequest::RemoveAllReactions { channel: "ch".into(), message_id: "m1".into() }, FeatureId::RemoveAllReactions),
            (OutboundRequest::SendAttachment { channel: "ch".into(), filename: "file.txt".into(), data: vec![1, 2, 3], content_type: Some("text/plain".into()) }, FeatureId::Attachments),
            (OutboundRequest::TypingIndicator { channel: "ch".into() }, FeatureId::TypingIndicator),
            (OutboundRequest::CreateThread { channel: "ch".into(), message_id: "m1".into(), name: "thread".into() }, FeatureId::CreateThread),
            (OutboundRequest::CreatePoll { channel: "ch".into(), question: "Vote?".into(), options: vec!["Yes".into(), "No".into()], duration_hours: Some(24) }, FeatureId::CreatePoll),
            (OutboundRequest::PollVote { channel: "ch".into(), poll_id: "p1".into(), option_index: 0 }, FeatureId::PollVote),
        ];
        for (request, expected_feature) in cases {
            assert_eq!(request.feature_id(), expected_feature, "Wrong feature_id for {:?}", request);
        }
    }

    #[test]
    fn test_outbound_request_serde_roundtrip() {
        let request = OutboundRequest::SendMessage {
            channel: "general".into(),
            content: "Hello!".into(),
            reply_to: Some("msg123".into()),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""send_message""#), "Should use snake_case: {}", json);
        let parsed: OutboundRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
    }

    #[test]
    fn test_outbound_request_base64_attachment() {
        let request = OutboundRequest::SendAttachment {
            channel: "ch".into(),
            filename: "test.bin".into(),
            data: vec![0x48, 0x65, 0x6c, 0x6c, 0x6f], // "Hello" in bytes
            content_type: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("SGVsbG8="), "Should contain base64 data: {}", json);
        let parsed: OutboundRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
    }
}

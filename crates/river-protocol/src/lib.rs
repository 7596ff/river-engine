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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_side_serde_roundtrip() {
        let left = Side::Left;
        let json = serde_json::to_string(&left).unwrap();
        assert_eq!(json, r#""left""#);
        let parsed: Side = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, left);

        let right = Side::Right;
        let json = serde_json::to_string(&right).unwrap();
        assert_eq!(json, r#""right""#);
        let parsed: Side = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, right);
    }

    #[test]
    fn test_baton_serde_roundtrip() {
        let actor = Baton::Actor;
        let json = serde_json::to_string(&actor).unwrap();
        assert_eq!(json, r#""actor""#);
        let parsed: Baton = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, actor);

        let spectator = Baton::Spectator;
        let json = serde_json::to_string(&spectator).unwrap();
        assert_eq!(json, r#""spectator""#);
        let parsed: Baton = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, spectator);
    }

    #[test]
    fn test_channel_serde_roundtrip() {
        let channel = Channel {
            adapter: "discord".to_string(),
            id: "123456789".to_string(),
            name: Some("general".to_string()),
        };
        let json = serde_json::to_string(&channel).unwrap();
        let parsed: Channel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, channel);

        // Test with None name
        let channel_no_name = Channel {
            adapter: "slack".to_string(),
            id: "C1234".to_string(),
            name: None,
        };
        let json = serde_json::to_string(&channel_no_name).unwrap();
        let parsed: Channel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, channel_no_name);
    }

    #[test]
    fn test_author_serde_roundtrip() {
        let author = Author {
            id: "user123".to_string(),
            name: "Alice".to_string(),
            bot: false,
        };
        let json = serde_json::to_string(&author).unwrap();
        let parsed: Author = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, author);

        let bot_author = Author {
            id: "bot456".to_string(),
            name: "Helper Bot".to_string(),
            bot: true,
        };
        let json = serde_json::to_string(&bot_author).unwrap();
        let parsed: Author = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, bot_author);
    }

    #[test]
    fn test_attachment_serde_roundtrip() {
        let attachment = Attachment {
            id: "attach123".to_string(),
            filename: "document.pdf".to_string(),
            url: "https://cdn.example.com/doc.pdf".to_string(),
            size: 1024000,
            content_type: Some("application/pdf".to_string()),
        };
        let json = serde_json::to_string(&attachment).unwrap();
        let parsed: Attachment = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, attachment);

        // Test with None content_type
        let attachment_no_type = Attachment {
            id: "attach456".to_string(),
            filename: "unknown.bin".to_string(),
            url: "https://cdn.example.com/file.bin".to_string(),
            size: 512,
            content_type: None,
        };
        let json = serde_json::to_string(&attachment_no_type).unwrap();
        let parsed: Attachment = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, attachment_no_type);
    }

    #[test]
    fn test_ground_serde_roundtrip() {
        let ground = Ground {
            name: "Cassie".to_string(),
            id: "user789".to_string(),
            channel: Channel {
                adapter: "discord".to_string(),
                id: "dm-channel-123".to_string(),
                name: Some("Direct Message".to_string()),
            },
        };
        let json = serde_json::to_string(&ground).unwrap();
        let parsed: Ground = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ground);
    }
}

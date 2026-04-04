//! River Protocol - Shared types for River Engine.
//!
//! This crate provides foundational types used across all River Engine crates.
//! It has no dependencies on other river-* crates.

pub mod conversation;
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

    #[test]
    fn test_process_entry_worker_roundtrip() {
        let entry = ProcessEntry::Worker {
            endpoint: "http://localhost:3001".to_string(),
            dyad: "river".to_string(),
            side: Side::Left,
            baton: Baton::Actor,
            model: "gpt-4".to_string(),
            ground: Ground {
                name: "Cassie".to_string(),
                id: "user123".to_string(),
                channel: Channel {
                    adapter: "discord".to_string(),
                    id: "ch123".to_string(),
                    name: Some("general".to_string()),
                },
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ProcessEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_process_entry_adapter_roundtrip() {
        let entry = ProcessEntry::Adapter {
            endpoint: "http://localhost:3002".to_string(),
            adapter_type: "discord".to_string(),
            dyad: "river".to_string(),
            features: vec![0, 1, 100, 200],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ProcessEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_process_entry_embed_roundtrip() {
        let entry = ProcessEntry::EmbedService {
            endpoint: "http://localhost:3003".to_string(),
            name: "embed-service".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ProcessEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_process_entry_tagged_discrimination() {
        // Verify the JSON has the correct "type" field for tagged enum
        let worker = ProcessEntry::Worker {
            endpoint: "http://localhost:3001".to_string(),
            dyad: "river".to_string(),
            side: Side::Left,
            baton: Baton::Actor,
            model: "gpt-4".to_string(),
            ground: Ground {
                name: "Cassie".to_string(),
                id: "user123".to_string(),
                channel: Channel {
                    adapter: "discord".to_string(),
                    id: "ch123".to_string(),
                    name: None,
                },
            },
        };
        let json = serde_json::to_string(&worker).unwrap();
        assert!(json.contains(r#""type":"worker""#), "JSON should contain type:worker tag: {}", json);

        let adapter = ProcessEntry::Adapter {
            endpoint: "http://localhost:3002".to_string(),
            adapter_type: "discord".to_string(),
            dyad: "river".to_string(),
            features: vec![0, 1],
        };
        let json = serde_json::to_string(&adapter).unwrap();
        assert!(json.contains(r#""type":"adapter""#), "JSON should contain type:adapter tag: {}", json);

        let embed = ProcessEntry::EmbedService {
            endpoint: "http://localhost:3003".to_string(),
            name: "embed".to_string(),
        };
        let json = serde_json::to_string(&embed).unwrap();
        assert!(json.contains(r#""type":"embed_service""#), "JSON should contain type:embed_service tag: {}", json);
    }

    #[test]
    fn test_registry_serde_roundtrip() {
        let registry = Registry {
            processes: vec![
                ProcessEntry::Worker {
                    endpoint: "http://localhost:3001".to_string(),
                    dyad: "river".to_string(),
                    side: Side::Left,
                    baton: Baton::Actor,
                    model: "gpt-4".to_string(),
                    ground: Ground {
                        name: "Cassie".to_string(),
                        id: "user123".to_string(),
                        channel: Channel {
                            adapter: "discord".to_string(),
                            id: "ch123".to_string(),
                            name: Some("general".to_string()),
                        },
                    },
                },
                ProcessEntry::Adapter {
                    endpoint: "http://localhost:3002".to_string(),
                    adapter_type: "discord".to_string(),
                    dyad: "river".to_string(),
                    features: vec![0, 1],
                },
            ],
        };
        let json = serde_json::to_string(&registry).unwrap();
        let parsed: Registry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, registry);
    }

    #[test]
    fn test_registry_default() {
        let registry = Registry::default();
        assert!(registry.processes.is_empty());
    }

    #[test]
    fn test_model_config_serde_roundtrip() {
        let config = ModelConfig {
            endpoint: "https://api.openai.com/v1".to_string(),
            name: "gpt-4-turbo".to_string(),
            api_key: "sk-test-key".to_string(),
            context_limit: 128000,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ModelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn test_worker_registration_serde_roundtrip() {
        let reg = WorkerRegistration {
            dyad: "river".to_string(),
            side: Side::Left,
        };
        let json = serde_json::to_string(&reg).unwrap();
        let parsed: WorkerRegistration = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reg);
    }

    #[test]
    fn test_worker_registration_request_serde_roundtrip() {
        let req = WorkerRegistrationRequest {
            endpoint: "http://localhost:3001".to_string(),
            worker: WorkerRegistration {
                dyad: "river".to_string(),
                side: Side::Right,
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        // WorkerRegistrationRequest only derives Serialize, test serialization works
        assert!(json.contains("endpoint"));
        assert!(json.contains("worker"));
    }

    #[test]
    fn test_worker_registration_response_serde_roundtrip() {
        let json = r#"{
            "accepted": true,
            "baton": "actor",
            "partner_endpoint": "http://localhost:3002",
            "model": {
                "endpoint": "https://api.openai.com",
                "name": "gpt-4",
                "api_key": "sk-key",
                "context_limit": 128000
            },
            "ground": {
                "name": "Cassie",
                "id": "user123",
                "channel": {
                    "adapter": "discord",
                    "id": "ch123",
                    "name": "general"
                }
            },
            "workspace": "/path/to/workspace",
            "initial_message": "Hello!",
            "start_sleeping": false
        }"#;
        let response: WorkerRegistrationResponse = serde_json::from_str(json).unwrap();
        assert!(response.accepted);
        assert_eq!(response.baton, Baton::Actor);
        assert_eq!(response.partner_endpoint, Some("http://localhost:3002".to_string()));
        assert_eq!(response.workspace, "/path/to/workspace");
    }

    #[test]
    fn test_adapter_registration_serde_roundtrip() {
        let reg = AdapterRegistration {
            adapter_type: "discord".to_string(),
            dyad: "river".to_string(),
            features: vec![0, 1, 100, 200, 300],
        };
        let json = serde_json::to_string(&reg).unwrap();
        // Verify the "type" rename works
        assert!(json.contains(r#""type":"discord""#), "Should rename adapter_type to type: {}", json);
        let parsed: AdapterRegistration = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reg);
    }

    #[test]
    fn test_adapter_registration_request_serde_roundtrip() {
        let req = AdapterRegistrationRequest {
            endpoint: "http://localhost:3002".to_string(),
            adapter: AdapterRegistration {
                adapter_type: "discord".to_string(),
                dyad: "river".to_string(),
                features: vec![0, 1],
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("endpoint"));
        assert!(json.contains("adapter"));
    }

    #[test]
    fn test_adapter_registration_response_serde_roundtrip() {
        let json = r#"{
            "accepted": true,
            "config": {"token": "discord-token", "guild_id": 123456},
            "worker_endpoint": "http://localhost:3001"
        }"#;
        let response: AdapterRegistrationResponse = serde_json::from_str(json).unwrap();
        assert!(response.accepted);
        assert_eq!(response.worker_endpoint, "http://localhost:3001");
        assert!(response.config.is_object());
    }
}

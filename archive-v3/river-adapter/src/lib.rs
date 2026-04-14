//! River Adapter — shared types for communication adapters

pub mod types;
pub mod capabilities;
pub mod registration;
pub mod traits;
pub mod error;
pub mod http;

pub use types::{IncomingEvent, EventType, Author, SendRequest, SendOptions, SendResponse};
pub use capabilities::Feature;
pub use registration::{AdapterInfo, RegisterRequest, RegisterResponse};
pub use traits::Adapter;
pub use error::AdapterError;
pub use http::HttpAdapter;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_event_type_serialization() {
        let event_type = EventType::MessageCreate;
        let json = serde_json::to_string(&event_type).unwrap();
        assert!(json.contains("MessageCreate"));
    }

    #[test]
    fn test_adapter_info_creation() {
        let info = AdapterInfo {
            name: "test".into(),
            version: "1.0.0".into(),
            url: "http://localhost:3000".into(),
            features: HashSet::from([Feature::ReadHistory, Feature::Reactions]),
            metadata: serde_json::json!({}),
        };
        assert_eq!(info.name, "test");
        assert!(info.features.contains(&Feature::ReadHistory));
    }

    #[test]
    fn test_send_request_serialization() {
        let request = SendRequest {
            channel: "123".into(),
            content: "Hello".into(),
            options: SendOptions::default(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"channel\":\"123\""));
        assert!(json.contains("\"content\":\"Hello\""));
    }
}

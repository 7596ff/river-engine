//! HTTP server for outbound messages and admin API

use serde::{Deserialize, Serialize};

/// Send message request from gateway
#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub channel: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub create_thread: Option<String>,
    #[serde(default)]
    pub reaction: Option<String>,
}

impl SendRequest {
    /// Validate the request
    pub fn validate(&self) -> Result<(), &'static str> {
        // Must have content or reaction
        if self.content.is_none() && self.reaction.is_none() {
            return Err("must provide content or reaction");
        }

        // content and reaction are mutually exclusive
        if self.content.is_some() && self.reaction.is_some() {
            return Err("content and reaction are mutually exclusive");
        }

        // reply_to and create_thread are mutually exclusive
        if self.reply_to.is_some() && self.create_thread.is_some() {
            return Err("reply_to and create_thread are mutually exclusive");
        }

        Ok(())
    }
}

/// Send message response
#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Add channel request
#[derive(Debug, Deserialize)]
pub struct AddChannelRequest {
    pub channel_id: String,
}

/// Channel operation response
#[derive(Debug, Serialize)]
pub struct ChannelResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// List channels response
#[derive(Debug, Serialize)]
pub struct ListChannelsResponse {
    pub channels: Vec<String>,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub discord: &'static str,
    pub gateway: &'static str,
    pub channel_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_request_validation_valid_content() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_send_request_validation_valid_reaction() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: None,
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: Some("\u{1F44D}".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_send_request_validation_no_content_or_reaction() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: None,
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: None,
        };
        assert_eq!(req.validate().unwrap_err(), "must provide content or reaction");
    }

    #[test]
    fn test_send_request_validation_both_content_and_reaction() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: Some("\u{1F44D}".to_string()),
        };
        assert_eq!(req.validate().unwrap_err(), "content and reaction are mutually exclusive");
    }

    #[test]
    fn test_send_request_validation_reply_and_thread() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: Some("msg1".to_string()),
            thread_id: None,
            create_thread: Some("New Thread".to_string()),
            reaction: None,
        };
        assert_eq!(req.validate().unwrap_err(), "reply_to and create_thread are mutually exclusive");
    }
}

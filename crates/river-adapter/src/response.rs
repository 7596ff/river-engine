//! Outbound response types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use river_protocol::Author;

/// Response from adapter execute endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct OutboundResponse {
    /// Whether the request succeeded.
    pub ok: bool,
    /// Response data on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ResponseData>,
    /// Error details on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

impl OutboundResponse {
    /// Create a successful response with data.
    pub fn success(data: ResponseData) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    /// Create a failure response with error.
    pub fn failure(error: ResponseError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Response data variants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResponseData {
    MessageSent { message_id: String },
    MessageEdited { message_id: String },
    MessageDeleted,
    MessagesPinned,
    MessagesUnpinned,
    MessagesDeleted { count: usize },
    ReactionAdded,
    ReactionRemoved,
    ReactionsCleared,
    AttachmentSent { message_id: String },
    TypingStarted,
    History { messages: Vec<HistoryMessage> },
    ThreadCreated { thread_id: String },
    PollCreated { poll_id: String },
    PollVoted,
}

/// Message from history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct HistoryMessage {
    pub message_id: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    pub timestamp: String,
}

/// Error response details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ResponseError {
    pub code: ErrorCode,
    pub message: String,
}

impl ResponseError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Error codes for adapter responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnsupportedFeature,
    InvalidPayload,
    PlatformError,
    RateLimited,
    NotFound,
    Unauthorized,
}

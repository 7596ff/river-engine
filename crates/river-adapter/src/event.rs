//! Inbound event types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::author::{Attachment, Author};

/// Inbound event from adapter to worker.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct InboundEvent {
    /// Adapter type (e.g., "discord", "slack")
    pub adapter: String,
    /// Event-specific metadata
    pub metadata: EventMetadata,
}

/// Lightweight enum for event type identification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    MessageCreate,
    MessageUpdate,
    MessageDelete,
    ReactionAdd,
    ReactionRemove,
    TypingStart,
    MemberJoin,
    MemberLeave,
    PresenceUpdate,
    VoiceStateUpdate,
    ChannelCreate,
    ChannelUpdate,
    ChannelDelete,
    ThreadCreate,
    ThreadUpdate,
    ThreadDelete,
    PinUpdate,
    PollVote,
    ScheduledEvent,
    ConnectionLost,
    ConnectionRestored,
    Unknown,
}

/// Data-carrying enum with per-event-type fields.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventMetadata {
    MessageCreate {
        channel: String,
        author: Author,
        content: String,
        message_id: String,
        timestamp: String,
        reply_to: Option<String>,
        attachments: Vec<Attachment>,
    },
    MessageUpdate {
        channel: String,
        message_id: String,
        content: String,
        timestamp: String,
    },
    MessageDelete {
        channel: String,
        message_id: String,
    },
    ReactionAdd {
        channel: String,
        message_id: String,
        user_id: String,
        emoji: String,
    },
    ReactionRemove {
        channel: String,
        message_id: String,
        user_id: String,
        emoji: String,
    },
    TypingStart {
        channel: String,
        user_id: String,
    },
    MemberJoin {
        user_id: String,
        username: String,
    },
    MemberLeave {
        user_id: String,
    },
    PresenceUpdate {
        user_id: String,
        status: String,
    },
    VoiceStateUpdate {
        user_id: String,
        channel: Option<String>,
    },
    ChannelCreate {
        channel: String,
        name: String,
    },
    ChannelUpdate {
        channel: String,
        name: String,
    },
    ChannelDelete {
        channel: String,
    },
    ThreadCreate {
        channel: String,
        parent_channel: String,
        name: String,
    },
    ThreadUpdate {
        channel: String,
        name: String,
    },
    ThreadDelete {
        channel: String,
    },
    PinUpdate {
        channel: String,
        message_id: String,
        pinned: bool,
    },
    PollVote {
        channel: String,
        poll_id: String,
        user_id: String,
        option_index: u32,
        added: bool,
    },
    ScheduledEvent {
        event_id: String,
        name: String,
        start_time: String,
    },
    ConnectionLost {
        reason: String,
        reconnecting: bool,
    },
    ConnectionRestored {
        downtime_seconds: u64,
    },
    Unknown(serde_json::Value),
}

impl EventMetadata {
    /// Get the event type for this metadata.
    pub fn event_type(&self) -> EventType {
        match self {
            Self::MessageCreate { .. } => EventType::MessageCreate,
            Self::MessageUpdate { .. } => EventType::MessageUpdate,
            Self::MessageDelete { .. } => EventType::MessageDelete,
            Self::ReactionAdd { .. } => EventType::ReactionAdd,
            Self::ReactionRemove { .. } => EventType::ReactionRemove,
            Self::TypingStart { .. } => EventType::TypingStart,
            Self::MemberJoin { .. } => EventType::MemberJoin,
            Self::MemberLeave { .. } => EventType::MemberLeave,
            Self::PresenceUpdate { .. } => EventType::PresenceUpdate,
            Self::VoiceStateUpdate { .. } => EventType::VoiceStateUpdate,
            Self::ChannelCreate { .. } => EventType::ChannelCreate,
            Self::ChannelUpdate { .. } => EventType::ChannelUpdate,
            Self::ChannelDelete { .. } => EventType::ChannelDelete,
            Self::ThreadCreate { .. } => EventType::ThreadCreate,
            Self::ThreadUpdate { .. } => EventType::ThreadUpdate,
            Self::ThreadDelete { .. } => EventType::ThreadDelete,
            Self::PinUpdate { .. } => EventType::PinUpdate,
            Self::PollVote { .. } => EventType::PollVote,
            Self::ScheduledEvent { .. } => EventType::ScheduledEvent,
            Self::ConnectionLost { .. } => EventType::ConnectionLost,
            Self::ConnectionRestored { .. } => EventType::ConnectionRestored,
            Self::Unknown(_) => EventType::Unknown,
        }
    }
}

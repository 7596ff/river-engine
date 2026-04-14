//! Feature system for adapter capabilities.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Lightweight enum for registration and capability checks.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeatureId {
    // === Core messaging (0-9) ===
    /// Required: adapter must support outbound messages
    SendMessage = 0,
    /// Required: adapter must forward inbound events
    ReceiveMessage = 1,

    // === Message operations (10-19) ===
    EditMessage = 10,
    DeleteMessage = 11,
    ReadHistory = 12,
    PinMessage = 13,
    UnpinMessage = 14,
    BulkDeleteMessages = 15,

    // === Reactions (20-29) ===
    AddReaction = 20,
    RemoveReaction = 21,
    RemoveAllReactions = 22,

    // === Attachments (30-39) ===
    Attachments = 30,

    // === Typing (40-49) ===
    TypingIndicator = 40,

    // === Threads (50-59) ===
    CreateThread = 50,
    ThreadEvents = 51,

    // === Polls (60-69) ===
    CreatePoll = 60,
    PollVote = 61,
    PollEvents = 62,

    // === Situational awareness (100-109) ===
    VoiceStateEvents = 100,
    PresenceEvents = 101,
    MemberEvents = 102,
    ScheduledEvents = 103,

    // === Server admin (200-209) ===
    ChannelEvents = 200,

    // === Connection (900-909) ===
    ConnectionEvents = 900,
}

impl FeatureId {
    /// Check if this feature is required for all adapters.
    pub fn is_required(&self) -> bool {
        matches!(self, Self::SendMessage | Self::ReceiveMessage)
    }
}

impl TryFrom<u16> for FeatureId {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::SendMessage),
            1 => Ok(Self::ReceiveMessage),
            10 => Ok(Self::EditMessage),
            11 => Ok(Self::DeleteMessage),
            12 => Ok(Self::ReadHistory),
            13 => Ok(Self::PinMessage),
            14 => Ok(Self::UnpinMessage),
            15 => Ok(Self::BulkDeleteMessages),
            20 => Ok(Self::AddReaction),
            21 => Ok(Self::RemoveReaction),
            22 => Ok(Self::RemoveAllReactions),
            30 => Ok(Self::Attachments),
            40 => Ok(Self::TypingIndicator),
            50 => Ok(Self::CreateThread),
            51 => Ok(Self::ThreadEvents),
            60 => Ok(Self::CreatePoll),
            61 => Ok(Self::PollVote),
            62 => Ok(Self::PollEvents),
            100 => Ok(Self::VoiceStateEvents),
            101 => Ok(Self::PresenceEvents),
            102 => Ok(Self::MemberEvents),
            103 => Ok(Self::ScheduledEvents),
            200 => Ok(Self::ChannelEvents),
            900 => Ok(Self::ConnectionEvents),
            _ => Err(value),
        }
    }
}

/// Data-carrying enum with typed payloads for outbound requests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutboundRequest {
    SendMessage {
        channel: String,
        content: String,
        reply_to: Option<String>,
    },
    EditMessage {
        channel: String,
        message_id: String,
        content: String,
    },
    DeleteMessage {
        channel: String,
        message_id: String,
    },
    ReadHistory {
        channel: String,
        limit: Option<u32>,
        before: Option<String>,
        after: Option<String>,
    },
    PinMessage {
        channel: String,
        message_id: String,
    },
    UnpinMessage {
        channel: String,
        message_id: String,
    },
    BulkDeleteMessages {
        channel: String,
        message_ids: Vec<String>,
    },
    AddReaction {
        channel: String,
        message_id: String,
        emoji: String,
    },
    RemoveReaction {
        channel: String,
        message_id: String,
        emoji: String,
    },
    RemoveAllReactions {
        channel: String,
        message_id: String,
    },
    SendAttachment {
        channel: String,
        filename: String,
        #[serde(with = "base64_bytes")]
        data: Vec<u8>,
        content_type: Option<String>,
    },
    TypingIndicator {
        channel: String,
    },
    CreateThread {
        channel: String,
        message_id: String,
        name: String,
    },
    CreatePoll {
        channel: String,
        question: String,
        options: Vec<String>,
        duration_hours: Option<u32>,
    },
    PollVote {
        channel: String,
        poll_id: String,
        option_index: u32,
    },
}

impl OutboundRequest {
    /// Get the feature ID for this request.
    pub fn feature_id(&self) -> FeatureId {
        match self {
            Self::SendMessage { .. } => FeatureId::SendMessage,
            Self::EditMessage { .. } => FeatureId::EditMessage,
            Self::DeleteMessage { .. } => FeatureId::DeleteMessage,
            Self::ReadHistory { .. } => FeatureId::ReadHistory,
            Self::PinMessage { .. } => FeatureId::PinMessage,
            Self::UnpinMessage { .. } => FeatureId::UnpinMessage,
            Self::BulkDeleteMessages { .. } => FeatureId::BulkDeleteMessages,
            Self::AddReaction { .. } => FeatureId::AddReaction,
            Self::RemoveReaction { .. } => FeatureId::RemoveReaction,
            Self::RemoveAllReactions { .. } => FeatureId::RemoveAllReactions,
            Self::SendAttachment { .. } => FeatureId::Attachments,
            Self::TypingIndicator { .. } => FeatureId::TypingIndicator,
            Self::CreateThread { .. } => FeatureId::CreateThread,
            Self::CreatePoll { .. } => FeatureId::CreatePoll,
            Self::PollVote { .. } => FeatureId::PollVote,
        }
    }
}

mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        use base64::Engine;
        let s = base64::engine::general_purpose::STANDARD.encode(bytes);
        s.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        use base64::Engine;
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

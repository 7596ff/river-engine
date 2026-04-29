//! Feature flags for adapter capabilities

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Feature {
    ReadHistory,
    Reactions,
    Threads,
    Attachments,
    Embeds,
    TypingIndicator,
    EditMessage,
    DeleteMessage,
    Custom(String),
}

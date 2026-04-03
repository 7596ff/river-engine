//! Identity types for River Engine entities.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Message author information.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Author {
    /// Unique identifier for the author.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether this is a bot account.
    pub bot: bool,
}

/// Communication channel identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct Channel {
    /// Adapter type (e.g., "discord", "slack").
    pub adapter: String,
    /// Channel identifier.
    pub id: String,
    /// Human-readable channel name.
    pub name: Option<String>,
}

/// File attachment metadata.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Attachment {
    /// Unique identifier.
    pub id: String,
    /// Original filename.
    pub filename: String,
    /// URL to download the attachment.
    pub url: String,
    /// File size in bytes.
    pub size: u64,
    /// MIME content type.
    pub content_type: Option<String>,
}

/// Worker role (actor or spectator).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Baton {
    /// Actor: handles external communication
    Actor,
    /// Spectator: manages memory and reviews
    Spectator,
}

/// Fixed position in the dyad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// Get the opposite side.
    pub fn opposite(&self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Human operator information.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Ground {
    /// Human operator name.
    pub name: String,
    /// Human operator platform ID.
    pub id: String,
    /// Channel for reaching the human.
    pub channel: Channel,
}

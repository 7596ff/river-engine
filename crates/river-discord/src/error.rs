//! Discord adapter error types.

/// Errors that can occur in Discord adapter operations.
#[derive(Debug, thiserror::Error)]
pub enum DiscordAdapterError {
    #[error("Invalid emoji format: {0}")]
    InvalidEmojiFormat(String),

    #[error("Invalid emoji ID: {0}")]
    InvalidEmojiId(String),

    #[error("Twilight HTTP error: {0}")]
    TwilightError(#[from] twilight_http::Error),
}

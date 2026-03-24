//! Discord event handling

use crate::channels::ChannelState;
use crate::gateway::GatewayClient;
use chrono::{DateTime, Utc};
use river_adapter::{Author, EventType, IncomingEvent};
use std::sync::Arc;
use twilight_model::channel::message::EmojiReactionType;
use twilight_model::gateway::payload::incoming::{MessageCreate, ReactionAdd};

/// Handles Discord events and forwards to gateway
pub struct EventHandler {
    channels: Arc<ChannelState>,
    gateway: Arc<GatewayClient>,
}

impl EventHandler {
    /// Create a new event handler
    pub fn new(channels: Arc<ChannelState>, gateway: Arc<GatewayClient>) -> Self {
        Self { channels, gateway }
    }

    /// Handle a message create event
    pub async fn handle_message(&self, msg: Box<MessageCreate>) {
        let channel_id = msg.channel_id.get();
        let is_dm = msg.guild_id.is_none();

        // Allow DMs, or check if we're listening to this guild channel
        if !is_dm && !self.channels.contains(channel_id).await {
            return;
        }

        // Build the event
        // Convert Twilight timestamp to chrono DateTime
        let timestamp: DateTime<Utc> = DateTime::from_timestamp(msg.timestamp.as_secs(), 0)
            .unwrap_or_else(Utc::now);

        let event = IncomingEvent {
            adapter: "discord".into(),
            event_type: EventType::MessageCreate,
            channel: channel_id.to_string(),
            channel_name: None, // TODO: Cache channel names from Discord
            author: Author {
                id: msg.author.id.get().to_string(),
                name: msg.author.name.clone(),
                is_bot: msg.author.bot,
            },
            content: msg.content.clone(),
            message_id: msg.id.get().to_string(),
            timestamp,
            metadata: serde_json::json!({
                "guild_id": msg.guild_id.map(|id| id.get().to_string()),
                "thread_id": msg.thread.as_ref().map(|t| t.id.get().to_string()),
                "reply_to": msg.referenced_message.as_ref().map(|m| m.id.get().to_string()),
            }),
        };

        // Send to gateway
        if let Err(e) = self.gateway.send_incoming(event).await {
            tracing::error!("Failed to forward message to gateway: {}", e);
        } else {
            tracing::info!("Forwarded message to gateway");
        }
    }

    /// Handle a reaction add event
    pub async fn handle_reaction(&self, reaction: Box<ReactionAdd>) {
        let channel_id = reaction.channel_id.get();
        let is_dm = reaction.guild_id.is_none();

        // Allow DMs, or check if we're listening to this guild channel
        if !is_dm && !self.channels.contains(channel_id).await {
            return;
        }

        // Get user info - in twilight 0.16, member.user is User not Option<User>
        let (user_id, user_name) = match &reaction.member {
            Some(member) => {
                let name = member.nick.as_ref()
                    .cloned()
                    .unwrap_or_else(|| member.user.name.clone());
                let id = member.user.id.get().to_string();
                (id, name)
            }
            None => (reaction.user_id.get().to_string(), "Unknown".to_string()),
        };

        // Get emoji string
        let emoji = match &reaction.emoji {
            EmojiReactionType::Custom { id, name, .. } => {
                name.clone().unwrap_or_else(|| format!("<:emoji:{}>", id))
            }
            EmojiReactionType::Unicode { name } => name.clone(),
        };

        // Get whether the user is a bot (from member info if available)
        let is_bot = reaction.member.as_ref()
            .map(|m| m.user.bot)
            .unwrap_or(false);

        let event = IncomingEvent {
            adapter: "discord".into(),
            event_type: EventType::ReactionAdd,
            channel: channel_id.to_string(),
            channel_name: None, // TODO: Cache channel names from Discord
            author: Author {
                id: user_id,
                name: user_name,
                is_bot,
            },
            content: emoji,
            message_id: reaction.message_id.get().to_string(),
            timestamp: Utc::now(), // Reactions don't have timestamps in Discord
            metadata: serde_json::json!({
                "guild_id": reaction.guild_id.map(|id| id.get().to_string()),
            }),
        };

        if let Err(e) = self.gateway.send_incoming(event).await {
            tracing::error!("Failed to forward reaction to gateway: {}", e);
        } else {
            tracing::info!("Forwarded reaction to gateway");
        }
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require mocking Twilight types
    // These are covered by manual testing with live Discord
}

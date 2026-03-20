//! Discord event handling

use crate::channels::ChannelState;
use crate::gateway::{Author, EventMetadata, GatewayClient, IncomingEvent};
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
        let event = IncomingEvent {
            adapter: "discord",
            event_type: "message".to_string(),
            channel: channel_id.to_string(),
            channel_name: None, // TODO: Cache channel names from Discord
            guild_id: msg.guild_id.map(|id| id.get().to_string()),
            guild_name: None, // TODO: Cache guild names from Discord
            author: Author {
                id: msg.author.id.get().to_string(),
                name: msg.author.name.clone(),
            },
            content: msg.content.clone(),
            message_id: msg.id.get().to_string(),
            metadata: EventMetadata {
                guild_id: msg.guild_id.map(|id| id.get().to_string()),
                // If message has/created a thread, capture its ID
                thread_id: msg.thread.as_ref().map(|t| t.id.get().to_string()),
                reply_to: msg.referenced_message.as_ref().map(|m| m.id.get().to_string()),
            },
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

        let event = IncomingEvent {
            adapter: "discord",
            event_type: "reaction_add".to_string(),
            channel: channel_id.to_string(),
            channel_name: None, // TODO: Cache channel names from Discord
            guild_id: reaction.guild_id.map(|id| id.get().to_string()),
            guild_name: None, // TODO: Cache guild names from Discord
            author: Author {
                id: user_id,
                name: user_name,
            },
            content: emoji,
            message_id: reaction.message_id.get().to_string(),
            metadata: EventMetadata {
                guild_id: reaction.guild_id.map(|id| id.get().to_string()),
                thread_id: None,
                reply_to: None,
            },
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

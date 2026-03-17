//! Twilight Discord client wrapper

use std::sync::Arc;
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_http::Client as HttpClient;
use twilight_model::id::{marker::GuildMarker, Id};

/// Discord client wrapping Twilight components
pub struct DiscordClient {
    pub http: Arc<HttpClient>,
    pub shard: Shard,
    pub guild_id: Id<GuildMarker>,
}

impl DiscordClient {
    /// Create a new Discord client
    pub async fn new(token: &str, guild_id: u64) -> anyhow::Result<Self> {
        let http = Arc::new(HttpClient::new(token.to_string()));

        let intents = Intents::GUILDS
            | Intents::GUILD_MESSAGES
            | Intents::GUILD_MESSAGE_REACTIONS
            | Intents::MESSAGE_CONTENT
            | Intents::DIRECT_MESSAGES;

        let shard = Shard::new(ShardId::ONE, token.to_string(), intents);

        Ok(Self {
            http,
            shard,
            guild_id: Id::new(guild_id),
        })
    }

    /// Receive the next event from Discord
    pub async fn next_event(&mut self) -> Option<Event> {
        // next_event returns Option<Result<Event, ReceiveMessageError>>
        match self.shard.next_event(EventTypeFlags::all()).await {
            Some(Ok(event)) => Some(event),
            Some(Err(e)) => {
                tracing::warn!("Error receiving Discord event: {}", e);
                None
            }
            None => None,
        }
    }

    /// Send a message to a channel
    pub async fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        reply_to: Option<u64>,
    ) -> anyhow::Result<u64> {
        use twilight_model::id::{marker::ChannelMarker, marker::MessageMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);

        let mut request = self.http.create_message(channel).content(content);

        if let Some(msg_id) = reply_to {
            let msg: Id<MessageMarker> = Id::new(msg_id);
            request = request.reply(msg);
        }

        let response = request.await?;
        let message = response.model().await?;

        Ok(message.id.get())
    }

    /// Add a reaction to a message
    pub async fn add_reaction(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> anyhow::Result<()> {
        use twilight_http::request::channel::reaction::RequestReactionType;
        use twilight_model::id::{marker::ChannelMarker, marker::MessageMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);
        let message: Id<MessageMarker> = Id::new(message_id);

        let reaction = RequestReactionType::Unicode { name: emoji };

        self.http
            .create_reaction(channel, message, &reaction)
            .await?;

        Ok(())
    }

    /// Check if connected to Discord
    pub fn is_connected(&self) -> bool {
        // is_identified returns true if Active or Resuming
        self.shard.state().is_identified()
    }
}

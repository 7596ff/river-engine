//! Twilight Discord client wrapper

use std::sync::Arc;
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_http::Client as HttpClient;
use twilight_model::id::{marker::GuildMarker, Id};

use crate::outbound::{ReadMessage, ReadReaction};

/// Shared Discord HTTP client (for sending messages)
/// Can be cloned and shared across tasks
#[derive(Clone)]
pub struct DiscordSender {
    pub http: Arc<HttpClient>,
    pub guild_id: Id<GuildMarker>,
}

impl DiscordSender {
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

    /// Create a thread from a message
    pub async fn create_thread(
        &self,
        channel_id: u64,
        message_id: u64,
        name: &str,
    ) -> anyhow::Result<u64> {
        use twilight_model::id::{marker::ChannelMarker, marker::MessageMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);
        let message: Id<MessageMarker> = Id::new(message_id);

        let response = self
            .http
            .create_thread_from_message(channel, message, name)
            .await?;
        let thread = response.model().await?;

        Ok(thread.id.get())
    }

    /// Read messages from a channel
    pub async fn read_messages(
        &self,
        channel_id: u64,
        limit: u16,
        before: Option<u64>,
    ) -> anyhow::Result<Vec<ReadMessage>> {
        use twilight_model::channel::message::EmojiReactionType;
        use twilight_model::id::{marker::ChannelMarker, marker::MessageMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);

        let request = self.http.channel_messages(channel).limit(limit);

        let response = if let Some(before_id) = before {
            let msg: Id<MessageMarker> = Id::new(before_id);
            request.before(msg).await?
        } else {
            request.await?
        };

        let messages = response.models().await?;

        Ok(messages
            .into_iter()
            .map(|m| ReadMessage {
                id: m.id.to_string(),
                author_id: m.author.id.to_string(),
                author_name: m.author.name.clone(),
                content: m.content.clone(),
                timestamp: m.timestamp.as_secs(),
                is_bot: m.author.bot,
                reactions: m
                    .reactions
                    .iter()
                    .map(|r| ReadReaction {
                        emoji: match &r.emoji {
                            EmojiReactionType::Unicode { name } => name.clone(),
                            EmojiReactionType::Custom { id, name, .. } => {
                                format!("<:{}:{}>", name.as_deref().unwrap_or(""), id)
                            }
                        },
                        count: r.count as usize,
                    })
                    .collect(),
            })
            .collect())
    }

    /// Send a typing indicator to a channel
    pub async fn trigger_typing(&self, channel_id: u64) -> anyhow::Result<()> {
        use twilight_model::id::{marker::ChannelMarker, Id};

        let channel: Id<ChannelMarker> = Id::new(channel_id);
        self.http.create_typing_trigger(channel).await?;
        Ok(())
    }
}

/// Discord client with gateway shard (for receiving events)
pub struct DiscordClient {
    pub sender: DiscordSender,
    pub shard: Shard,
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
            sender: DiscordSender {
                http,
                guild_id: Id::new(guild_id),
            },
            shard,
        })
    }

    /// Get the sender (can be cloned and shared)
    pub fn sender(&self) -> DiscordSender {
        self.sender.clone()
    }

    /// Get the HTTP client (for slash command registration)
    pub fn http(&self) -> &Arc<HttpClient> {
        &self.sender.http
    }

    /// Get the guild ID
    pub fn guild_id(&self) -> Id<GuildMarker> {
        self.sender.guild_id
    }

    /// Receive the next event from Discord
    pub async fn next_event(&mut self) -> Option<Event> {
        match self.shard.next_event(EventTypeFlags::all()).await {
            Some(Ok(event)) => Some(event),
            Some(Err(e)) => {
                tracing::warn!("Error receiving Discord event: {}", e);
                None
            }
            None => None,
        }
    }

    /// Check if connected to Discord
    pub fn is_connected(&self) -> bool {
        self.shard.state().is_identified()
    }
}

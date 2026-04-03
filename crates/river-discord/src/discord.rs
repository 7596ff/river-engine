//! Discord gateway client using twilight.

use crate::DiscordConfig;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use river_adapter::{
    Adapter, AdapterError, Attachment, Author, ErrorCode, EventMetadata, FeatureId, InboundEvent,
    OutboundRequest, OutboundResponse, ResponseData, ResponseError,
};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use twilight_gateway::{Event, Intents, Shard, ShardId, StreamExt};
use twilight_http::Client as HttpClient;
use twilight_model::channel::message::{EmojiReactionType, MessageType};
use twilight_model::id::marker::{ChannelMarker, MessageMarker};
use twilight_model::id::Id;

/// Discord client wrapping twilight gateway and HTTP.
pub struct DiscordClient {
    http: Arc<HttpClient>,
    event_rx: Arc<RwLock<mpsc::Receiver<InboundEvent>>>,
    connected: Arc<RwLock<bool>>,
    adapter_name: String,
}

impl DiscordClient {
    /// Create a new Discord client.
    pub async fn new(
        config: DiscordConfig,
        adapter_name: String,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let intents = Intents::from_bits_truncate(
            config.intents.unwrap_or(
                Intents::GUILD_MESSAGES.bits()
                    | Intents::MESSAGE_CONTENT.bits()
                    | Intents::GUILD_MESSAGE_REACTIONS.bits()
                    | Intents::GUILD_MESSAGE_TYPING.bits()
                    | Intents::DIRECT_MESSAGES.bits(),
            ),
        );

        let mut shard = Shard::new(ShardId::ONE, config.token.clone(), intents);
        let http = Arc::new(HttpClient::new(config.token));
        let (event_tx, event_rx) = mpsc::channel::<InboundEvent>(256);
        let connected = Arc::new(RwLock::new(true));

        // Spawn gateway event loop
        let connected_clone = connected.clone();
        let adapter_name_clone = adapter_name.clone();

        tokio::spawn(async move {
            tracing::info!("Starting Discord gateway event loop");

            while let Some(event) = shard.next_event(twilight_gateway::EventTypeFlags::all()).await
            {
                match event {
                    Ok(event) => {
                        if let Some(inbound) = convert_event(&adapter_name_clone, event) {
                            if event_tx.send(inbound).await.is_err() {
                                tracing::warn!("Event channel closed");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Gateway error: {:?}", e);
                        // Mark as disconnected on error
                        let mut c = connected_clone.write().await;
                        *c = false;
                        break;
                    }
                }
            }

            tracing::info!("Gateway event loop ended");
        });

        Ok(Self {
            http,
            event_rx: Arc::new(RwLock::new(event_rx)),
            connected,
            adapter_name,
        })
    }

    /// Poll for new events from the gateway.
    pub async fn poll_events(&self) -> Vec<InboundEvent> {
        let mut events = Vec::new();
        let mut rx = self.event_rx.write().await;

        // Drain available events without blocking
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        events
    }

    /// Execute an outbound request (internal implementation).
    async fn execute_impl(&self, request: OutboundRequest) -> OutboundResponse {
        match request {
            OutboundRequest::SendMessage {
                channel,
                content,
                reply_to,
            } => {
                let channel_id = match channel.parse::<u64>() {
                    Ok(id) => Id::<ChannelMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid channel ID")
                    }
                };

                let mut builder = self.http.create_message(channel_id).content(&content);

                if let Some(ref reply_id) = reply_to {
                    if let Ok(msg_id) = reply_id.parse::<u64>() {
                        builder = builder.reply(Id::<MessageMarker>::new(msg_id));
                    }
                }

                match builder.await {
                    Ok(response) => match response.model().await {
                        Ok(msg) => OutboundResponse {
                            ok: true,
                            data: Some(ResponseData::MessageSent {
                                message_id: msg.id.to_string(),
                            }),
                            error: None,
                        },
                        Err(e) => error_response(ErrorCode::PlatformError, &e.to_string()),
                    },
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
                }
            }
            OutboundRequest::EditMessage {
                channel,
                message_id,
                content,
            } => {
                let channel_id = match channel.parse::<u64>() {
                    Ok(id) => Id::<ChannelMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid channel ID")
                    }
                };
                let msg_id = match message_id.parse::<u64>() {
                    Ok(id) => Id::<MessageMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid message ID")
                    }
                };

                let builder = self
                    .http
                    .update_message(channel_id, msg_id)
                    .content(Some(&content));

                match builder.await {
                    Ok(_) => OutboundResponse {
                        ok: true,
                        data: Some(ResponseData::MessageEdited { message_id }),
                        error: None,
                    },
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
                }
            }
            OutboundRequest::DeleteMessage { channel, message_id } => {
                let channel_id = match channel.parse::<u64>() {
                    Ok(id) => Id::<ChannelMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid channel ID")
                    }
                };
                let msg_id = match message_id.parse::<u64>() {
                    Ok(id) => Id::<MessageMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid message ID")
                    }
                };

                match self.http.delete_message(channel_id, msg_id).await {
                    Ok(_) => OutboundResponse {
                        ok: true,
                        data: Some(ResponseData::MessageDeleted),
                        error: None,
                    },
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
                }
            }
            OutboundRequest::AddReaction {
                channel,
                message_id,
                emoji,
            } => {
                let channel_id = match channel.parse::<u64>() {
                    Ok(id) => Id::<ChannelMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid channel ID")
                    }
                };
                let msg_id = match message_id.parse::<u64>() {
                    Ok(id) => Id::<MessageMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid message ID")
                    }
                };

                let request_emoji = parse_emoji(&emoji);

                match self
                    .http
                    .create_reaction(channel_id, msg_id, &request_emoji)
                    .await
                {
                    Ok(_) => OutboundResponse {
                        ok: true,
                        data: Some(ResponseData::ReactionAdded),
                        error: None,
                    },
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
                }
            }
            OutboundRequest::RemoveReaction {
                channel,
                message_id,
                emoji,
            } => {
                let channel_id = match channel.parse::<u64>() {
                    Ok(id) => Id::<ChannelMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid channel ID")
                    }
                };
                let msg_id = match message_id.parse::<u64>() {
                    Ok(id) => Id::<MessageMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid message ID")
                    }
                };

                let request_emoji = parse_emoji(&emoji);

                match self
                    .http
                    .delete_current_user_reaction(channel_id, msg_id, &request_emoji)
                    .await
                {
                    Ok(_) => OutboundResponse {
                        ok: true,
                        data: Some(ResponseData::ReactionRemoved),
                        error: None,
                    },
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
                }
            }
            OutboundRequest::TypingIndicator { channel } => {
                let channel_id = match channel.parse::<u64>() {
                    Ok(id) => Id::<ChannelMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid channel ID")
                    }
                };

                match self.http.create_typing_trigger(channel_id).await {
                    Ok(_) => OutboundResponse {
                        ok: true,
                        data: Some(ResponseData::TypingStarted),
                        error: None,
                    },
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
                }
            }
            OutboundRequest::ReadHistory {
                channel,
                limit,
                before,
            } => {
                let channel_id = match channel.parse::<u64>() {
                    Ok(id) => Id::<ChannelMarker>::new(id),
                    Err(_) => {
                        return error_response(ErrorCode::InvalidPayload, "Invalid channel ID")
                    }
                };

                let builder = self.http.channel_messages(channel_id);

                // Apply limit
                let builder = if let Some(l) = limit {
                    builder.limit(l.min(100) as u16)
                } else {
                    builder
                };

                // Apply before - this converts GetChannelMessages to GetChannelMessagesConfigured
                let response = if let Some(ref before_id) = before {
                    if let Ok(msg_id) = before_id.parse::<u64>() {
                        builder.before(Id::<MessageMarker>::new(msg_id)).await
                    } else {
                        builder.await
                    }
                } else {
                    builder.await
                };

                match response {
                    Ok(response) => match response.models().await {
                        Ok(messages) => {
                            let history = messages
                                .into_iter()
                                .map(|m| river_adapter::HistoryMessage {
                                    message_id: m.id.to_string(),
                                    channel: m.channel_id.to_string(),
                                    author: Author {
                                        id: m.author.id.to_string(),
                                        name: m.author.name,
                                        bot: m.author.bot,
                                    },
                                    content: m.content,
                                    timestamp: format_timestamp(m.timestamp),
                                })
                                .collect();
                            OutboundResponse {
                                ok: true,
                                data: Some(ResponseData::History { messages: history }),
                                error: None,
                            }
                        }
                        Err(e) => error_response(ErrorCode::PlatformError, &e.to_string()),
                    },
                    Err(e) => {
                        if let Some(retry_after_ms) = check_rate_limit(&e) {
                            error_response_rate_limited(retry_after_ms)
                        } else {
                            error_response(ErrorCode::PlatformError, &e.to_string())
                        }
                    }
                }
            }
            _ => error_response(
                ErrorCode::UnsupportedFeature,
                &format!("{:?} not supported", request.feature_id()),
            ),
        }
    }

    /// Check health.
    pub async fn is_healthy(&self) -> bool {
        *self.connected.read().await
    }
}

#[async_trait]
impl Adapter for DiscordClient {
    fn adapter_type(&self) -> &str {
        &self.adapter_name
    }

    fn features(&self) -> Vec<FeatureId> {
        supported_features()
    }

    async fn start(&self, _worker_endpoint: String) -> Result<(), AdapterError> {
        // Event forwarding is started in new(), this is a no-op
        // The worker_endpoint is provided during registration
        Ok(())
    }

    async fn execute(&self, request: OutboundRequest) -> Result<OutboundResponse, AdapterError> {
        Ok(self.execute_impl(request).await)
    }

    async fn health(&self) -> Result<(), AdapterError> {
        if self.is_healthy().await {
            Ok(())
        } else {
            Err(AdapterError::Connection("websocket disconnected".into()))
        }
    }
}

/// Convert twilight event to InboundEvent.
fn convert_event(adapter_name: &str, event: Event) -> Option<InboundEvent> {
    match event {
        Event::MessageCreate(msg) => {
            // Skip bot messages to avoid feedback loops
            if msg.author.bot {
                return None;
            }

            // Only handle regular messages
            if msg.kind != MessageType::Regular && msg.kind != MessageType::Reply {
                return None;
            }

            let attachments = msg
                .attachments
                .iter()
                .map(|a| Attachment {
                    id: a.id.to_string(),
                    filename: a.filename.clone(),
                    url: a.url.clone(),
                    size: a.size,
                    content_type: a.content_type.clone(),
                })
                .collect();

            Some(InboundEvent {
                adapter: adapter_name.into(),
                metadata: EventMetadata::MessageCreate {
                    channel: msg.channel_id.to_string(),
                    author: Author {
                        id: msg.author.id.to_string(),
                        name: msg.author.name.clone(),
                        bot: msg.author.bot,
                    },
                    content: msg.content.clone(),
                    message_id: msg.id.to_string(),
                    timestamp: format_timestamp(msg.timestamp),
                    reply_to: msg.referenced_message.as_ref().map(|m| m.id.to_string()),
                    attachments,
                },
            })
        }
        Event::MessageUpdate(msg) => Some(InboundEvent {
            adapter: adapter_name.into(),
            metadata: EventMetadata::MessageUpdate {
                channel: msg.channel_id.to_string(),
                message_id: msg.id.to_string(),
                content: msg.content.clone(),
                timestamp: msg
                    .edited_timestamp
                    .map(format_timestamp)
                    .unwrap_or_default(),
            },
        }),
        Event::MessageDelete(msg) => Some(InboundEvent {
            adapter: adapter_name.into(),
            metadata: EventMetadata::MessageDelete {
                channel: msg.channel_id.to_string(),
                message_id: msg.id.to_string(),
            },
        }),
        Event::ReactionAdd(reaction) => Some(InboundEvent {
            adapter: adapter_name.into(),
            metadata: EventMetadata::ReactionAdd {
                channel: reaction.channel_id.to_string(),
                message_id: reaction.message_id.to_string(),
                user_id: reaction.user_id.to_string(),
                emoji: format_emoji(&reaction.emoji),
            },
        }),
        Event::ReactionRemove(reaction) => Some(InboundEvent {
            adapter: adapter_name.into(),
            metadata: EventMetadata::ReactionRemove {
                channel: reaction.channel_id.to_string(),
                message_id: reaction.message_id.to_string(),
                user_id: reaction.user_id.to_string(),
                emoji: format_emoji(&reaction.emoji),
            },
        }),
        Event::TypingStart(typing) => Some(InboundEvent {
            adapter: adapter_name.into(),
            metadata: EventMetadata::TypingStart {
                channel: typing.channel_id.to_string(),
                user_id: typing.user_id.to_string(),
            },
        }),
        Event::GatewayReconnect => {
            tracing::info!("Gateway reconnecting");
            None
        }
        Event::GatewayClose(close) => {
            let reason = close
                .map(|c| format!("{}: {}", c.code, c.reason))
                .unwrap_or_else(|| "unknown".into());
            Some(InboundEvent {
                adapter: adapter_name.into(),
                metadata: EventMetadata::ConnectionLost {
                    reason,
                    reconnecting: true,
                },
            })
        }
        _ => None, // Ignore other events
    }
}

/// Features supported by this adapter.
pub fn supported_features() -> Vec<FeatureId> {
    vec![
        FeatureId::SendMessage,
        FeatureId::ReceiveMessage,
        FeatureId::EditMessage,
        FeatureId::DeleteMessage,
        FeatureId::ReadHistory,
        FeatureId::AddReaction,
        FeatureId::RemoveReaction,
        FeatureId::TypingIndicator,
    ]
}

/// Format timestamp using chrono.
fn format_timestamp(ts: twilight_model::util::Timestamp) -> String {
    // Convert to chrono DateTime via microseconds since epoch
    let micros = ts.as_micros();
    let secs = micros / 1_000_000;
    let nsecs = ((micros % 1_000_000) * 1000) as u32;

    DateTime::<Utc>::from_timestamp(secs, nsecs)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

/// Format emoji for InboundEvent.
fn format_emoji(emoji: &EmojiReactionType) -> String {
    match emoji {
        EmojiReactionType::Custom { id, name, .. } => {
            format!("<:{}:{}>", name.as_deref().unwrap_or("emoji"), id)
        }
        EmojiReactionType::Unicode { name } => name.clone(),
    }
}

/// Parse emoji string for API request.
fn parse_emoji(emoji: &str) -> twilight_http::request::channel::reaction::RequestReactionType<'_> {
    // Check if it's a custom emoji format: <:name:id> or <a:name:id>
    if emoji.starts_with('<') && emoji.ends_with('>') {
        let inner = &emoji[1..emoji.len() - 1];
        let parts: Vec<&str> = inner.split(':').collect();
        if parts.len() >= 3 {
            if let Ok(id) = parts[2].parse::<u64>() {
                return twilight_http::request::channel::reaction::RequestReactionType::Custom {
                    id: Id::new(id),
                    name: Some(parts[1]),
                };
            }
        }
    }

    // Default to unicode emoji
    twilight_http::request::channel::reaction::RequestReactionType::Unicode { name: emoji }
}

/// Create an error response.
fn error_response(code: ErrorCode, message: &str) -> OutboundResponse {
    OutboundResponse {
        ok: false,
        data: None,
        error: Some(ResponseError {
            code,
            message: message.into(),
        }),
    }
}

/// Create a rate-limited error response.
fn error_response_rate_limited(retry_after_ms: u64) -> OutboundResponse {
    OutboundResponse {
        ok: false,
        data: None,
        error: Some(ResponseError {
            code: ErrorCode::RateLimited,
            message: format!("rate limited, retry after {}ms", retry_after_ms),
        }),
    }
}

/// Check if error is a rate limit and extract retry_after if so.
fn check_rate_limit(err: &twilight_http::Error) -> Option<u64> {
    if let twilight_http::error::ErrorType::Response { status, .. } = err.kind() {
        if status.get() == 429 {
            // Default to 1 second if we can't parse retry_after
            // In practice, twilight handles rate limits internally,
            // but we expose this for the adapter protocol
            return Some(1000);
        }
    }
    None
}

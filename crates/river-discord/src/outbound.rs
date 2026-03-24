//! HTTP server for outbound messages and admin API

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use river_adapter::{SendOptions, SendRequest as AdapterSendRequest, SendResponse};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::channels::ChannelState;
use crate::client::DiscordSender;

/// Discord-specific send request (extends adapter SendRequest)
#[derive(Debug, Deserialize)]
pub struct DiscordSendRequest {
    pub channel: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub create_thread: Option<String>,
    #[serde(default)]
    pub reaction: Option<String>,
}

impl DiscordSendRequest {
    /// Validate the request
    pub fn validate(&self) -> Result<(), &'static str> {
        // Must have content or reaction
        if self.content.is_none() && self.reaction.is_none() {
            return Err("must provide content or reaction");
        }

        // content and reaction are mutually exclusive
        if self.content.is_some() && self.reaction.is_some() {
            return Err("content and reaction are mutually exclusive");
        }

        // reply_to and create_thread are mutually exclusive
        if self.reply_to.is_some() && self.create_thread.is_some() {
            return Err("reply_to and create_thread are mutually exclusive");
        }

        Ok(())
    }

    /// Convert to adapter SendRequest (for logging/forwarding)
    pub fn to_adapter_request(&self) -> Option<AdapterSendRequest> {
        self.content.as_ref().map(|content| AdapterSendRequest {
            channel: self.channel.clone(),
            content: content.clone(),
            options: SendOptions {
                reply_to: self.reply_to.clone(),
                thread_id: self.thread_id.clone(),
                metadata: serde_json::json!({
                    "create_thread": self.create_thread,
                }),
            },
        })
    }
}

/// Add channel request
#[derive(Debug, Deserialize)]
pub struct AddChannelRequest {
    pub channel_id: String,
}

/// Channel operation response
#[derive(Debug, Serialize)]
pub struct ChannelResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// List channels response
#[derive(Debug, Serialize)]
pub struct ListChannelsResponse {
    pub channels: Vec<String>,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub healthy: bool,
    pub status: &'static str,
    pub discord: &'static str,
    pub gateway: &'static str,
    pub channel_count: usize,
}

/// Read message query parameters
#[derive(Debug, Deserialize)]
pub struct ReadQuery {
    pub channel: String,
    #[serde(default = "default_limit")]
    pub limit: u64,
    pub before: Option<String>,
}

fn default_limit() -> u64 {
    50
}

/// Read message response
#[derive(Debug, Serialize)]
pub struct ReadMessage {
    pub id: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: i64,
    pub is_bot: bool,
    pub reactions: Vec<ReadReaction>,
}

/// Message reaction
#[derive(Debug, Serialize)]
pub struct ReadReaction {
    pub emoji: String,
    pub count: usize,
}

/// Shared application state for HTTP server
pub struct AppState {
    pub channels: Arc<ChannelState>,
    pub discord: Arc<RwLock<Option<DiscordSender>>>,
    pub discord_connected: std::sync::atomic::AtomicBool,
    pub gateway_reachable: std::sync::atomic::AtomicBool,
}

impl AppState {
    pub fn new(channels: Arc<ChannelState>) -> Arc<Self> {
        Arc::new(Self {
            channels,
            discord: Arc::new(RwLock::new(None)),
            discord_connected: std::sync::atomic::AtomicBool::new(false),
            gateway_reachable: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub async fn set_discord(&self, sender: DiscordSender) {
        *self.discord.write().await = Some(sender);
    }

    pub fn set_discord_connected(&self, connected: bool) {
        self.discord_connected
            .store(connected, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_gateway_reachable(&self, reachable: bool) {
        self.gateway_reachable
            .store(reachable, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Create the HTTP router
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/send", post(handle_send))
        .route("/read", get(handle_read))
        .route("/channels", get(list_channels))
        .route("/channels", post(add_channel))
        .route("/channels/{id}", delete(remove_channel))
        .with_state(state)
}

async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let discord_connected = state
        .discord_connected
        .load(std::sync::atomic::Ordering::Relaxed);
    let gateway_reachable = state
        .gateway_reachable
        .load(std::sync::atomic::Ordering::Relaxed);

    let discord = if discord_connected {
        "connected"
    } else {
        "disconnected"
    };
    let gateway = if gateway_reachable {
        "reachable"
    } else {
        "unreachable"
    };

    Json(HealthResponse {
        healthy: discord_connected,
        status: "ok",
        discord,
        gateway,
        channel_count: state.channels.count().await,
    })
}

async fn handle_send(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DiscordSendRequest>,
) -> Result<Json<SendResponse>, (StatusCode, Json<SendResponse>)> {
    // Validate request
    if let Err(e) = req.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some(format!("validation error: {}", e)),
            }),
        ));
    }

    // Get Discord sender
    let discord_guard = state.discord.read().await;
    let Some(ref discord) = *discord_guard else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some("discord client not initialized".to_string()),
            }),
        ));
    };

    // Parse channel ID
    let channel_id: u64 = req.channel.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some("invalid channel id".to_string()),
            }),
        )
    })?;

    // Handle reaction
    if let Some(emoji) = &req.reaction {
        let message_id: u64 = req
            .reply_to
            .as_ref()
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(SendResponse {
                        success: false,
                        message_id: None,
                        error: Some("reply_to required for reactions".to_string()),
                    }),
                )
            })?
            .parse()
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(SendResponse {
                        success: false,
                        message_id: None,
                        error: Some("invalid message id".to_string()),
                    }),
                )
            })?;

        discord
            .add_reaction(channel_id, message_id, emoji)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(SendResponse {
                        success: false,
                        message_id: None,
                        error: Some(format!("discord api error: {}", e)),
                    }),
                )
            })?;

        return Ok(Json(SendResponse {
            success: true,
            message_id: None,
            error: None,
        }));
    }

    // Handle message
    let content = req.content.as_ref().unwrap();
    let reply_to = req.reply_to.as_ref().and_then(|s| s.parse().ok());

    // If thread_id is provided, use it as the target channel (threads are channels in Discord)
    let target_channel_id = if let Some(ref thread_id_str) = req.thread_id {
        thread_id_str.parse().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(SendResponse {
                    success: false,
                    message_id: None,
                    error: Some("invalid thread_id".to_string()),
                }),
            )
        })?
    } else {
        channel_id
    };

    let message_id = discord
        .send_message(target_channel_id, content, reply_to)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(SendResponse {
                    success: false,
                    message_id: None,
                    error: Some(format!("discord api error: {}", e)),
                }),
            )
        })?;

    // If create_thread is provided, create a thread from the message we just sent
    if let Some(ref thread_name) = req.create_thread {
        let thread_id = discord
            .create_thread(target_channel_id, message_id, thread_name)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(SendResponse {
                        success: false,
                        message_id: Some(message_id.to_string()),
                        error: Some(format!("failed to create thread: {}", e)),
                    }),
                )
            })?;

        // Add the thread to channel state so we listen to it
        state.channels.add(thread_id).await;
        tracing::info!("Created thread and added to listen set");
    }

    tracing::info!("Sent message to Discord");

    Ok(Json(SendResponse {
        success: true,
        message_id: Some(message_id.to_string()),
        error: None,
    }))
}

async fn list_channels(State(state): State<Arc<AppState>>) -> Json<ListChannelsResponse> {
    let channels = state.channels.list().await;
    Json(ListChannelsResponse {
        channels: channels.into_iter().map(|c| c.to_string()).collect(),
    })
}

async fn add_channel(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddChannelRequest>,
) -> Result<Json<ChannelResponse>, (StatusCode, Json<ChannelResponse>)> {
    let channel_id: u64 = req.channel_id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ChannelResponse {
                success: false,
                error: Some("invalid channel id".to_string()),
            }),
        )
    })?;

    state.channels.add(channel_id).await;
    tracing::info!("Channel added");

    Ok(Json(ChannelResponse {
        success: true,
        error: None,
    }))
}

async fn remove_channel(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ChannelResponse>, (StatusCode, Json<ChannelResponse>)> {
    let channel_id: u64 = id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ChannelResponse {
                success: false,
                error: Some("invalid channel id".to_string()),
            }),
        )
    })?;

    let removed = state.channels.remove(channel_id).await;
    if removed {
        tracing::info!("Channel removed");
        Ok(Json(ChannelResponse {
            success: true,
            error: None,
        }))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ChannelResponse {
                success: false,
                error: Some("channel not in listen set".to_string()),
            }),
        ))
    }
}

async fn handle_read(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Vec<ReadMessage>>, (StatusCode, Json<SendResponse>)> {
    let discord_guard = state.discord.read().await;
    let Some(ref discord) = *discord_guard else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some("discord client not initialized".to_string()),
            }),
        ));
    };

    let channel_id: u64 = query.channel.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(SendResponse {
                success: false,
                message_id: None,
                error: Some("invalid channel id".to_string()),
            }),
        )
    })?;

    let before_id: Option<u64> = query
        .before
        .as_ref()
        .map(|s| s.parse())
        .transpose()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(SendResponse {
                    success: false,
                    message_id: None,
                    error: Some("invalid before message id".to_string()),
                }),
            )
        })?;

    let limit = query.limit.min(100) as u16;

    let messages = discord
        .read_messages(channel_id, limit, before_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(SendResponse {
                    success: false,
                    message_id: None,
                    error: Some(format!("discord api error: {}", e)),
                }),
            )
        })?;

    Ok(Json(messages))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_request_validation_valid_content() {
        let req = DiscordSendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_send_request_validation_valid_reaction() {
        let req = DiscordSendRequest {
            channel: "123".to_string(),
            content: None,
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: Some("\u{1F44D}".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_send_request_validation_no_content_or_reaction() {
        let req = DiscordSendRequest {
            channel: "123".to_string(),
            content: None,
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: None,
        };
        assert_eq!(
            req.validate().unwrap_err(),
            "must provide content or reaction"
        );
    }

    #[test]
    fn test_send_request_validation_both_content_and_reaction() {
        let req = DiscordSendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: Some("\u{1F44D}".to_string()),
        };
        assert_eq!(
            req.validate().unwrap_err(),
            "content and reaction are mutually exclusive"
        );
    }

    #[test]
    fn test_send_request_validation_reply_and_thread() {
        let req = DiscordSendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: Some("msg1".to_string()),
            thread_id: None,
            create_thread: Some("New Thread".to_string()),
            reaction: None,
        };
        assert_eq!(
            req.validate().unwrap_err(),
            "reply_to and create_thread are mutually exclusive"
        );
    }

    #[tokio::test]
    async fn test_health_check() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![1, 2, 3], None);
        let state = AppState::new(channels);
        state.set_discord_connected(true);
        state.set_gateway_reachable(true);

        let app = create_router(state);
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_channels() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![111, 222], None);
        let state = AppState::new(channels);

        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/channels")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_add_channel() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![], None);
        let state = AppState::new(channels.clone());

        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"channel_id": "999"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(channels.contains(999).await);
    }

    #[test]
    fn test_read_query_default_limit() {
        let json = r#"{"channel": "123"}"#;
        let query: ReadQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.channel, "123");
        assert_eq!(query.limit, 50);
        assert_eq!(query.before, None);
    }

    #[test]
    fn test_read_query_with_limit_and_before() {
        let json = r#"{"channel": "123", "limit": 25, "before": "999"}"#;
        let query: ReadQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.channel, "123");
        assert_eq!(query.limit, 25);
        assert_eq!(query.before, Some("999".to_string()));
    }

    #[test]
    fn test_read_message_serialization() {
        let msg = ReadMessage {
            id: "123".to_string(),
            author_id: "456".to_string(),
            author_name: "TestUser".to_string(),
            content: "Hello".to_string(),
            timestamp: 1234567890,
            is_bot: false,
            reactions: vec![ReadReaction {
                emoji: "👍".to_string(),
                count: 3,
            }],
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"id\":\"123\""));
        assert!(json.contains("\"author_name\":\"TestUser\""));
        assert!(json.contains("\"reactions\":["));
    }

    #[tokio::test]
    async fn test_read_endpoint_no_discord_client() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![111], None);
        let state = AppState::new(channels);

        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/read?channel=111")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_read_endpoint_invalid_channel() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![], None);
        let state = AppState::new(channels);

        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/read?channel=invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Without Discord client, service is unavailable before validation
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_read_endpoint_invalid_before() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let channels = ChannelState::new(vec![], None);
        let state = AppState::new(channels);

        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/read?channel=111&before=invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Without Discord client, service is unavailable before validation
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}

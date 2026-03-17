//! HTTP server for outbound messages and admin API

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::channels::ChannelState;

/// Send message request from gateway
#[derive(Debug, Deserialize)]
pub struct SendRequest {
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

impl SendRequest {
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
}

/// Send message response
#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    pub status: &'static str,
    pub discord: &'static str,
    pub gateway: &'static str,
    pub channel_count: usize,
}

/// Shared application state for HTTP server
pub struct AppState {
    pub channels: Arc<ChannelState>,
    pub discord_connected: std::sync::atomic::AtomicBool,
    pub gateway_reachable: std::sync::atomic::AtomicBool,
}

impl AppState {
    pub fn new(channels: Arc<ChannelState>) -> Arc<Self> {
        Arc::new(Self {
            channels,
            discord_connected: std::sync::atomic::AtomicBool::new(false),
            gateway_reachable: std::sync::atomic::AtomicBool::new(false),
        })
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
        .route("/channels", get(list_channels))
        .route("/channels", post(add_channel))
        .route("/channels/{id}", delete(remove_channel))
        .with_state(state)
}

async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let discord = if state
        .discord_connected
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        "connected"
    } else {
        "disconnected"
    };
    let gateway = if state
        .gateway_reachable
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        "reachable"
    } else {
        "unreachable"
    };

    Json(HealthResponse {
        status: "ok",
        discord,
        gateway,
        channel_count: state.channels.count().await,
    })
}

async fn handle_send(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<SendRequest>,
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

    // TODO: Actually send to Discord via Twilight HTTP client
    // For now, return a placeholder response
    Ok(Json(SendResponse {
        success: true,
        message_id: Some("placeholder".to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_request_validation_valid_content() {
        let req = SendRequest {
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
        let req = SendRequest {
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
        let req = SendRequest {
            channel: "123".to_string(),
            content: None,
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: None,
        };
        assert_eq!(req.validate().unwrap_err(), "must provide content or reaction");
    }

    #[test]
    fn test_send_request_validation_both_content_and_reaction() {
        let req = SendRequest {
            channel: "123".to_string(),
            content: Some("hello".to_string()),
            reply_to: None,
            thread_id: None,
            create_thread: None,
            reaction: Some("\u{1F44D}".to_string()),
        };
        assert_eq!(req.validate().unwrap_err(), "content and reaction are mutually exclusive");
    }

    #[test]
    fn test_send_request_validation_reply_and_thread() {
        let req = SendRequest {
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
}

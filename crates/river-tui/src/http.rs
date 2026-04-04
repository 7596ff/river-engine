//! HTTP server for adapter API.

use crate::tui::UiEvent;
use crate::SharedState;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use river_adapter::{ErrorCode, OutboundRequest, OutboundResponse, ResponseData, ResponseError};
use serde::Serialize;
use tokio::sync::mpsc;

/// Combined state for handlers.
#[derive(Clone)]
pub struct HttpState {
    pub state: SharedState,
    pub ui_tx: mpsc::Sender<UiEvent>,
}

/// Create the HTTP router.
pub fn router(state: SharedState, ui_tx: mpsc::Sender<UiEvent>) -> Router {
    let http_state = HttpState { state, ui_tx };
    Router::new()
        .route("/execute", post(execute))
        .route("/health", get(health))
        .with_state(http_state)
}

/// POST /execute - Execute an outbound request.
async fn execute(
    State(state): State<HttpState>,
    Json(request): Json<OutboundRequest>,
) -> impl IntoResponse {
    let mut s = state.state.write().await;

    let response = match request {
        OutboundRequest::SendMessage {
            channel: _,
            content: _,
            reply_to: _,
        } => {
            // Generate ID for the message (actual content shows via context tail)
            let id = s.generate_message_id();
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::MessageSent { message_id: id }),
                error: None,
            }
        }
        OutboundRequest::EditMessage {
            channel: _,
            message_id,
            ..
        } => {
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::MessageEdited { message_id }),
                error: None,
            }
        }
        OutboundRequest::DeleteMessage {
            channel: _,
            message_id: _,
        } => {
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::MessageDeleted),
                error: None,
            }
        }
        OutboundRequest::TypingIndicator { channel: _ } => {
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::TypingStarted),
                error: None,
            }
        }
        OutboundRequest::AddReaction {
            channel: _,
            message_id: _,
            emoji: _,
        } => {
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::ReactionAdded),
                error: None,
            }
        }
        OutboundRequest::ReadHistory {
            channel: _,
            limit,
            before: _,
            after: _,
        } => {
            // Return user messages from our history
            let limit = limit.unwrap_or(50) as usize;
            let messages: Vec<river_adapter::HistoryMessage> = s
                .messages
                .iter()
                .rev()
                .take(limit)
                .filter_map(|m| match m {
                    crate::adapter::DisplayMessage::User {
                        id,
                        content,
                        timestamp,
                    } => Some(river_adapter::HistoryMessage {
                        message_id: id.clone(),
                        channel: s.channel.clone(),
                        author: river_adapter::Author {
                            id: "user-1".into(),
                            name: "Human".into(),
                            bot: false,
                        },
                        content: content.clone(),
                        timestamp: timestamp.to_rfc3339(),
                        reply_to: None,
                    }),
                    _ => None,
                })
                .collect();

            OutboundResponse {
                ok: true,
                data: Some(ResponseData::History { messages }),
                error: None,
            }
        }
        _ => OutboundResponse {
            ok: false,
            data: None,
            error: Some(ResponseError {
                code: ErrorCode::UnsupportedFeature,
                message: "Not supported by mock adapter".into(),
                retry_after_ms: None,
            }),
        },
    };

    // Notify UI to refresh
    let _ = state.ui_tx.send(UiEvent::Refresh).await;

    (StatusCode::OK, Json(response))
}

/// Health response.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub struct HealthResponse {
    pub status: String,
}

/// GET /health - Health check.
async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    use crate::adapter::AdapterState;

    /// Helper to create test state and router.
    fn create_test_app() -> (axum::Router, mpsc::Receiver<UiEvent>) {
        let state: SharedState = Arc::new(RwLock::new(AdapterState::new(
            "test-dyad".to_string(),
            "mock".to_string(),
            "test-channel".to_string(),
        )));
        let (ui_tx, ui_rx) = mpsc::channel::<UiEvent>(16);
        let app = router(state, ui_tx);
        (app, ui_rx)
    }

    #[tokio::test]
    async fn test_health_endpoint_returns_ok() {
        let (app, _ui_rx) = create_test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let health: HealthResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(health.status, "ok");
    }

    #[tokio::test]
    async fn test_execute_send_message() {
        let (app, mut ui_rx) = create_test_app();

        let request = OutboundRequest::SendMessage {
            channel: "test-channel".into(),
            content: "Hello, world!".into(),
            reply_to: None,
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);
        assert!(resp.error.is_none());

        // Should have MessageSent response with a message_id
        match resp.data {
            Some(ResponseData::MessageSent { message_id }) => {
                assert!(!message_id.is_empty());
            }
            _ => panic!("Expected MessageSent response"),
        }

        // Should have triggered UI refresh
        let event = ui_rx.try_recv();
        assert!(event.is_ok());
    }

    #[tokio::test]
    async fn test_execute_edit_message() {
        let (app, mut ui_rx) = create_test_app();

        let request = OutboundRequest::EditMessage {
            channel: "test-channel".into(),
            message_id: "msg-123".into(),
            content: "Edited content".into(),
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);

        match resp.data {
            Some(ResponseData::MessageEdited { message_id }) => {
                assert_eq!(message_id, "msg-123");
            }
            _ => panic!("Expected MessageEdited response"),
        }

        // Should have triggered UI refresh
        let event = ui_rx.try_recv();
        assert!(event.is_ok());
    }

    #[tokio::test]
    async fn test_execute_typing_indicator() {
        let (app, mut ui_rx) = create_test_app();

        let request = OutboundRequest::TypingIndicator {
            channel: "test-channel".into(),
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.data, Some(ResponseData::TypingStarted));

        // Should have triggered UI refresh
        let event = ui_rx.try_recv();
        assert!(event.is_ok());
    }

    #[tokio::test]
    async fn test_execute_read_history_empty() {
        let (app, _ui_rx) = create_test_app();

        let request = OutboundRequest::ReadHistory {
            channel: "test-channel".into(),
            limit: Some(10),
            before: None,
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);

        match resp.data {
            Some(ResponseData::History { messages }) => {
                assert!(messages.is_empty());
            }
            _ => panic!("Expected History response"),
        }
    }

    #[tokio::test]
    async fn test_execute_read_history_with_messages() {
        // Create state with some user messages
        let state: SharedState = Arc::new(RwLock::new(AdapterState::new(
            "test-dyad".to_string(),
            "mock".to_string(),
            "test-channel".to_string(),
        )));

        // Add user messages
        {
            let mut s = state.write().await;
            s.add_user_message("First message");
            s.add_user_message("Second message");
        }

        let (ui_tx, _ui_rx) = mpsc::channel::<UiEvent>(16);
        let app = router(state, ui_tx);

        let request = OutboundRequest::ReadHistory {
            channel: "test-channel".into(),
            limit: Some(10),
            before: None,
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);

        match resp.data {
            Some(ResponseData::History { messages }) => {
                assert_eq!(messages.len(), 2);
                // Messages are returned in reverse order (most recent first), then collected
                // The implementation takes reverse, so the order should reflect that
                assert!(messages.iter().any(|m| m.content == "First message"));
                assert!(messages.iter().any(|m| m.content == "Second message"));
                // Verify author info
                for msg in &messages {
                    assert_eq!(msg.author.name, "Human");
                    assert!(!msg.author.bot);
                    assert_eq!(msg.channel, "test-channel");
                }
            }
            _ => panic!("Expected History response"),
        }
    }

    #[tokio::test]
    async fn test_execute_delete_message() {
        let (app, _ui_rx) = create_test_app();

        let request = OutboundRequest::DeleteMessage {
            channel: "test-channel".into(),
            message_id: "msg-456".into(),
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.data, Some(ResponseData::MessageDeleted));
    }

    #[tokio::test]
    async fn test_execute_add_reaction() {
        let (app, _ui_rx) = create_test_app();

        let request = OutboundRequest::AddReaction {
            channel: "test-channel".into(),
            message_id: "msg-789".into(),
            emoji: "thumbsup".into(),
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.data, Some(ResponseData::ReactionAdded));
    }

    #[tokio::test]
    async fn test_execute_unsupported_feature() {
        let (app, _ui_rx) = create_test_app();

        // PinMessage is not fully implemented (falls through to the catch-all)
        let request = OutboundRequest::PinMessage {
            channel: "test-channel".into(),
            message_id: "msg-123".into(),
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(!resp.ok);
        assert!(resp.data.is_none());
        assert!(resp.error.is_some());

        let error = resp.error.unwrap();
        assert_eq!(error.code, ErrorCode::UnsupportedFeature);
    }
}

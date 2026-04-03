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
use serde::{Deserialize, Serialize};
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
        .route("/start", post(start))
        .route("/execute", post(execute))
        .route("/health", get(health))
        .with_state(http_state)
}

/// Start request body.
#[derive(Debug, Deserialize, Serialize)]
pub struct StartRequest {
    pub worker_endpoint: String,
}

/// Start response.
#[derive(Debug, Serialize)]
pub struct StartResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// POST /start - Bind adapter to worker.
async fn start(
    State(state): State<HttpState>,
    Json(request): Json<StartRequest>,
) -> impl IntoResponse {
    let mut s = state.state.write().await;

    if s.worker_endpoint.is_some() {
        return Json(StartResponse {
            ok: false,
            error: Some("already bound to worker".into()),
        });
    }

    s.worker_endpoint = Some(request.worker_endpoint.clone());
    s.add_system_message(&format!("Worker bound: {}", request.worker_endpoint));

    // Notify UI
    let _ = state.ui_tx.send(UiEvent::Refresh).await;

    Json(StartResponse {
        ok: true,
        error: None,
    })
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
            }),
        },
    };

    // Notify UI to refresh
    let _ = state.ui_tx.send(UiEvent::Refresh).await;

    (StatusCode::OK, Json(response))
}

/// Health response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
}

/// GET /health - Health check.
async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".into(),
    })
}

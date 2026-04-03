//! HTTP server for adapter API.

use crate::adapter::{DebugEvent, FlashMessage};
use crate::log::TrafficLog;
use crate::tui::UiEvent;
use crate::SharedState;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use river_adapter::{ErrorCode, OutboundRequest, OutboundResponse, ResponseData, ResponseError};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Combined state for handlers.
#[derive(Clone)]
pub struct HttpState {
    pub state: SharedState,
    pub ui_tx: mpsc::Sender<UiEvent>,
    pub log: Arc<Mutex<Option<TrafficLog>>>,
}

/// Create the HTTP router.
pub fn router(
    state: SharedState,
    ui_tx: mpsc::Sender<UiEvent>,
    log: Arc<Mutex<Option<TrafficLog>>>,
) -> Router {
    let http_state = HttpState { state, ui_tx, log };
    Router::new()
        .route("/start", post(start))
        .route("/execute", post(execute))
        .route("/debug", post(debug))
        .route("/flash", post(flash))
        .route("/health", get(health))
        .with_state(http_state)
}

/// Start request body.
#[derive(Debug, Deserialize)]
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

    // Log
    if let Ok(mut log) = state.log.try_lock() {
        if let Some(ref mut l) = *log {
            l.log("start", &request);
        }
    }

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
    // Log request
    if let Ok(mut log) = state.log.try_lock() {
        if let Some(ref mut l) = *log {
            l.log("execute_request", &request);
        }
    }

    let mut s = state.state.write().await;

    let response = match request {
        OutboundRequest::SendMessage {
            channel: _,
            content,
            reply_to: _,
        } => {
            // Worker is sending a message - display it
            let id = s.add_worker_message(&content);
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::MessageSent { message_id: id }),
                error: None,
            }
        }
        OutboundRequest::EditMessage {
            channel: _,
            message_id,
            content,
        } => {
            s.add_system_message(&format!("Edit {}: {}", &message_id[..8], content));
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::MessageEdited { message_id }),
                error: None,
            }
        }
        OutboundRequest::DeleteMessage {
            channel: _,
            message_id,
        } => {
            s.add_system_message(&format!("Deleted message {}", &message_id[..8]));
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::MessageDeleted),
                error: None,
            }
        }
        OutboundRequest::TypingIndicator { channel: _ } => {
            // Could show typing indicator in UI
            OutboundResponse {
                ok: true,
                data: Some(ResponseData::TypingStarted),
                error: None,
            }
        }
        OutboundRequest::AddReaction {
            channel: _,
            message_id,
            emoji,
        } => {
            s.add_system_message(&format!("Reaction {} on {}", emoji, &message_id[..8]));
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
            // Return recent messages from our history
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
                    crate::adapter::DisplayMessage::Worker {
                        id,
                        content,
                        timestamp,
                    } => Some(river_adapter::HistoryMessage {
                        message_id: id.clone(),
                        channel: s.channel.clone(),
                        author: river_adapter::Author {
                            id: "worker-1".into(),
                            name: "Worker".into(),
                            bot: true,
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

    // Log response
    if let Ok(mut log) = state.log.try_lock() {
        if let Some(ref mut l) = *log {
            l.log("execute_response", &response);
        }
    }

    // Notify UI to refresh
    let _ = state.ui_tx.send(UiEvent::Refresh).await;

    (StatusCode::OK, Json(response))
}

/// POST /debug - Receive debug events from worker.
async fn debug(
    State(state): State<HttpState>,
    Json(event): Json<DebugEvent>,
) -> impl IntoResponse {
    // Log
    if let Ok(mut log) = state.log.try_lock() {
        if let Some(ref mut l) = *log {
            l.log("debug", &event);
        }
    }

    let mut s = state.state.write().await;

    match &event {
        DebugEvent::ToolCall {
            tool,
            args,
            result,
            error,
            ..
        } => {
            let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
            let result_str = result
                .as_ref()
                .map(|r| serde_json::to_string_pretty(r).unwrap_or_default());
            let error_str = error.as_deref();
            s.add_tool_trace(tool, &args_str, result_str.as_deref(), error_str);
        }
        DebugEvent::Thinking { started, .. } => {
            s.thinking = *started;
        }
        DebugEvent::LlmRequest { .. } => {
            s.thinking = true;
        }
        DebugEvent::LlmResponse { .. } => {
            // Keep thinking until we get a message
        }
    }

    // Notify UI
    let _ = state.ui_tx.send(UiEvent::Refresh).await;

    Json(serde_json::json!({ "ok": true }))
}

/// POST /flash - Receive flash messages.
async fn flash(
    State(state): State<HttpState>,
    Json(flash): Json<FlashMessage>,
) -> impl IntoResponse {
    // Log
    if let Ok(mut log) = state.log.try_lock() {
        if let Some(ref mut l) = *log {
            l.log("flash", &flash);
        }
    }

    let mut s = state.state.write().await;

    // Parse expires_at
    if let Ok(expires_at) = DateTime::parse_from_rfc3339(&flash.expires_at) {
        s.add_flash(&flash.from, &flash.content, expires_at.with_timezone(&Utc));
    }

    // Notify UI
    let _ = state.ui_tx.send(UiEvent::Refresh).await;

    Json(serde_json::json!({ "ok": true }))
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

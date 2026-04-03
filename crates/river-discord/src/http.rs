//! HTTP server for adapter API.

use crate::HttpState;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use river_adapter::{Adapter, ErrorCode, OutboundRequest, OutboundResponse, ResponseError};
use serde::Serialize;
use std::sync::Arc;

/// Create the HTTP router.
pub fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/execute", post(execute))
        .route("/health", get(health))
        .with_state(state)
}

/// POST /execute - Execute an outbound request.
async fn execute(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<OutboundRequest>,
) -> impl IntoResponse {
    let response = match state.discord.execute(request).await {
        Ok(r) => r,
        Err(e) => OutboundResponse {
            ok: false,
            data: None,
            error: Some(ResponseError {
                code: ErrorCode::PlatformError,
                message: e.to_string(),
            }),
        },
    };
    (StatusCode::OK, Json(response))
}

/// Health response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// GET /health - Health check.
async fn health(State(state): State<Arc<HttpState>>) -> impl IntoResponse {
    if state.discord.is_healthy().await {
        (
            StatusCode::OK,
            Json(HealthResponse {
                status: "ok".into(),
                message: None,
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "error".into(),
                message: Some("websocket disconnected".into()),
            }),
        )
    }
}

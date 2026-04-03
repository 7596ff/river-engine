//! HTTP server for adapter API.

use crate::HttpState;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use river_adapter::OutboundRequest;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Create the HTTP router.
pub fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/start", post(start))
        .route("/execute", post(execute))
        .route("/health", get(health))
        .with_state(state)
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
    State(state): State<Arc<HttpState>>,
    Json(request): Json<StartRequest>,
) -> impl IntoResponse {
    let mut s = state.state.write().await;

    // Check if already bound
    if s.worker_endpoint.is_some() {
        return Json(StartResponse {
            ok: false,
            error: Some("already bound to worker".into()),
        });
    }

    s.worker_endpoint = Some(request.worker_endpoint.clone());
    tracing::info!("Bound to worker at {}", request.worker_endpoint);

    Json(StartResponse {
        ok: true,
        error: None,
    })
}

/// POST /execute - Execute an outbound request.
async fn execute(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<OutboundRequest>,
) -> impl IntoResponse {
    let response = state.discord.execute(request).await;
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

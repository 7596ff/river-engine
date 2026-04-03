//! HTTP server for snowflake generation.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{AgentBirth, GeneratorCache, SnowflakeType};

/// Shared state for the server.
pub struct AppState {
    pub cache: GeneratorCache,
}

/// Query parameters for single ID generation.
#[derive(Deserialize)]
pub struct IdQuery {
    birth: u64,
}

/// Response for single ID generation.
#[derive(Serialize)]
pub struct IdResponse {
    id: String,
}

/// Request body for batch ID generation.
#[derive(Deserialize)]
pub struct BatchRequest {
    birth: u64,
    #[serde(rename = "type")]
    snowflake_type: String,
    count: usize,
}

/// Response for batch ID generation.
#[derive(Serialize)]
pub struct BatchResponse {
    ids: Vec<String>,
}

/// Error response.
#[derive(Serialize)]
pub struct ErrorResponse {
    error: String,
}

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
    generators: usize,
}

/// Build the router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/id/{type}", get(get_id))
        .route("/ids", post(post_ids))
        .route("/health", get(health))
        .with_state(state)
}

/// GET /id/{type}?birth={birth}
async fn get_id(
    State(state): State<Arc<AppState>>,
    Path(type_str): Path<String>,
    Query(query): Query<IdQuery>,
) -> impl IntoResponse {
    let Some(snowflake_type) = SnowflakeType::from_str(&type_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid type: {}", type_str),
            }),
        )
            .into_response();
    };

    let birth = AgentBirth::from_u64(query.birth);
    match state.cache.next_id(birth, snowflake_type) {
        Ok(id) => (StatusCode::OK, Json(IdResponse { id: id.to_string() })).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// POST /ids
async fn post_ids(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchRequest>,
) -> impl IntoResponse {
    let Some(snowflake_type) = SnowflakeType::from_str(&req.snowflake_type) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid type: {}", req.snowflake_type),
            }),
        )
            .into_response();
    };

    if req.count > 10000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "count must be <= 10000".into(),
            }),
        )
            .into_response();
    }

    let birth = AgentBirth::from_u64(req.birth);
    match state.cache.next_ids(birth, snowflake_type, req.count) {
        Ok(ids) => {
            let ids = ids.into_iter().map(|id| id.to_string()).collect();
            (StatusCode::OK, Json(BatchResponse { ids })).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// GET /health
async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".into(),
        generators: state.cache.len(),
    })
}

//! HTTP server and endpoints.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use river_snowflake::{AgentBirth, GeneratorCache};

use crate::embed::EmbedClient;
use crate::index::IndexError;
use crate::search::{hit_to_result, CursorManager, SearchResponse};
use crate::store::Store;

/// Maximum number of results to fetch per search.
const MAX_SEARCH_RESULTS: usize = 100;

/// Shared application state.
pub struct AppState {
    pub store: Mutex<Store>,
    pub embed_client: EmbedClient,
    pub cursor_manager: CursorManager,
    pub id_cache: GeneratorCache,
    pub birth: AgentBirth,
}

// Request/response types

#[derive(Deserialize)]
pub struct IndexRequest {
    pub source: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct IndexResponse {
    pub indexed: bool,
    pub chunks: usize,
}

#[derive(Serialize)]
pub struct DeleteResponse {
    pub deleted: bool,
    pub chunks: usize,
}

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
}

#[derive(Deserialize)]
pub struct NextRequest {
    pub cursor: String,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub sources: usize,
    pub chunks: usize,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Build the router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/index", post(handle_index))
        .route("/source/{path:.*}", delete(handle_delete))
        .route("/search", post(handle_search))
        .route("/next", post(handle_next))
        .route("/health", get(handle_health))
        .with_state(state)
}

async fn handle_index(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IndexRequest>,
) -> impl IntoResponse {
    let result = crate::index::index_content(&state, &req.source, &req.content, state.birth).await;

    match result {
        Ok((indexed, chunks)) => (
            StatusCode::OK,
            Json(IndexResponse { indexed, chunks }),
        )
            .into_response(),
        Err(IndexError::EmptyContent) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "empty content".into(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn handle_delete(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let store = state.store.lock().await;

    match store.delete_source(&path) {
        Ok(count) => (
            StatusCode::OK,
            Json(DeleteResponse {
                deleted: true,
                chunks: count,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn handle_search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    // Generate query embedding
    let query_embedding = match state.embed_client.embed_one(&req.query).await {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    };

    // Search store
    let hits = {
        let store = state.store.lock().await;
        match store.search(&query_embedding, MAX_SEARCH_RESULTS, 0) {
            Ok(h) => h,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response()
            }
        }
    };

    let total = hits.len();
    let first_result = hits.into_iter().next().map(|h| {
        hit_to_result(
            h.id,
            h.source_path,
            h.line_start,
            h.line_end,
            h.text,
            h.distance,
        )
    });

    // Create cursor
    let cursor = state.cursor_manager.create(query_embedding, total);

    (
        StatusCode::OK,
        Json(SearchResponse {
            cursor,
            result: first_result,
            remaining: total.saturating_sub(1),
        }),
    )
        .into_response()
}

async fn handle_next(
    State(state): State<Arc<AppState>>,
    Json(req): Json<NextRequest>,
) -> impl IntoResponse {
    let Some((query_embedding, offset, remaining)) = state.cursor_manager.advance(&req.cursor)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "cursor not found or expired".into(),
            }),
        )
            .into_response();
    };

    let hits = {
        let store = state.store.lock().await;
        match store.search(&query_embedding, 1, offset) {
            Ok(h) => h,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response()
            }
        }
    };

    let result = hits.into_iter().next().map(|h| {
        hit_to_result(
            h.id,
            h.source_path,
            h.line_start,
            h.line_end,
            h.text,
            h.distance,
        )
    });

    (
        StatusCode::OK,
        Json(SearchResponse {
            cursor: req.cursor,
            result,
            remaining,
        }),
    )
        .into_response()
}

async fn handle_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let store = state.store.lock().await;

    match store.counts() {
        Ok((sources, chunks)) => Json(HealthResponse {
            status: "ok".into(),
            sources,
            chunks,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

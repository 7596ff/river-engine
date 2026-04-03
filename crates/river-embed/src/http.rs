//! HTTP server and endpoints.

use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use river_snowflake::{AgentBirth, GeneratorCache};

use crate::embed::EmbedClient;
use crate::index::IndexError;
use crate::search::{hit_to_result, CursorManager, SearchResponse};
use crate::store::Store;

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
    let result = index_content_async(&state, &req.source, &req.content, state.birth).await;

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

async fn index_content_async(
    state: &AppState,
    source: &str,
    content: &str,
    birth: AgentBirth,
) -> Result<(bool, usize), IndexError> {
    use crate::chunk::{chunk_markdown, ChunkConfig};
    use river_snowflake::SnowflakeType;
    use sha2::{Digest, Sha256};

    if content.trim().is_empty() {
        return Err(IndexError::EmptyContent);
    }

    // Hash content
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    // Check if update needed (synchronous db access)
    let needs_update = {
        let store = state.store.lock().unwrap();
        store.needs_update(source, &hash)?
    };

    if !needs_update {
        return Ok((false, 0));
    }

    // Delete existing chunks
    {
        let store = state.store.lock().unwrap();
        store.delete_source(source)?;
    }

    // Chunk content
    let config = ChunkConfig::default();
    let text_chunks = chunk_markdown(content, &config);

    if text_chunks.is_empty() {
        return Ok((true, 0));
    }

    // Generate embeddings (async)
    let texts: Vec<String> = text_chunks.iter().map(|c| c.text.clone()).collect();
    let embeddings = state.embed_client.embed(&texts).await?;

    // Store source and chunks
    {
        let store = state.store.lock().unwrap();
        store.upsert_source(source, &hash)?;

        for (chunk, embedding) in text_chunks.iter().zip(embeddings.iter()) {
            let id = state.id_cache.next_id(birth, SnowflakeType::Embedding);
            store.insert_chunk(
                &id.to_string(),
                source,
                chunk.line_start,
                chunk.line_end,
                &chunk.text,
                embedding,
            )?;
        }
    }

    Ok((true, text_chunks.len()))
}

async fn handle_delete(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let store = state.store.lock().unwrap();

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
        let store = state.store.lock().unwrap();
        match store.search(&query_embedding, 100, 0) {
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
        let store = state.store.lock().unwrap();
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
    let store = state.store.lock().unwrap();

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

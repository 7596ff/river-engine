//! HTTP API routes

use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Health check response
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// Incoming message request
#[derive(Deserialize)]
pub struct IncomingMessage {
    pub adapter: String,
    pub event_type: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    pub message_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct Author {
    pub id: String,
    pub name: String,
}

/// Create the router with all routes
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/incoming", post(handle_incoming))
        .route("/tools", get(list_tools))
        .route("/context/status", get(context_status))
        .with_state(state)
}

async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn handle_incoming(
    State(_state): State<Arc<AppState>>,
    Json(msg): Json<IncomingMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    tracing::info!(
        "Received message from {} in {}: {}",
        msg.author.name,
        msg.channel,
        msg.content
    );

    // TODO: Queue message and trigger tool loop (implemented in later plan)
    Ok(Json(serde_json::json!({
        "status": "queued",
        "channel": msg.channel
    })))
}

async fn list_tools(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<crate::tools::ToolSchema>> {
    let executor = state.tool_executor.read().await;
    Json(executor.schemas())
}

async fn context_status(
    State(state): State<Arc<AppState>>,
) -> Json<river_core::ContextStatus> {
    let executor = state.tool_executor.read().await;
    Json(executor.context_status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::state::GatewayConfig;
    use crate::tools::ToolRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use river_core::AgentBirth;
    use std::path::PathBuf;
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth: AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap(),
        };

        let db = Database::open_in_memory().unwrap();
        let registry = ToolRegistry::new();
        Arc::new(AppState::new(config, db, registry))
    }

    #[tokio::test]
    async fn test_health_check() {
        let app = create_router(test_state());

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_tools() {
        let app = create_router(test_state());

        let response = app
            .oneshot(Request::builder().uri("/tools").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_handle_incoming() {
        let app = create_router(test_state());

        let body = serde_json::json!({
            "adapter": "discord",
            "event_type": "message",
            "channel": "general",
            "author": {
                "id": "user123",
                "name": "Alice"
            },
            "content": "Hello, world!"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/incoming")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_context_status() {
        let app = create_router(test_state());

        let response = app
            .oneshot(Request::builder().uri("/context/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}

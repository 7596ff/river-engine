//! HTTP API routes

use crate::inbox::{format_inbox_line, build_discord_path, append_line};
use crate::r#loop::LoopEvent;
use crate::state::AppState;
use axum::{
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Validate bearer token from Authorization header
fn validate_auth(headers: &HeaderMap, expected_token: Option<&str>) -> Result<(), StatusCode> {
    let Some(expected) = expected_token else {
        // No auth configured, allow all requests
        return Ok(());
    };

    let auth_header = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !auth_header.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth_header[7..]; // Skip "Bearer "
    if token == expected {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Health check response
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// Incoming message request
#[derive(Debug, Clone, Deserialize)]
pub struct IncomingMessage {
    pub adapter: String,
    pub event_type: String,
    pub channel: String,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub guild_name: Option<String>,
    pub author: Author,
    pub content: String,
    pub message_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    /// Priority level (defaults to Interactive for user messages)
    #[serde(default = "default_priority")]
    pub priority: river_core::Priority,
}

fn default_priority() -> river_core::Priority {
    river_core::Priority::Interactive
}

#[derive(Debug, Clone, Deserialize)]
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
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(msg): Json<IncomingMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    tracing::info!(
        adapter = %msg.adapter,
        event_type = %msg.event_type,
        channel = %msg.channel,
        author_id = %msg.author.id,
        author_name = %msg.author.name,
        content_len = msg.content.len(),
        content_preview = %msg.content.chars().take(100).collect::<String>(),
        message_id = ?msg.message_id,
        priority = ?msg.priority,
        "Received incoming message"
    );

    // Validate authentication
    if let Err(status) = validate_auth(&headers, state.auth_token.as_deref()) {
        tracing::warn!(
            status = %status,
            has_auth_header = headers.get(AUTHORIZATION).is_some(),
            "Authentication failed for incoming message"
        );
        return Err(status);
    }
    tracing::debug!("Authentication passed");

    // Build inbox path based on adapter
    let inbox_path = if msg.adapter == "discord" {
        build_discord_path(
            &state.config.workspace,
            msg.guild_id.as_deref(),
            msg.guild_name.as_deref(),
            &msg.channel,
            msg.channel_name.as_deref().unwrap_or("unknown"),
        )
    } else {
        // Generic path for other adapters
        state.config.workspace
            .join("inbox")
            .join(&msg.adapter)
            .join(format!("{}.txt", msg.channel))
    };

    // Format and write inbox line
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let message_id = msg.message_id.as_deref().unwrap_or("unknown");
    let line = format_inbox_line(
        &timestamp,
        message_id,
        &msg.author.name,
        &msg.author.id,
        &msg.content,
    );

    if let Err(e) = append_line(&inbox_path, &line) {
        tracing::error!(error = %e, path = %inbox_path.display(), "Failed to write to inbox");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    tracing::debug!(path = %inbox_path.display(), "Message written to inbox");

    // Send inbox update to the loop
    if state.loop_tx.send(LoopEvent::InboxUpdate(vec![inbox_path.clone()])).await.is_err() {
        tracing::error!("Failed to send inbox update to loop - channel closed");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    tracing::info!(
        channel = %msg.channel,
        author = %msg.author.name,
        inbox_path = %inbox_path.display(),
        "Message delivered to inbox"
    );

    Ok(Json(serde_json::json!({
        "status": "delivered",
        "inbox_path": inbox_path.to_string_lossy()
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
    use crate::r#loop::MessageQueue;
    use crate::state::GatewayConfig;
    use crate::subagent::SubagentManager;
    use crate::tools::ToolRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use river_core::{AgentBirth, SnowflakeGenerator};
    use std::path::PathBuf;
    use tokio::sync::{mpsc, RwLock};
    use tower::ServiceExt;

    fn test_state() -> (Arc<AppState>, mpsc::Receiver<LoopEvent>) {
        let agent_birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth,
            agent_name: "test-agent".to_string(),
            embedding: None,
            redis: None,
        };

        let db = Arc::new(std::sync::Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let (loop_tx, loop_rx) = mpsc::channel(256);
        let message_queue = Arc::new(MessageQueue::new());
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));
        let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)));
        // No auth token for basic tests - tests that need auth should set it explicitly
        (Arc::new(AppState::new(config, db, registry, None, None, loop_tx, message_queue, None, subagent_manager)), loop_rx)
    }

    #[tokio::test]
    async fn test_health_check() {
        let (state, _rx) = test_state();
        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_tools() {
        let (state, _rx) = test_state();
        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/tools").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_handle_incoming() {
        let (state, _rx) = test_state();
        let app = create_router(state);

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
        let (state, _rx) = test_state();
        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/context/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    fn test_state_with_auth(token: &str) -> (Arc<AppState>, mpsc::Receiver<LoopEvent>) {
        let agent_birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth,
            agent_name: "test-agent".to_string(),
            embedding: None,
            redis: None,
        };

        let db = Arc::new(std::sync::Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let (loop_tx, loop_rx) = mpsc::channel(256);
        let message_queue = Arc::new(MessageQueue::new());
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));
        let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)));
        (Arc::new(AppState::new(config, db, registry, None, None, loop_tx, message_queue, Some(token.to_string()), subagent_manager)), loop_rx)
    }

    #[tokio::test]
    async fn test_incoming_requires_auth_when_configured() {
        let (state, _rx) = test_state_with_auth("secret-token");
        let app = create_router(state);

        let body = serde_json::json!({
            "adapter": "discord",
            "event_type": "message",
            "channel": "general",
            "author": { "id": "user123", "name": "Alice" },
            "content": "Hello"
        });

        // Request without auth should be rejected
        let response = app
            .clone()
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

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_incoming_accepts_valid_token() {
        let (state, _rx) = test_state_with_auth("secret-token");
        let app = create_router(state);

        let body = serde_json::json!({
            "adapter": "discord",
            "event_type": "message",
            "channel": "general",
            "author": { "id": "user123", "name": "Alice" },
            "content": "Hello"
        });

        // Request with valid auth should succeed
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/incoming")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer secret-token")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_incoming_rejects_invalid_token() {
        let (state, _rx) = test_state_with_auth("secret-token");
        let app = create_router(state);

        let body = serde_json::json!({
            "adapter": "discord",
            "event_type": "message",
            "channel": "general",
            "author": { "id": "user123", "name": "Alice" },
            "content": "Hello"
        });

        // Request with wrong token should be rejected
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/incoming")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

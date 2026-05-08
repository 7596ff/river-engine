//! HTTP server — receives outbound messages from the gateway

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use river_adapter::{SendRequest, SendResponse};
use chrono::Local;

use crate::state::{ChatLine, SharedState};

/// Health check response
#[derive(serde::Serialize)]
struct HealthResponse {
    healthy: bool,
}

/// Create the HTTP router
pub fn create_router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/send", post(handle_send))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { healthy: true })
}

async fn handle_send(
    State(state): State<SharedState>,
    Json(req): Json<SendRequest>,
) -> Result<Json<SendResponse>, StatusCode> {
    let line = ChatLine {
        timestamp: Local::now(),
        sender: "agent".into(),
        content: req.content,
        is_agent: true,
    };

    state.push_message(line);

    let msg_id = format!("tui-{}", chrono::Utc::now().timestamp_millis());

    Ok(Json(SendResponse {
        success: true,
        message_id: Some(msg_id),
        error: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_check() {
        let state = SharedState::new();
        let app = create_router(state);
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_handle_send() {
        let state = SharedState::new();
        let app = create_router(state.clone());

        let body = serde_json::json!({
            "channel": "terminal",
            "content": "Hello from agent!"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/send")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let msgs = state.get_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello from agent!");
        assert!(msgs[0].is_agent);
    }
}

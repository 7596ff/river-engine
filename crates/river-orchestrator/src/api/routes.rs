//! HTTP route handlers

use crate::agents::AgentStatus;
use crate::models::ModelInfo;
use crate::state::OrchestratorState;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Health check response
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub agents_registered: usize,
}

/// Heartbeat request
#[derive(Deserialize)]
pub struct HeartbeatRequest {
    pub agent: String,
    pub gateway_url: String,
}

/// Heartbeat response
#[derive(Serialize)]
pub struct HeartbeatResponse {
    pub acknowledged: bool,
}

/// Create the router with all routes
pub fn create_router(state: Arc<OrchestratorState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/heartbeat", post(handle_heartbeat))
        .route("/agents/status", get(agents_status))
        .route("/models/available", get(models_available))
        .with_state(state)
}

async fn health_check(State(state): State<Arc<OrchestratorState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        agents_registered: state.agent_count().await,
    })
}

async fn handle_heartbeat(
    State(state): State<Arc<OrchestratorState>>,
    Json(req): Json<HeartbeatRequest>,
) -> Json<HeartbeatResponse> {
    tracing::debug!("Heartbeat from {} at {}", req.agent, req.gateway_url);
    state.heartbeat(req.agent, req.gateway_url).await;
    Json(HeartbeatResponse { acknowledged: true })
}

async fn agents_status(
    State(state): State<Arc<OrchestratorState>>,
) -> Json<Vec<AgentStatus>> {
    Json(state.agent_statuses().await)
}

async fn models_available(
    State(state): State<Arc<OrchestratorState>>,
) -> Json<Vec<ModelInfo>> {
    Json(state.models.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OrchestratorConfig;
    use crate::models::ModelProvider;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> Arc<OrchestratorState> {
        Arc::new(OrchestratorState::new(OrchestratorConfig::default(), vec![]))
    }

    fn test_state_with_models() -> Arc<OrchestratorState> {
        let models = vec![
            ModelInfo::new("qwen3-32b".to_string(), ModelProvider::Local),
        ];
        Arc::new(OrchestratorState::new(OrchestratorConfig::default(), models))
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
    async fn test_heartbeat() {
        let state = test_state();
        let app = create_router(state.clone());

        let body = serde_json::json!({
            "agent": "thomas",
            "gateway_url": "http://localhost:3000"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/heartbeat")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_agents_status() {
        let state = test_state();
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;

        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/agents/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_models_available() {
        let app = create_router(test_state_with_models());

        let response = app
            .oneshot(Request::builder().uri("/models/available").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}

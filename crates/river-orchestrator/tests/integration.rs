//! Integration tests for orchestrator

use axum::body::Body;
use axum::http::{Request, StatusCode};
use river_orchestrator::{
    api::create_router, config::OrchestratorConfig, process::ProcessConfig,
    resources::ResourceConfig, OrchestratorState,
};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn test_full_workflow() {
    // Create orchestrator (no local models for basic integration test)
    let state = Arc::new(OrchestratorState::new(
        OrchestratorConfig::default(),
        vec![],
        vec![],
        ResourceConfig::default(),
        ProcessConfig::default(),
        "test-token".to_string(),
    ));
    let app = create_router(state.clone());

    // 1. Check health (no agents yet) — no auth needed
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 2. Send heartbeat
    let heartbeat = serde_json::json!({
        "agent": "test-agent",
        "gateway_url": "http://localhost:3000"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/heartbeat")
                .header("content-type", "application/json")
                .header("authorization", "Bearer test-token")
                .body(Body::from(serde_json::to_string(&heartbeat).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 3. Check agents status
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/agents/status")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify agent is registered
    assert_eq!(state.agent_count().await, 1);

    // 4. Check models
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/models/available")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

//! Integration tests for river-snowflake HTTP server.

#![cfg(feature = "server")]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use river_snowflake::{server, AgentBirth, GeneratorCache};
use tower::ServiceExt;

fn create_app() -> axum::Router {
    let state = Arc::new(server::AppState {
        cache: GeneratorCache::new(),
    });
    server::router(state)
}

fn valid_birth() -> u64 {
    AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap().as_u64()
}

#[tokio::test]
async fn test_get_id_success() {
    let app = create_app();
    let birth = valid_birth();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/id/message?birth={}", birth))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_id_invalid_type() {
    let app = create_app();
    let birth = valid_birth();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/id/invalid_type?birth={}", birth))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_health_endpoint() {
    let app = create_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

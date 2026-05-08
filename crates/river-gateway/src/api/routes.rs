//! HTTP API routes

use crate::metrics::{get_rss_bytes, LoopStateLabel};
use crate::policy::HealthStatus;
use crate::state::AppState;
use river_adapter::{RegisterRequest, RegisterResponse};
use axum::{
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
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

/// Rich health check response
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_seconds: u64,
    agent: AgentInfo,
    loop_state: LoopInfo,
    context: ContextInfo,
    resources: ResourceInfo,
    counters: CounterInfo,
    policy: PolicyInfo,
}

#[derive(Serialize)]
struct AgentInfo {
    name: String,
    birth: DateTime<Utc>,
}

#[derive(Serialize)]
struct LoopInfo {
    state: LoopStateLabel,
    last_wake: Option<DateTime<Utc>>,
    last_settle: Option<DateTime<Utc>>,
    turns_since_restart: u64,
}

#[derive(Serialize)]
struct ContextInfo {
    current_tokens: u64,
    limit_tokens: u64,
    usage_percent: f64,
    context_id: Option<String>,
    rotations: u64,
}

#[derive(Serialize)]
struct ResourceInfo {
    db_size_bytes: u64,
    rss_bytes: u64,
}

#[derive(Serialize)]
struct CounterInfo {
    model_calls: u64,
    tool_calls: u64,
    tool_errors: u64,
}

#[derive(Serialize)]
struct PolicyInfo {
    health_status: HealthStatus,
    consecutive_errors: u32,
    current_backoff_secs: u64,
    recovery_attempts: u32,
    attention_file: Option<String>,
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
        .route("/adapters/register", post(register_adapter))
        .with_state(state)
}

async fn health_check(State(state): State<Arc<AppState>>) -> (StatusCode, Json<HealthResponse>) {
    let metrics = state.metrics.read().await;
    let policy = state.policy.read().await;

    // Get current DB size
    let db_size = std::fs::metadata(state.config.db_path())
        .map(|m| m.len())
        .unwrap_or(0);

    // Get current RSS
    let rss = get_rss_bytes();

    let uptime = Utc::now()
        .signed_duration_since(metrics.start_time)
        .num_seconds()
        .max(0) as u64;

    let health_status = policy.status();
    let policy_info = PolicyInfo {
        health_status,
        consecutive_errors: policy.consecutive_errors(),
        current_backoff_secs: policy.error_backoff().as_secs(),
        recovery_attempts: policy.recovery_attempts(),
        attention_file: policy.attention_file_path(),
    };

    // Determine HTTP status code based on health status
    let http_status = match health_status {
        HealthStatus::Healthy | HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::NeedsAttention => StatusCode::SERVICE_UNAVAILABLE,
    };

    // Determine status string based on health status
    let status_str = match health_status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Degraded => "degraded",
        HealthStatus::NeedsAttention => "needs_attention",
    };

    (http_status, Json(HealthResponse {
        status: status_str,
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: uptime,
        agent: AgentInfo {
            name: metrics.agent_name.clone(),
            birth: metrics.agent_birth,
        },
        loop_state: LoopInfo {
            state: metrics.loop_state,
            last_wake: metrics.last_wake,
            last_settle: metrics.last_settle,
            turns_since_restart: metrics.turns_since_restart,
        },
        context: ContextInfo {
            current_tokens: metrics.context_tokens,
            limit_tokens: metrics.context_limit,
            usage_percent: metrics.context_usage_percent(),
            context_id: metrics.context_id.clone(),
            rotations: metrics.rotations_since_restart,
        },
        resources: ResourceInfo {
            db_size_bytes: db_size,
            rss_bytes: rss,
        },
        counters: CounterInfo {
            model_calls: metrics.model_calls,
            tool_calls: metrics.tool_calls,
            tool_errors: metrics.tool_errors,
        },
        policy: policy_info,
    }))
}

async fn handle_incoming(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(msg): Json<IncomingMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    tracing::info!(
        adapter = %msg.adapter,
        channel = %msg.channel,
        author_name = %msg.author.name,
        content_len = msg.content.len(),
        "Received incoming message"
    );

    // Validate authentication
    if let Err(status) = validate_auth(&headers, state.auth_token.as_deref()) {
        return Err(status);
    }

    // Generate snowflake ID
    let snowflake = state.snowflake_gen.next_id(river_core::SnowflakeType::Message);
    let snowflake_str = snowflake.to_string();

    // Build channel log entry
    let entry = crate::channels::MessageEntry::incoming(
        snowflake_str.clone(),
        msg.author.name.clone(),
        msg.author.id.clone(),
        msg.content.clone(),
        msg.adapter.clone(),
        msg.message_id.clone(),
    );

    // Append to channel log
    let channels_dir = state.config.workspace.join("channels");
    let log = crate::channels::ChannelLog::open(&channels_dir, &msg.adapter, &msg.channel);

    if let Err(e) = log.append_entry(&entry).await {
        tracing::error!(error = %e, "Failed to write to channel log");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Only push notification after successful write
    let channel_key = format!("{}_{}", msg.adapter, msg.channel);
    state.message_queue.push(crate::queue::ChannelNotification {
        channel: channel_key.clone(),
        snowflake_id: snowflake_str,
    });

    tracing::info!(channel = %channel_key, "Message delivered to channel log");

    Ok(Json(serde_json::json!({
        "status": "delivered",
        "channel": channel_key,
    })))
}

async fn list_tools(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<crate::tools::ToolSchema>> {
    let executor = state.tool_executor.read().await;
    Json(executor.schemas())
}

async fn register_adapter(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, StatusCode> {
    // Validate authentication
    if let Err(status) = validate_auth(&headers, state.auth_token.as_deref()) {
        tracing::warn!(
            adapter = %request.adapter.name,
            "Authentication failed for adapter registration"
        );
        return Err(status);
    }

    tracing::info!(
        adapter = %request.adapter.name,
        version = %request.adapter.version,
        url = %request.adapter.url,
        features = ?request.adapter.features,
        "Registering adapter"
    );

    // Insert into the tool adapter registry so send_message can find it
    let adapter_config = crate::tools::AdapterConfig {
        name: request.adapter.name.clone(),
        outbound_url: format!("{}/send", request.adapter.url),
        read_url: Some(format!("{}/read", request.adapter.url)),
        features: request.adapter.features.clone(),
    };
    {
        let mut reg = state.adapter_registry.write().await;
        reg.register(adapter_config);
    }

    tracing::info!(adapter = %request.adapter.name, "Adapter registered");

    Ok(Json(RegisterResponse {
        accepted: true,
        error: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::metrics::AgentMetrics;
    use crate::policy::HealthPolicy;
    use crate::queue::MessageQueue;
    use crate::state::GatewayConfig;
    use crate::subagent::SubagentManager;
    use crate::tools::ToolRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use river_core::{AgentBirth, SnowflakeGenerator};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
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
        let message_queue = Arc::new(MessageQueue::new());
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));
        let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)));
        let metrics = Arc::new(RwLock::new(AgentMetrics::new(
            "test-agent".to_string(),
            Utc::now(),
            65536,
        )));
        let policy = Arc::new(RwLock::new(HealthPolicy::new(
            "test-agent".to_string(),
            PathBuf::from("/tmp/test"),
        )));
        Arc::new(AppState::new(config, db, registry, None, None, message_queue, None, subagent_manager, metrics, policy))
    }

    #[tokio::test]
    async fn test_health_check() {
        let state = test_state();
        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_tools() {
        let state = test_state();
        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/tools").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_handle_incoming() {
        let state = test_state();
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

    fn test_state_with_auth(token: &str) -> Arc<AppState> {
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
        let message_queue = Arc::new(MessageQueue::new());
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));
        let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)));
        let metrics = Arc::new(RwLock::new(AgentMetrics::new(
            "test-agent".to_string(),
            Utc::now(),
            65536,
        )));
        let policy = Arc::new(RwLock::new(HealthPolicy::new(
            "test-agent".to_string(),
            PathBuf::from("/tmp/test"),
        )));
        Arc::new(AppState::new(config, db, registry, None, None, message_queue, Some(token.to_string()), subagent_manager, metrics, policy))
    }

    #[tokio::test]
    async fn test_incoming_requires_auth_when_configured() {
        let state = test_state_with_auth("secret-token");
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
        let state = test_state_with_auth("secret-token");
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
        let state = test_state_with_auth("secret-token");
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

    #[tokio::test]
    async fn test_health_returns_rich_response() {
        let state = test_state();
        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(health["status"], "healthy");
        assert!(health["agent"]["name"].is_string());
        assert!(health["loop_state"]["state"].is_string());
        assert!(health["context"]["usage_percent"].is_number());
        assert!(health["resources"]["rss_bytes"].is_number());
        assert!(health["counters"]["tool_calls"].is_number());
    }

    #[tokio::test]
    async fn test_health_includes_policy_info() {
        let state = test_state();
        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Check policy info is present with expected structure
        assert_eq!(health["policy"]["health_status"], "healthy");
        assert_eq!(health["policy"]["consecutive_errors"], 0);
        assert_eq!(health["policy"]["current_backoff_secs"], 0);
        assert_eq!(health["policy"]["recovery_attempts"], 0);
        // attention_file may be null or a string path, just check field exists
        assert!(health["policy"].get("attention_file").is_some());
    }

    #[tokio::test]
    async fn test_health_returns_503_for_needs_attention() {
        let state = test_state();

        // Escalate the policy to NeedsAttention
        {
            let mut policy = state.policy.write().await;
            let _ = policy.escalate("Test escalation", "Test context");
        }

        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(health["status"], "needs_attention");
        assert_eq!(health["policy"]["health_status"], "needs_attention");
    }

    #[tokio::test]
    async fn test_health_returns_200_for_degraded() {
        let state = test_state();

        // Degrade the policy (simulate some errors)
        {
            let mut policy = state.policy.write().await;
            policy.on_turn_complete(1, 1); // 100% failure
            policy.on_turn_complete(1, 1); // Another failure
        }

        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        // Degraded still returns 200, just with degraded status
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(health["status"], "degraded");
        assert_eq!(health["policy"]["health_status"], "degraded");
        assert!(health["policy"]["consecutive_errors"].as_u64().unwrap() > 0);
    }
}

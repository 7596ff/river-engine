//! HTTP route handlers

use crate::agents::AgentStatus;
use crate::state::{LocalModelStatus, ModelRequestResponse, OrchestratorState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
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

/// Model request
#[derive(Deserialize)]
pub struct ModelRequest {
    pub model: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
}

fn default_priority() -> String {
    "interactive".to_string()
}

fn default_timeout() -> u32 {
    120
}

/// Model request response
#[derive(Serialize)]
pub struct ModelRequestApiResponse {
    pub status: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Model release request
#[derive(Deserialize)]
pub struct ModelReleaseRequest {
    pub model: String,
}

/// Model release response
#[derive(Serialize)]
pub struct ModelReleaseResponse {
    pub acknowledged: bool,
}

/// Local model info for API
#[derive(Serialize)]
pub struct LocalModelApiResponse {
    pub id: String,
    pub path: String,
    pub architecture: String,
    pub parameters: String,
    pub quantization: String,
    pub estimated_vram_gb: f64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_seconds: Option<u64>,
}

/// External model info for API
#[derive(Serialize)]
pub struct ExternalModelApiResponse {
    pub id: String,
    pub provider: String,
    pub endpoint: String,
    pub status: String,
}

/// Device resource info for API
#[derive(Serialize)]
pub struct DeviceApiResponse {
    pub id: String,
    pub total_memory_gb: f64,
    pub used_memory_gb: f64,
    pub available_memory_gb: f64,
}

/// Models available response
#[derive(Serialize)]
pub struct ModelsAvailableResponse {
    pub local: Vec<LocalModelApiResponse>,
    pub external: Vec<ExternalModelApiResponse>,
    pub resources: ResourcesApiResponse,
    pub llama_server_available: bool,
}

/// Resources API response
#[derive(Serialize)]
pub struct ResourcesApiResponse {
    pub devices: Vec<DeviceApiResponse>,
    pub loaded_models: Vec<LoadedModelApiResponse>,
}

/// Loaded model info for resources endpoint
#[derive(Serialize)]
pub struct LoadedModelApiResponse {
    pub model_id: String,
    pub device: String,
    pub vram_bytes: u64,
    pub port: u16,
    pub pid: u32,
    pub uptime_seconds: u64,
    pub idle_seconds: u64,
}

/// Create the router with all routes
pub fn create_router(state: Arc<OrchestratorState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/heartbeat", post(handle_heartbeat))
        .route("/agents/status", get(agents_status))
        .route("/models/available", get(models_available))
        .route("/model/request", post(model_request))
        .route("/model/release", post(model_release))
        .route("/resources", get(resources))
        .with_state(state)
}

/// Validate bearer token from Authorization header
fn validate_auth(headers: &HeaderMap, expected_token: &str) -> Result<(), StatusCode> {
    let auth_header = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if river_core::validate_bearer(auth_header, expected_token) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
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
    headers: HeaderMap,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, StatusCode> {
    validate_auth(&headers, &state.auth_token)?;
    tracing::debug!("Heartbeat from {} at {}", req.agent, req.gateway_url);
    state.heartbeat(req.agent, req.gateway_url).await;
    Ok(Json(HeartbeatResponse { acknowledged: true }))
}

async fn agents_status(
    State(state): State<Arc<OrchestratorState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AgentStatus>>, StatusCode> {
    validate_auth(&headers, &state.auth_token)?;
    Ok(Json(state.agent_statuses().await))
}

async fn models_available(
    State(state): State<Arc<OrchestratorState>>,
    headers: HeaderMap,
) -> Result<Json<ModelsAvailableResponse>, StatusCode> {
    validate_auth(&headers, &state.auth_token)?;
    let local_models = state.local_models.read().await;

    let local: Vec<LocalModelApiResponse> = local_models
        .values()
        .map(|entry| {
            let (status, endpoint, device, idle_seconds) = match &entry.status {
                LocalModelStatus::Available => ("available".to_string(), None, None, None),
                LocalModelStatus::Loading => ("loading".to_string(), None, None, None),
                LocalModelStatus::Loaded { endpoint, device, idle_seconds } => (
                    "loaded".to_string(),
                    Some(endpoint.clone()),
                    Some(device.to_api_string()),
                    Some(*idle_seconds),
                ),
                LocalModelStatus::Error(e) => (format!("error: {}", e), None, None, None),
            };

            LocalModelApiResponse {
                id: entry.model.id.clone(),
                path: entry.model.path.display().to_string(),
                architecture: entry.model.metadata.architecture.clone(),
                parameters: format_parameters(entry.model.metadata.parameters),
                quantization: format!("{:?}", entry.model.metadata.quantization),
                estimated_vram_gb: entry.model.metadata.estimate_vram() as f64 / 1_073_741_824.0,
                status,
                endpoint,
                device,
                idle_seconds,
            }
        })
        .collect();

    let external: Vec<ExternalModelApiResponse> = state.external_models
        .iter()
        .map(|m| ExternalModelApiResponse {
            id: m.id.clone(),
            provider: m.provider.clone(),
            endpoint: m.endpoint(),
            status: "available".to_string(),
        })
        .collect();

    let device_resources = state.resource_tracker.get_all_resources().await;
    let devices: Vec<DeviceApiResponse> = device_resources
        .iter()
        .map(|d| DeviceApiResponse {
            id: d.device.to_api_string(),
            total_memory_gb: d.total_memory as f64 / 1_073_741_824.0,
            used_memory_gb: d.allocated as f64 / 1_073_741_824.0,
            available_memory_gb: d.available as f64 / 1_073_741_824.0,
        })
        .collect();

    // Get loaded models from process manager
    let processes = state.process_manager.get_all_processes().await;
    let loaded_models: Vec<LoadedModelApiResponse> = processes
        .iter()
        .map(|p| {
            let vram_bytes = local_models
                .get(&p.model_id)
                .map(|e| e.model.metadata.estimate_vram())
                .unwrap_or(0);
            LoadedModelApiResponse {
                model_id: p.model_id.clone(),
                device: p.device.to_api_string(),
                vram_bytes,
                port: p.port,
                pid: p.pid,
                uptime_seconds: p.uptime_seconds,
                idle_seconds: p.idle_seconds,
            }
        })
        .collect();

    Ok(Json(ModelsAvailableResponse {
        local,
        external,
        resources: ResourcesApiResponse { devices, loaded_models },
        llama_server_available: state.llama_server_available(),
    }))
}

async fn model_request(
    State(state): State<Arc<OrchestratorState>>,
    headers: HeaderMap,
    Json(req): Json<ModelRequest>,
) -> Result<Json<ModelRequestApiResponse>, (StatusCode, Json<ModelRequestApiResponse>)> {
    if let Err(_) = validate_auth(&headers, &state.auth_token) {
        return Err((StatusCode::UNAUTHORIZED, Json(ModelRequestApiResponse {
            status: "error".to_string(),
            model: req.model,
            endpoint: None,
            device: None,
            warning: None,
            error: Some("Unauthorized".to_string()),
        })));
    }
    match state.request_model(&req.model, req.timeout_seconds).await {
        Ok(ModelRequestResponse::Ready { endpoint, device, warning }) => {
            Ok(Json(ModelRequestApiResponse {
                status: "ready".to_string(),
                model: req.model,
                endpoint: Some(endpoint),
                device: device.map(|d| d.to_api_string()),
                warning,
                error: None,
            }))
        }
        Ok(ModelRequestResponse::Loading { estimated_seconds: _ }) => {
            Ok(Json(ModelRequestApiResponse {
                status: "loading".to_string(),
                model: req.model,
                endpoint: None,
                device: None,
                warning: None,
                error: None,
            }))
        }
        Err(e) => {
            Err((
                StatusCode::BAD_REQUEST,
                Json(ModelRequestApiResponse {
                    status: "error".to_string(),
                    model: req.model,
                    endpoint: None,
                    device: None,
                    warning: None,
                    error: Some(e.to_string()),
                }),
            ))
        }
    }
}

async fn model_release(
    State(state): State<Arc<OrchestratorState>>,
    headers: HeaderMap,
    Json(req): Json<ModelReleaseRequest>,
) -> Result<Json<ModelReleaseResponse>, StatusCode> {
    validate_auth(&headers, &state.auth_token)?;
    let acknowledged = state.release_model(&req.model).await;
    Ok(Json(ModelReleaseResponse { acknowledged }))
}

async fn resources(
    State(state): State<Arc<OrchestratorState>>,
    headers: HeaderMap,
) -> Result<Json<ResourcesApiResponse>, StatusCode> {
    validate_auth(&headers, &state.auth_token)?;
    let device_resources = state.resource_tracker.get_all_resources().await;
    let devices: Vec<DeviceApiResponse> = device_resources
        .iter()
        .map(|d| DeviceApiResponse {
            id: d.device.to_api_string(),
            total_memory_gb: d.total_memory as f64 / 1_073_741_824.0,
            used_memory_gb: d.allocated as f64 / 1_073_741_824.0,
            available_memory_gb: d.available as f64 / 1_073_741_824.0,
        })
        .collect();

    // Get loaded models from process manager
    let processes = state.process_manager.get_all_processes().await;
    let local_models = state.local_models.read().await;
    let loaded_models: Vec<LoadedModelApiResponse> = processes
        .iter()
        .map(|p| {
            let vram_bytes = local_models
                .get(&p.model_id)
                .map(|e| e.model.metadata.estimate_vram())
                .unwrap_or(0);
            LoadedModelApiResponse {
                model_id: p.model_id.clone(),
                device: p.device.to_api_string(),
                vram_bytes,
                port: p.port,
                pid: p.pid,
                uptime_seconds: p.uptime_seconds,
                idle_seconds: p.idle_seconds,
            }
        })
        .collect();

    Ok(Json(ResourcesApiResponse { devices, loaded_models }))
}

fn format_parameters(params: u64) -> String {
    if params >= 1_000_000_000 {
        format!("{:.0}B", params as f64 / 1_000_000_000.0)
    } else if params >= 1_000_000 {
        format!("{:.0}M", params as f64 / 1_000_000.0)
    } else {
        format!("{}", params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OrchestratorConfig;
    use crate::process::ProcessConfig;
    use crate::resources::ResourceConfig;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state() -> Arc<OrchestratorState> {
        Arc::new(OrchestratorState::new(
            OrchestratorConfig::default(),
            vec![],
            vec![],
            ResourceConfig::default(),
            ProcessConfig::default(),
            "test-token".to_string(),
        ))
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
                    .header("authorization", "Bearer test-token")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_models_available() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/available")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_resources() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/resources")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}

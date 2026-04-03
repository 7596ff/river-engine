//! HTTP server and endpoints.

use crate::config::{Config, ModelConfig};
use crate::model::{ModelSwitchError, ModelSwitchRequest, ModelSwitchResponse};
use crate::registry::{push_registry, SharedRegistry};
use river_protocol::Registry;
use crate::respawn::{OutputAck, RespawnAction, SharedRespawnManager, WorkerOutput};
use crate::supervisor::{ProcessKey, SharedSupervisor};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use river_adapter::FeatureId;
use river_protocol::{Baton, Ground, Side};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub registry: SharedRegistry,
    pub supervisor: SharedSupervisor,
    pub respawn: SharedRespawnManager,
    pub client: reqwest::Client,
    pub dyad_locks: Arc<RwLock<HashMap<String, bool>>>,
    pub orchestrator_url: String,
}

/// Worker registration request.
#[derive(Debug, Deserialize)]
pub struct WorkerRegistrationRequest {
    pub endpoint: String,
    pub worker: WorkerRegistration,
}

#[derive(Debug, Deserialize)]
pub struct WorkerRegistration {
    pub dyad: String,
    pub side: Side,
}

/// Worker registration response.
#[derive(Debug, Serialize)]
pub struct WorkerRegistrationResponse {
    pub accepted: bool,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub model: WorkerModelConfig,
    pub ground: Ground,
    pub workspace: String,
    pub initial_message: Option<String>,
    pub start_sleeping: bool,
}

#[derive(Debug, Serialize)]
pub struct WorkerModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,
    pub context_limit: usize,
}

impl From<&ModelConfig> for WorkerModelConfig {
    fn from(m: &ModelConfig) -> Self {
        Self {
            endpoint: m.endpoint.clone(),
            name: m.name.clone(),
            api_key: m.api_key.clone(),
            context_limit: m.context_limit.unwrap_or(8192),
        }
    }
}

/// Adapter registration request.
#[derive(Debug, Deserialize)]
pub struct AdapterRegistrationRequest {
    pub endpoint: String,
    pub adapter: AdapterRegistration,
}

#[derive(Debug, Deserialize)]
pub struct AdapterRegistration {
    pub dyad: String,
    #[serde(rename = "type")]
    pub adapter_type: String,
    pub features: Vec<u16>,
}

/// Adapter registration response.
#[derive(Debug, Serialize)]
pub struct AdapterRegistrationResponse {
    pub accepted: bool,
    pub worker_endpoint: Option<String>,
    pub validated_features: Vec<u16>,
    pub config: Value,
}

/// Embed service registration request.
#[derive(Debug, Deserialize)]
pub struct EmbedRegistrationRequest {
    pub endpoint: String,
    pub embed: EmbedRegistration,
}

#[derive(Debug, Deserialize)]
pub struct EmbedRegistration {
    pub name: String,
}

/// Embed service registration response.
#[derive(Debug, Serialize)]
pub struct EmbedRegistrationResponse {
    pub accepted: bool,
    pub model: EmbedModelConfig,
}

#[derive(Debug, Serialize)]
pub struct EmbedModelConfig {
    pub endpoint: String,
    pub name: String,
    pub api_key: String,
    pub dimensions: usize,
}

/// Registration request (unified).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RegistrationRequest {
    Worker(WorkerRegistrationRequest),
    Adapter(AdapterRegistrationRequest),
    Embed(EmbedRegistrationRequest),
}

/// Registration error response.
#[derive(Debug, Serialize)]
pub struct RegistrationError {
    pub error: String,
}

/// Switch roles request.
#[derive(Debug, Deserialize)]
pub struct SwitchRolesRequest {
    pub dyad: String,
    pub side: Side,
}

/// Switch roles response.
#[derive(Debug, Serialize)]
pub struct SwitchRolesResponse {
    pub switched: bool,
    pub your_new_baton: Baton,
    pub partner_new_baton: Baton,
}

/// Switch roles error.
#[derive(Debug, Serialize)]
pub struct SwitchRolesError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Health response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub workers: usize,
    pub adapters: usize,
    pub embed_services: usize,
}

/// Build the router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/register", post(handle_register))
        .route("/model/switch", post(handle_model_switch))
        .route("/switch_roles", post(handle_switch_roles))
        .route("/worker/output", post(handle_worker_output))
        .route("/registry", get(handle_get_registry))
        .route("/health", get(handle_health))
        .with_state(state)
}

/// POST /register
async fn handle_register(
    State(state): State<AppState>,
    Json(req): Json<RegistrationRequest>,
) -> Result<Json<Value>, (StatusCode, Json<RegistrationError>)> {
    match req {
        RegistrationRequest::Worker(req) => {
            handle_worker_registration(state, req).await
        }
        RegistrationRequest::Adapter(req) => {
            handle_adapter_registration(state, req).await
        }
        RegistrationRequest::Embed(req) => {
            handle_embed_registration(state, req).await
        }
    }
}

async fn handle_worker_registration(
    state: AppState,
    req: WorkerRegistrationRequest,
) -> Result<Json<Value>, (StatusCode, Json<RegistrationError>)> {
    let dyad_config = state.config.dyads.get(&req.worker.dyad).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(RegistrationError {
                error: format!("Unknown dyad: {}", req.worker.dyad),
            }),
        )
    })?;

    // Get model config
    let model_name = match req.worker.side {
        Side::Left => &dyad_config.left_model,
        Side::Right => &dyad_config.right_model,
    };
    let model_config = state.config.models.get(model_name).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(RegistrationError {
                error: format!("Model not found: {}", model_name),
            }),
        )
    })?;

    // Determine baton based on initial_actor
    let baton = if req.worker.side == dyad_config.initial_actor {
        Baton::Actor
    } else {
        Baton::Spectator
    };

    // Check for respawn state
    let respawn_info = {
        let mgr = state.respawn.read().await;
        mgr.get_respawn_info(&req.worker.dyad, &req.worker.side).cloned()
    };

    let (initial_message, start_sleeping) = match respawn_info {
        Some(info) => (info.summary.clone(), info.start_sleeping),
        None => (None, false),
    };

    // Register in registry
    {
        let mut reg = state.registry.write().await;
        reg.register_worker(
            req.worker.dyad.clone(),
            req.worker.side.clone(),
            req.endpoint.clone(),
            baton.clone(),
            model_name.clone(),
            dyad_config.ground.clone(),
        );
    }

    // Get partner endpoint
    let partner_endpoint = {
        let reg = state.registry.read().await;
        reg.get_partner_endpoint(&req.worker.dyad, &req.worker.side)
    };

    // Update supervisor with endpoint
    {
        let mut sup = state.supervisor.write().await;
        sup.set_endpoint(
            &ProcessKey::Worker {
                dyad: req.worker.dyad.clone(),
                side: req.worker.side.clone(),
            },
            req.endpoint.clone(),
        );
    }

    // Clear respawn state
    {
        let mut mgr = state.respawn.write().await;
        mgr.clear(&req.worker.dyad, &req.worker.side);
    }

    // Push registry to all
    push_registry_to_all(&state).await;

    let response = WorkerRegistrationResponse {
        accepted: true,
        baton,
        partner_endpoint,
        model: WorkerModelConfig::from(model_config),
        ground: dyad_config.ground.clone(),
        workspace: dyad_config.workspace.to_string_lossy().to_string(),
        initial_message,
        start_sleeping,
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

async fn handle_adapter_registration(
    state: AppState,
    req: AdapterRegistrationRequest,
) -> Result<Json<Value>, (StatusCode, Json<RegistrationError>)> {
    let dyad_config = state.config.dyads.get(&req.adapter.dyad).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(RegistrationError {
                error: format!("Unknown dyad: {}", req.adapter.dyad),
            }),
        )
    })?;

    // Find adapter config
    let adapter_config = dyad_config
        .adapters
        .iter()
        .find(|a| a.adapter_type == req.adapter.adapter_type)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(RegistrationError {
                    error: format!(
                        "Unknown adapter type '{}' for dyad '{}'",
                        req.adapter.adapter_type, req.adapter.dyad
                    ),
                }),
            )
        })?;

    // Validate features
    let validated = validate_features(&req.adapter.features)?;

    // Register in registry
    {
        let mut reg = state.registry.write().await;
        reg.register_adapter(
            req.adapter.dyad.clone(),
            req.adapter.adapter_type.clone(),
            req.endpoint.clone(),
            req.adapter.features.clone(),
        );
    }

    // Get actor worker endpoint
    let worker_endpoint = {
        let reg = state.registry.read().await;
        // Find the actor for this dyad
        reg.get_worker_endpoint(&req.adapter.dyad, &dyad_config.initial_actor)
    };

    // Update supervisor with endpoint
    {
        let mut sup = state.supervisor.write().await;
        sup.set_endpoint(
            &ProcessKey::Adapter {
                dyad: req.adapter.dyad.clone(),
                adapter_type: req.adapter.adapter_type.clone(),
            },
            req.endpoint.clone(),
        );
    }

    // Push registry to all
    push_registry_to_all(&state).await;

    let response = AdapterRegistrationResponse {
        accepted: true,
        worker_endpoint,
        validated_features: validated.iter().map(|f| *f as u16).collect(),
        config: adapter_config.config.clone(),
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

async fn handle_embed_registration(
    state: AppState,
    req: EmbedRegistrationRequest,
) -> Result<Json<Value>, (StatusCode, Json<RegistrationError>)> {
    let embed_config = state.config.embed.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(RegistrationError {
                error: "No embed configuration".into(),
            }),
        )
    })?;

    let model_config = state.config.models.get(&embed_config.model).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(RegistrationError {
                error: format!("Embed model not found: {}", embed_config.model),
            }),
        )
    })?;

    // Register in registry
    {
        let mut reg = state.registry.write().await;
        reg.register_embed(req.embed.name.clone(), req.endpoint.clone());
    }

    // Update supervisor with endpoint
    {
        let mut sup = state.supervisor.write().await;
        sup.set_endpoint(
            &ProcessKey::Embed {
                name: req.embed.name.clone(),
            },
            req.endpoint.clone(),
        );
    }

    // Push registry to all
    push_registry_to_all(&state).await;

    let response = EmbedRegistrationResponse {
        accepted: true,
        model: EmbedModelConfig {
            endpoint: model_config.endpoint.clone(),
            name: model_config.name.clone(),
            api_key: model_config.api_key.clone(),
            dimensions: model_config.dimensions.unwrap_or(768),
        },
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

fn validate_features(features: &[u16]) -> Result<Vec<FeatureId>, (StatusCode, Json<RegistrationError>)> {
    let mut parsed = Vec::new();
    for &f in features {
        let id = FeatureId::try_from(f).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(RegistrationError {
                    error: format!("Unknown feature ID: {}", f),
                }),
            )
        })?;
        parsed.push(id);
    }

    // Check required features
    if !parsed.contains(&FeatureId::SendMessage) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(RegistrationError {
                error: "Missing required feature: SendMessage".into(),
            }),
        ));
    }
    if !parsed.contains(&FeatureId::ReceiveMessage) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(RegistrationError {
                error: "Missing required feature: ReceiveMessage".into(),
            }),
        ));
    }

    Ok(parsed)
}

/// POST /model/switch
async fn handle_model_switch(
    State(state): State<AppState>,
    Json(req): Json<ModelSwitchRequest>,
) -> Result<Json<ModelSwitchResponse>, (StatusCode, Json<ModelSwitchError>)> {
    let model_config = state.config.models.get(&req.model).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ModelSwitchError {
                error: format!("Unknown model: {}", req.model),
            }),
        )
    })?;

    // Update registry
    {
        let mut reg = state.registry.write().await;
        if !reg.update_worker_model(&req.dyad, &req.side, req.model.clone()) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ModelSwitchError {
                    error: "Worker not found".into(),
                }),
            ));
        }
    }

    // Push registry
    push_registry_to_all(&state).await;

    Ok(Json(ModelSwitchResponse::from(model_config)))
}

/// POST /switch_roles
async fn handle_switch_roles(
    State(state): State<AppState>,
    Json(req): Json<SwitchRolesRequest>,
) -> Result<Json<SwitchRolesResponse>, (StatusCode, Json<SwitchRolesError>)> {
    // Acquire dyad lock
    {
        let mut locks = state.dyad_locks.write().await;
        if *locks.get(&req.dyad).unwrap_or(&false) {
            return Err((
                StatusCode::CONFLICT,
                Json(SwitchRolesError {
                    error: "switch_in_progress".into(),
                    message: Some("Another switch is already in progress".into()),
                }),
            ));
        }
        locks.insert(req.dyad.clone(), true);
    }

    // Get both worker endpoints
    let (initiator_endpoint, partner_endpoint) = {
        let reg = state.registry.read().await;
        let initiator = reg.get_worker_endpoint(&req.dyad, &req.side);
        let partner = reg.get_partner_endpoint(&req.dyad, &req.side);
        (initiator, partner)
    };

    let partner_endpoint = match partner_endpoint {
        Some(ep) => ep,
        None => {
            release_lock(&state, &req.dyad).await;
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(SwitchRolesError {
                    error: "partner_unreachable".into(),
                    message: None,
                }),
            ));
        }
    };

    let initiator_endpoint = match initiator_endpoint {
        Some(ep) => ep,
        None => {
            release_lock(&state, &req.dyad).await;
            return Err((
                StatusCode::NOT_FOUND,
                Json(SwitchRolesError {
                    error: "initiator_not_found".into(),
                    message: None,
                }),
            ));
        }
    };

    // Phase 1: Prepare both workers
    let prepare_result = prepare_both(
        &state.client,
        &initiator_endpoint,
        &partner_endpoint,
    ).await;

    if !prepare_result {
        release_lock(&state, &req.dyad).await;
        return Err((
            StatusCode::CONFLICT,
            Json(SwitchRolesError {
                error: "partner_busy".into(),
                message: Some("Partner worker is mid-operation".into()),
            }),
        ));
    }

    // Phase 2: Commit both workers
    let commit_result = commit_both(
        &state.client,
        &initiator_endpoint,
        &partner_endpoint,
    ).await;

    if !commit_result {
        release_lock(&state, &req.dyad).await;
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SwitchRolesError {
                error: "commit_failed".into(),
                message: Some("Failed to commit switch".into()),
            }),
        ));
    }

    // Update registry with swapped batons
    let (your_new_baton, partner_new_baton) = {
        let mut reg = state.registry.write().await;
        let partner_side = match req.side {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        };

        // Swap batons
        reg.update_worker_baton(&req.dyad, &req.side, Baton::Spectator);
        reg.update_worker_baton(&req.dyad, &partner_side, Baton::Actor);

        // The initiator becomes spectator, partner becomes actor
        // (Assuming initiator was actor requesting the switch)
        (Baton::Spectator, Baton::Actor)
    };

    // Push registry
    push_registry_to_all(&state).await;

    // Release lock
    release_lock(&state, &req.dyad).await;

    Ok(Json(SwitchRolesResponse {
        switched: true,
        your_new_baton,
        partner_new_baton,
    }))
}

async fn release_lock(state: &AppState, dyad: &str) {
    let mut locks = state.dyad_locks.write().await;
    locks.remove(dyad);
}

async fn prepare_both(client: &reqwest::Client, initiator: &str, partner: &str) -> bool {
    let prep_body = serde_json::json!({"phase": "prepare"});

    let init_result = client
        .post(format!("{}/prepare_switch", initiator))
        .json(&prep_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    let partner_result = client
        .post(format!("{}/prepare_switch", partner))
        .json(&prep_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    matches!(
        (init_result, partner_result),
        (Ok(r1), Ok(r2)) if r1.status().is_success() && r2.status().is_success()
    )
}

async fn commit_both(client: &reqwest::Client, initiator: &str, partner: &str) -> bool {
    let commit_body = serde_json::json!({"phase": "commit"});

    let init_result = client
        .post(format!("{}/commit_switch", initiator))
        .json(&commit_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    let partner_result = client
        .post(format!("{}/commit_switch", partner))
        .json(&commit_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    matches!(
        (init_result, partner_result),
        (Ok(r1), Ok(r2)) if r1.status().is_success() && r2.status().is_success()
    )
}

/// POST /worker/output
async fn handle_worker_output(
    State(state): State<AppState>,
    Json(output): Json<WorkerOutput>,
) -> Json<OutputAck> {
    tracing::info!(
        "Worker output: dyad={}, side={:?}, status={:?}",
        output.dyad,
        output.side,
        output.status
    );

    // Process the output
    let action = {
        let mut mgr = state.respawn.write().await;
        mgr.process_output(&output)
    };

    tracing::info!("Respawn action: {:?}", action);

    // Remove from registry
    {
        let mut reg = state.registry.write().await;
        reg.remove_worker(&output.dyad, &output.side);
    }

    // Push registry
    push_registry_to_all(&state).await;

    // Trigger respawn based on action
    match action {
        RespawnAction::ImmediateWithSleep
        | RespawnAction::ImmediateWithSummary
        | RespawnAction::ImmediateFromJSONL => {
            // Spawn worker immediately
            let mut sup = state.supervisor.write().await;
            if let Err(e) = sup
                .spawn_worker(&state.orchestrator_url, &output.dyad, output.side.clone())
                .await
            {
                tracing::error!("Failed to respawn worker: {}", e);
            }
        }
        RespawnAction::WaitThenRespawn { minutes } => {
            // Respawn manager already stored the wake time
            tracing::info!(
                "Worker will respawn in {} minutes",
                minutes
            );
        }
    }

    Json(OutputAck { acknowledged: true })
}

/// GET /registry
async fn handle_get_registry(State(state): State<AppState>) -> Json<Registry> {
    let reg = state.registry.read().await;
    Json(reg.build_registry())
}

/// GET /health
async fn handle_health(State(state): State<AppState>) -> Json<HealthResponse> {
    let reg = state.registry.read().await;
    Json(HealthResponse {
        status: "ok".into(),
        workers: reg.worker_count(),
        adapters: reg.adapter_count(),
        embed_services: reg.embed_count(),
    })
}

async fn push_registry_to_all(state: &AppState) {
    let (registry, endpoints) = {
        let reg = state.registry.read().await;
        (reg.build_registry(), reg.all_endpoints())
    };
    push_registry(&state.client, &registry, &endpoints).await;
}

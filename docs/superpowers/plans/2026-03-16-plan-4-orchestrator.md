# Plan 4: Minimal Orchestrator Implementation

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a minimal orchestrator that tracks agent health via heartbeats and serves a static model registry.

**Architecture:** New `river-orchestrator` crate with in-memory state. Gateways send heartbeats every 30 seconds. Orchestrator exposes HTTP API for agent status and model list. Graceful degradation - gateways work without orchestrator.

**Tech Stack:** Rust, axum, tokio, clap, serde, chrono (same patterns as river-gateway)

**Spec:** `docs/superpowers/specs/2026-03-16-orchestrator-minimal-design.md`

---

## File Structure

```
crates/river-orchestrator/
├── Cargo.toml
└── src/
    ├── lib.rs              # Public exports
    ├── main.rs             # CLI entry point
    ├── config.rs           # Configuration types
    ├── state.rs            # Shared application state
    ├── agents.rs           # Agent registry and health logic
    ├── models.rs           # Static model registry
    └── api/
        ├── mod.rs          # Router setup
        └── routes.rs       # HTTP handlers
```

**Gateway modifications:**
- `crates/river-gateway/src/main.rs` - Add `--orchestrator-url` flag
- `crates/river-gateway/src/server.rs` - Add heartbeat task
- `crates/river-gateway/src/heartbeat.rs` - NEW: Heartbeat client

---

## Chunk 1: Orchestrator Crate Setup

### Task 1: Create Crate Structure

**Files:**
- Create: `crates/river-orchestrator/Cargo.toml`
- Create: `crates/river-orchestrator/src/lib.rs`
- Create: `crates/river-orchestrator/src/main.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "river-orchestrator"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "River Engine orchestrator - coordination service"

[[bin]]
name = "river-orchestrator"
path = "src/main.rs"

[dependencies]
river-core = { path = "../river-core" }
tokio.workspace = true
axum.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
clap.workspace = true
chrono.workspace = true
anyhow.workspace = true

[dev-dependencies]
tower = { workspace = true, features = ["util"] }
```

- [ ] **Step 2: Create lib.rs**

```rust
//! River Engine Orchestrator
//!
//! Coordination service for River Engine agents.

pub mod agents;
pub mod api;
pub mod config;
pub mod models;
pub mod state;

pub use config::{ModelConfig, OrchestratorConfig};
pub use state::OrchestratorState;
```

- [ ] **Step 3: Create placeholder main.rs**

```rust
fn main() {
    println!("river-orchestrator placeholder");
}
```

- [ ] **Step 4: Create placeholder modules**

Create empty files with module declarations:

`src/config.rs`:
```rust
//! Configuration types
```

`src/state.rs`:
```rust
//! Shared application state
```

`src/agents.rs`:
```rust
//! Agent registry and health tracking
```

`src/models.rs`:
```rust
//! Static model registry
```

`src/api/mod.rs`:
```rust
//! HTTP API
pub mod routes;
```

`src/api/routes.rs`:
```rust
//! HTTP route handlers
```

- [ ] **Step 5: Verify crate compiles**

Run: `cargo build -p river-orchestrator`
Expected: Successful compilation

- [ ] **Step 6: Commit**

```bash
git add crates/river-orchestrator/
git commit -m "feat(orchestrator): create crate structure"
```

---

### Task 2: Configuration Types

**Files:**
- Modify: `crates/river-orchestrator/src/config.rs`

- [ ] **Step 1: Write failing test for config defaults**

```rust
//! Configuration types

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_config_defaults() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.port, 5000);
        assert_eq!(config.health_threshold_seconds, 120);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-orchestrator test_orchestrator_config_defaults`
Expected: FAIL - `OrchestratorConfig` not found

- [ ] **Step 3: Implement OrchestratorConfig**

```rust
//! Configuration types

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Orchestrator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,

    /// Seconds before agent marked unhealthy
    #[serde(default = "default_health_threshold")]
    pub health_threshold_seconds: u64,

    /// Path to models config file (optional)
    pub models_config: Option<PathBuf>,
}

fn default_port() -> u16 {
    5000
}

fn default_health_threshold() -> u64 {
    120
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            health_threshold_seconds: default_health_threshold(),
            models_config: None,
        }
    }
}

/// Model configuration entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub provider: String,
}

/// Models configuration file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsFile {
    pub models: Vec<ModelConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_config_defaults() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.port, 5000);
        assert_eq!(config.health_threshold_seconds, 120);
    }

    #[test]
    fn test_models_file_deserialize() {
        let json = r#"{"models": [{"name": "qwen3-32b", "provider": "local"}]}"#;
        let file: ModelsFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.models.len(), 1);
        assert_eq!(file.models[0].name, "qwen3-32b");
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-orchestrator/src/config.rs
git commit -m "feat(orchestrator): add configuration types"
```

---

### Task 3: Agent Registry and Health Logic

**Files:**
- Modify: `crates/river-orchestrator/src/agents.rs`

- [ ] **Step 1: Write failing test for AgentInfo health check**

```rust
//! Agent registry and health tracking

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_is_healthy_within_threshold() {
        let agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        // Just created, should be healthy
        assert!(agent.is_healthy(Duration::from_secs(120)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-orchestrator test_agent_is_healthy`
Expected: FAIL - `AgentInfo` not found

- [ ] **Step 3: Implement AgentInfo**

```rust
//! Agent registry and health tracking

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::{Duration, Instant};

/// Information about a registered agent
pub struct AgentInfo {
    pub name: String,
    pub gateway_url: String,
    pub last_heartbeat: Instant,
    pub registered_at: DateTime<Utc>,
}

impl AgentInfo {
    /// Create new agent info (registers current time as first heartbeat)
    pub fn new(name: String, gateway_url: String) -> Self {
        Self {
            name,
            gateway_url,
            last_heartbeat: Instant::now(),
            registered_at: Utc::now(),
        }
    }

    /// Update heartbeat timestamp
    pub fn heartbeat(&mut self) {
        self.last_heartbeat = Instant::now();
    }

    /// Update gateway URL
    pub fn update_url(&mut self, url: String) {
        self.gateway_url = url;
    }

    /// Check if agent is healthy (heartbeat within threshold)
    pub fn is_healthy(&self, threshold: Duration) -> bool {
        self.last_heartbeat.elapsed() < threshold
    }

    /// Get seconds since last heartbeat
    pub fn seconds_since_heartbeat(&self) -> u64 {
        self.last_heartbeat.elapsed().as_secs()
    }
}

/// Agent status for API response
#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub name: String,
    pub gateway_url: String,
    pub healthy: bool,
    pub last_heartbeat_seconds_ago: u64,
    pub registered_at: DateTime<Utc>,
}

impl AgentStatus {
    pub fn from_agent(agent: &AgentInfo, threshold: Duration) -> Self {
        Self {
            name: agent.name.clone(),
            gateway_url: agent.gateway_url.clone(),
            healthy: agent.is_healthy(threshold),
            last_heartbeat_seconds_ago: agent.seconds_since_heartbeat(),
            registered_at: agent.registered_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_is_healthy_within_threshold() {
        let agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        // Just created, should be healthy
        assert!(agent.is_healthy(Duration::from_secs(120)));
    }

    #[test]
    fn test_agent_heartbeat_updates_timestamp() {
        let mut agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        let before = agent.last_heartbeat;
        std::thread::sleep(Duration::from_millis(10));
        agent.heartbeat();
        assert!(agent.last_heartbeat > before);
    }

    #[test]
    fn test_agent_status_from_agent() {
        let agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        let status = AgentStatus::from_agent(&agent, Duration::from_secs(120));
        assert_eq!(status.name, "test");
        assert!(status.healthy);
        assert!(status.last_heartbeat_seconds_ago < 1);
    }

    #[test]
    fn test_agent_update_url() {
        let mut agent = AgentInfo::new("test".to_string(), "http://localhost:3000".to_string());
        agent.update_url("http://localhost:4000".to_string());
        assert_eq!(agent.gateway_url, "http://localhost:4000");
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-orchestrator/src/agents.rs
git commit -m "feat(orchestrator): add agent registry and health logic"
```

---

### Task 4: Model Registry

**Files:**
- Modify: `crates/river-orchestrator/src/models.rs`

- [ ] **Step 1: Write failing test for ModelInfo**

```rust
//! Static model registry

use serde::Serialize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_creation() {
        let model = ModelInfo::new("qwen3-32b".to_string(), ModelProvider::Local);
        assert_eq!(model.name, "qwen3-32b");
        assert!(matches!(model.provider, ModelProvider::Local));
        assert!(matches!(model.status, ModelStatus::Available));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-orchestrator test_model_info_creation`
Expected: FAIL - `ModelInfo` not found

- [ ] **Step 3: Implement model types**

```rust
//! Static model registry

use serde::Serialize;

/// Model provider type
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModelProvider {
    Local,
    LiteLLM,
}

impl From<&str> for ModelProvider {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "local" => ModelProvider::Local,
            "litellm" => ModelProvider::LiteLLM,
            _ => ModelProvider::Local, // default
        }
    }
}

/// Model availability status
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModelStatus {
    Available,
    Unavailable,
}

/// Information about a configured model
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub provider: ModelProvider,
    pub status: ModelStatus,
}

impl ModelInfo {
    pub fn new(name: String, provider: ModelProvider) -> Self {
        Self {
            name,
            provider,
            status: ModelStatus::Available,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_creation() {
        let model = ModelInfo::new("qwen3-32b".to_string(), ModelProvider::Local);
        assert_eq!(model.name, "qwen3-32b");
        assert!(matches!(model.provider, ModelProvider::Local));
        assert!(matches!(model.status, ModelStatus::Available));
    }

    #[test]
    fn test_model_provider_from_str() {
        assert!(matches!(ModelProvider::from("local"), ModelProvider::Local));
        assert!(matches!(ModelProvider::from("litellm"), ModelProvider::LiteLLM));
        assert!(matches!(ModelProvider::from("LOCAL"), ModelProvider::Local));
        assert!(matches!(ModelProvider::from("unknown"), ModelProvider::Local));
    }

    #[test]
    fn test_model_info_serialize() {
        let model = ModelInfo::new("test".to_string(), ModelProvider::LiteLLM);
        let json = serde_json::to_string(&model).unwrap();
        assert!(json.contains("\"provider\":\"litellm\""));
        assert!(json.contains("\"status\":\"available\""));
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-orchestrator/src/models.rs
git commit -m "feat(orchestrator): add model registry types"
```

---

### Task 5: Application State

**Files:**
- Modify: `crates/river-orchestrator/src/state.rs`

- [ ] **Step 1: Write failing test for state**

```rust
//! Shared application state

use crate::agents::AgentInfo;
use crate::config::OrchestratorConfig;
use crate::models::ModelInfo;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let config = OrchestratorConfig::default();
        let state = OrchestratorState::new(config, vec![]);
        assert_eq!(state.config.port, 5000);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-orchestrator test_state_creation`
Expected: FAIL - `OrchestratorState` not found

- [ ] **Step 3: Implement OrchestratorState**

```rust
//! Shared application state

use crate::agents::{AgentInfo, AgentStatus};
use crate::config::OrchestratorConfig;
use crate::models::ModelInfo;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;

/// Shared orchestrator state
pub struct OrchestratorState {
    pub agents: RwLock<HashMap<String, AgentInfo>>,
    pub models: Vec<ModelInfo>,
    pub config: OrchestratorConfig,
}

impl OrchestratorState {
    /// Create new orchestrator state
    pub fn new(config: OrchestratorConfig, models: Vec<ModelInfo>) -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            models,
            config,
        }
    }

    /// Get health threshold as Duration
    pub fn health_threshold(&self) -> Duration {
        Duration::from_secs(self.config.health_threshold_seconds)
    }

    /// Register or update agent heartbeat
    pub async fn heartbeat(&self, name: String, gateway_url: String) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&name) {
            agent.heartbeat();
            if agent.gateway_url != gateway_url {
                agent.update_url(gateway_url);
            }
        } else {
            agents.insert(name.clone(), AgentInfo::new(name, gateway_url));
        }
    }

    /// Get all agent statuses
    pub async fn agent_statuses(&self) -> Vec<AgentStatus> {
        let agents = self.agents.read().await;
        let threshold = self.health_threshold();
        agents
            .values()
            .map(|a| AgentStatus::from_agent(a, threshold))
            .collect()
    }

    /// Get count of registered agents
    pub async fn agent_count(&self) -> usize {
        self.agents.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let config = OrchestratorConfig::default();
        let state = OrchestratorState::new(config, vec![]);
        assert_eq!(state.config.port, 5000);
    }

    #[tokio::test]
    async fn test_state_heartbeat_creates_agent() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;
        assert_eq!(state.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_state_heartbeat_updates_existing() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;
        state.heartbeat("test".to_string(), "http://localhost:4000".to_string()).await;

        let statuses = state.agent_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].gateway_url, "http://localhost:4000");
    }

    #[tokio::test]
    async fn test_state_agent_statuses() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("agent1".to_string(), "http://localhost:3000".to_string()).await;
        state.heartbeat("agent2".to_string(), "http://localhost:3001".to_string()).await;

        let statuses = state.agent_statuses().await;
        assert_eq!(statuses.len(), 2);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-orchestrator/src/state.rs
git commit -m "feat(orchestrator): add application state"
```

---

## Chunk 2: HTTP API

### Task 6: API Routes

**Files:**
- Modify: `crates/river-orchestrator/src/api/routes.rs`
- Modify: `crates/river-orchestrator/src/api/mod.rs`

- [ ] **Step 1: Write failing test for health endpoint**

`src/api/routes.rs`:
```rust
//! HTTP route handlers

use crate::state::OrchestratorState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OrchestratorConfig;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state() -> Arc<OrchestratorState> {
        Arc::new(OrchestratorState::new(OrchestratorConfig::default(), vec![]))
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
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-orchestrator test_health_check`
Expected: FAIL - `create_router` not found

- [ ] **Step 3: Implement health endpoint and router**

`src/api/routes.rs`:
```rust
//! HTTP route handlers

use crate::agents::AgentStatus;
use crate::models::ModelInfo;
use crate::state::OrchestratorState;
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
    use axum::http::Request;
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
```

- [ ] **Step 4: Update api/mod.rs**

`src/api/mod.rs`:
```rust
//! HTTP API

pub mod routes;

pub use routes::create_router;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-orchestrator/src/api/
git commit -m "feat(orchestrator): add HTTP API routes"
```

---

### Task 7: CLI and Server Startup

**Files:**
- Modify: `crates/river-orchestrator/src/main.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Implement main.rs with CLI**

```rust
use clap::Parser;
use river_orchestrator::{
    api::create_router,
    config::{ModelsFile, OrchestratorConfig},
    models::{ModelInfo, ModelProvider},
    OrchestratorState,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "river-orchestrator")]
#[command(about = "River Engine Orchestrator - Coordination Service")]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "5000")]
    port: u16,

    /// Health threshold in seconds
    #[arg(long, default_value = "120")]
    health_threshold: u64,

    /// Path to models config JSON file
    #[arg(long)]
    models_config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    tracing::info!("Starting River Orchestrator");
    tracing::info!("Port: {}", args.port);
    tracing::info!("Health threshold: {}s", args.health_threshold);

    // Load models from config file if provided
    let models = if let Some(path) = &args.models_config {
        tracing::info!("Loading models from {:?}", path);
        let content = std::fs::read_to_string(path)?;
        let file: ModelsFile = serde_json::from_str(&content)?;
        file.models
            .into_iter()
            .map(|m| ModelInfo::new(m.name, ModelProvider::from(m.provider.as_str())))
            .collect()
    } else {
        tracing::info!("No models config provided, starting with empty registry");
        vec![]
    };

    tracing::info!("Loaded {} models", models.len());

    let config = OrchestratorConfig {
        port: args.port,
        health_threshold_seconds: args.health_threshold,
        models_config: args.models_config,
    };

    let state = Arc::new(OrchestratorState::new(config, models));
    let app = create_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    tracing::info!("Orchestrator listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

- [ ] **Step 2: Update lib.rs exports**

```rust
//! River Engine Orchestrator
//!
//! Coordination service for River Engine agents.

pub mod agents;
pub mod api;
pub mod config;
pub mod models;
pub mod state;

pub use config::{ModelConfig, ModelsFile, OrchestratorConfig};
pub use state::OrchestratorState;
```

- [ ] **Step 3: Verify build and run**

Run: `cargo build -p river-orchestrator`
Expected: Successful build

Run: `cargo run -p river-orchestrator -- --help`
Expected: Shows help with port, health-threshold, models-config options

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/main.rs crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): add CLI and server startup"
```

---

## Chunk 3: Gateway Integration

### Task 8: Gateway Heartbeat Client

**Files:**
- Create: `crates/river-gateway/src/heartbeat.rs`
- Modify: `crates/river-gateway/src/lib.rs`

**Note:** `reqwest` is already a workspace dependency for `river-gateway` with JSON support.

- [ ] **Step 1: Write failing test for heartbeat client**

`src/heartbeat.rs`:
```rust
//! Heartbeat client for orchestrator communication

use river_core::RiverResult;
use serde::Serialize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_request_serialize() {
        let req = HeartbeatRequest {
            agent: "test".to_string(),
            gateway_url: "http://localhost:3000".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"agent\":\"test\""));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-gateway test_heartbeat_request`
Expected: FAIL - `HeartbeatRequest` not found

- [ ] **Step 3: Implement heartbeat client**

```rust
//! Heartbeat client for orchestrator communication

use river_core::RiverResult;
use serde::Serialize;
use std::time::Duration;

/// Heartbeat request payload
#[derive(Serialize)]
pub struct HeartbeatRequest {
    pub agent: String,
    pub gateway_url: String,
}

/// Heartbeat client for sending heartbeats to orchestrator
#[derive(Clone)]
pub struct HeartbeatClient {
    client: reqwest::Client,
    orchestrator_url: String,
    agent_name: String,
    gateway_url: String,
}

impl HeartbeatClient {
    /// Create new heartbeat client
    pub fn new(orchestrator_url: String, agent_name: String, gateway_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            orchestrator_url,
            agent_name,
            gateway_url,
        }
    }

    /// Send heartbeat to orchestrator
    pub async fn send_heartbeat(&self) -> RiverResult<()> {
        let url = format!("{}/heartbeat", self.orchestrator_url);
        let req = HeartbeatRequest {
            agent: self.agent_name.clone(),
            gateway_url: self.gateway_url.clone(),
        };

        match self.client.post(&url).json(&req).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    tracing::debug!("Heartbeat sent successfully");
                    Ok(())
                } else {
                    tracing::warn!("Heartbeat failed with status: {}", response.status());
                    Ok(()) // Don't error, graceful degradation
                }
            }
            Err(e) => {
                tracing::warn!("Failed to send heartbeat: {}", e);
                Ok(()) // Don't error, graceful degradation
            }
        }
    }

    /// Start heartbeat loop (runs forever, call in background task)
    pub async fn run_loop(&self, interval_seconds: u64) {
        let interval = Duration::from_secs(interval_seconds);
        loop {
            let _ = self.send_heartbeat().await;
            tokio::time::sleep(interval).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_request_serialize() {
        let req = HeartbeatRequest {
            agent: "test".to_string(),
            gateway_url: "http://localhost:3000".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"agent\":\"test\""));
    }

    #[test]
    fn test_heartbeat_client_creation() {
        let client = HeartbeatClient::new(
            "http://localhost:5000".to_string(),
            "test-agent".to_string(),
            "http://localhost:3000".to_string(),
        );
        assert_eq!(client.orchestrator_url, "http://localhost:5000");
        assert_eq!(client.agent_name, "test-agent");
    }
}
```

- [ ] **Step 4: Add to lib.rs**

Add to `crates/river-gateway/src/lib.rs`:
```rust
pub mod heartbeat;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-gateway heartbeat`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/heartbeat.rs crates/river-gateway/src/lib.rs
git commit -m "feat(gateway): add heartbeat client"
```

---

### Task 9: Gateway CLI Flag and Server Integration

**Files:**
- Modify: `crates/river-gateway/src/main.rs`
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Add orchestrator-url CLI flag**

In `src/main.rs`, add to Args struct:
```rust
    /// Orchestrator URL (enables heartbeats)
    #[arg(long)]
    orchestrator_url: Option<String>,
```

Update ServerConfig creation to include:
```rust
    let config = ServerConfig {
        workspace: args.workspace,
        data_dir: args.data_dir,
        port: args.port,
        agent_name: args.agent_name,
        model_url: args.model_url,
        model_name: args.model_name,
        embedding_url: args.embedding_url,
        redis_url: args.redis_url,
        orchestrator_url: args.orchestrator_url,
    };
```

- [ ] **Step 2: Update ServerConfig in server.rs**

Add field to `ServerConfig`:
```rust
pub struct ServerConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub agent_name: String,
    pub model_url: Option<String>,
    pub model_name: Option<String>,
    pub embedding_url: Option<String>,
    pub redis_url: Option<String>,
    pub orchestrator_url: Option<String>,
}
```

- [ ] **Step 3: Start heartbeat task in server.rs**

Add at end of `run()` function, before `axum::serve`:
```rust
    // Start heartbeat task if orchestrator configured
    if let Some(orchestrator_url) = &config.orchestrator_url {
        let gateway_url = format!("http://127.0.0.1:{}", config.port);
        let heartbeat_client = crate::heartbeat::HeartbeatClient::new(
            orchestrator_url.clone(),
            config.agent_name.clone(),
            gateway_url,
        );

        tokio::spawn(async move {
            heartbeat_client.run_loop(30).await;
        });

        tracing::info!("Started heartbeat to orchestrator: {}", orchestrator_url);
    }
```

Add import at top:
```rust
use crate::heartbeat::HeartbeatClient;
```

- [ ] **Step 4: Verify build**

Run: `cargo build -p river-gateway`
Expected: Successful build

Run: `cargo run -p river-gateway -- --help`
Expected: Shows `--orchestrator-url` option

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/main.rs crates/river-gateway/src/server.rs
git commit -m "feat(gateway): integrate heartbeat with orchestrator"
```

---

## Chunk 4: Final Verification

### Task 10: Integration Test

**Files:**
- Create: `crates/river-orchestrator/tests/integration.rs`

- [ ] **Step 1: Write integration test**

```rust
//! Integration tests for orchestrator

use river_orchestrator::{
    api::create_router,
    config::OrchestratorConfig,
    models::{ModelInfo, ModelProvider},
    OrchestratorState,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn test_full_workflow() {
    // Create orchestrator with a model
    let models = vec![
        ModelInfo::new("test-model".to_string(), ModelProvider::Local),
    ];
    let state = Arc::new(OrchestratorState::new(
        OrchestratorConfig::default(),
        models,
    ));
    let app = create_router(state.clone());

    // 1. Check health (no agents yet)
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
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
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run integration test**

Run: `cargo test -p river-orchestrator --test integration`
Expected: Test passes

- [ ] **Step 3: Commit**

```bash
git add crates/river-orchestrator/tests/
git commit -m "test(orchestrator): add integration test"
```

---

### Task 11: Run Full Test Suite

- [ ] **Step 1: Run all tests**

Run: `cargo test`
Expected: All tests pass (river-core, river-gateway, river-orchestrator)

- [ ] **Step 2: Build release binaries**

Run: `cargo build --release`
Expected: Successful build

- [ ] **Step 3: Verify binaries**

Run: `./target/release/river-orchestrator --help`
Expected: Shows CLI help

Run: `./target/release/river-gateway --help`
Expected: Shows CLI help with `--orchestrator-url`

- [ ] **Step 4: Document test count**

Record total test count for STATUS.md update.

---

### Task 12: Update STATUS.md

**Files:**
- Modify: `docs/superpowers/STATUS.md`

- [ ] **Step 1: Add Plan 4 completion to STATUS.md**

Add under "## Completed":

```markdown
### Plan 4: Minimal Orchestrator ✅
- `river-orchestrator` crate with:
  - Agent health monitoring via heartbeats
  - Agent status API (`/agents/status`)
  - Static model registry (`/models/available`)
  - Health endpoint (`/health`)
  - CLI: `river-orchestrator --port --health-threshold --models-config`
- Gateway integration:
  - `--orchestrator-url` flag for heartbeat configuration
  - Background heartbeat task (30 second interval)
  - Graceful degradation (works without orchestrator)
- [Record actual test count from Task 11 Step 4]
```

Update "## Next Up" to show Plan 5: Discord Adapter.

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/STATUS.md
git commit -m "docs: complete Plan 4 Minimal Orchestrator implementation"
```

---

## Summary

**Tasks:** 12 total
- Tasks 1-5: Orchestrator crate setup (config, agents, models, state)
- Tasks 6-7: HTTP API and CLI
- Tasks 8-9: Gateway heartbeat integration
- Tasks 10-12: Testing and documentation

**New files:**
- `crates/river-orchestrator/` (entire crate)
- `crates/river-gateway/src/heartbeat.rs`

**Modified files:**
- `crates/river-gateway/src/main.rs`
- `crates/river-gateway/src/server.rs`
- `crates/river-gateway/src/lib.rs`
- `docs/superpowers/STATUS.md`

**Binaries:**
- `river-orchestrator` - Coordination service
- `river-gateway` - Now with `--orchestrator-url` flag

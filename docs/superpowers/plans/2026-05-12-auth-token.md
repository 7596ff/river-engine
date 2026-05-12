# Auth Token Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bearer token authentication to every non-health endpoint across all river-engine services.

**Architecture:** A shared `auth` module in `river-core` provides token loading and validation. Each service reads `RIVER_AUTH_TOKEN` from the environment (loaded from `.env` via `dotenvy`), stores it in state, and validates it on every non-health endpoint. Outbound HTTP calls use a `reqwest::Client` built with a default `Authorization` header.

**Tech Stack:** Rust, dotenvy, reqwest (default_headers), river-core

---

### Task 1: Shared Auth Module in river-core

**Files:**
- Create: `crates/river-core/src/auth.rs`
- Modify: `crates/river-core/src/lib.rs`

- [ ] **Step 1: Write failing tests**

```rust
// crates/river-core/src/auth.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_bearer_valid() {
        assert!(validate_bearer("Bearer my-secret-token", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_wrong_token() {
        assert!(!validate_bearer("Bearer wrong-token", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_missing_prefix() {
        assert!(!validate_bearer("my-secret-token", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_empty_header() {
        assert!(!validate_bearer("", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_bearer_only() {
        assert!(!validate_bearer("Bearer ", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_case_sensitive_prefix() {
        assert!(!validate_bearer("bearer my-secret-token", "my-secret-token"));
    }

    #[test]
    fn test_require_auth_token_from_env() {
        std::env::set_var("RIVER_AUTH_TOKEN", "test-token-123");
        let result = require_auth_token();
        assert_eq!(result.unwrap(), "test-token-123");
        std::env::remove_var("RIVER_AUTH_TOKEN");
    }

    #[test]
    fn test_require_auth_token_missing() {
        std::env::remove_var("RIVER_AUTH_TOKEN");
        let result = require_auth_token();
        assert!(result.is_err());
    }

    #[test]
    fn test_require_auth_token_empty() {
        std::env::set_var("RIVER_AUTH_TOKEN", "");
        let result = require_auth_token();
        assert!(result.is_err());
        std::env::remove_var("RIVER_AUTH_TOKEN");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-core -- auth`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement auth module**

```rust
//! Shared authentication — token loading and validation.
//!
//! Every river-engine service reads RIVER_AUTH_TOKEN from the environment
//! and validates it on non-health HTTP endpoints.

use crate::RiverError;

/// Read RIVER_AUTH_TOKEN from the environment.
/// Returns Err if missing or empty.
pub fn require_auth_token() -> Result<String, RiverError> {
    match std::env::var("RIVER_AUTH_TOKEN") {
        Ok(token) if !token.is_empty() => Ok(token),
        Ok(_) => Err(RiverError::config(
            "RIVER_AUTH_TOKEN is set but empty — set a token in .env or the environment"
        )),
        Err(_) => Err(RiverError::config(
            "RIVER_AUTH_TOKEN not set — create a .env file or set the environment variable"
        )),
    }
}

/// Validate a bearer token from an Authorization header value.
/// `auth_header` is the raw value of the Authorization header.
/// Returns true if it matches "Bearer <expected>".
pub fn validate_bearer(auth_header: &str, expected: &str) -> bool {
    match auth_header.strip_prefix("Bearer ") {
        Some(token) => !token.is_empty() && token == expected,
        None => false,
    }
}

/// Build a reqwest::Client with a default Authorization header.
/// Use this for all outbound HTTP calls that need auth.
pub fn build_authed_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    let value = format!("Bearer {}", token);
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&value)
            .expect("auth token contains invalid header characters"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
}
```

- [ ] **Step 4: Add `pub mod auth;` and re-exports to lib.rs**

Add after `pub mod types;`:

```rust
pub mod auth;
```

Add to re-exports:

```rust
pub use auth::{require_auth_token, validate_bearer, build_authed_client};
```

- [ ] **Step 5: Add reqwest dependency to river-core Cargo.toml**

Add to `[dependencies]`:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

- [ ] **Step 6: Check that RiverError has a config variant**

Check `crates/river-core/src/error.rs` for a `config` constructor. If missing, add:

```rust
pub fn config(msg: impl Into<String>) -> Self {
    Self::Config(msg.into())
}
```

And the variant:

```rust
#[error("Configuration error: {0}")]
Config(String),
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p river-core -- auth`
Expected: PASS (9 tests)

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat(core): shared auth module — require_auth_token, validate_bearer, build_authed_client"
```

---

### Task 2: .env Setup

**Files:**
- Create: `.env.example`
- Modify: `.gitignore`

- [ ] **Step 1: Create .env.example**

```
RIVER_AUTH_TOKEN=your-secret-token-here
```

- [ ] **Step 2: Add .env to .gitignore**

```
.env
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: add .env.example and gitignore .env"
```

---

### Task 3: Gateway Auth — Env Loading and Non-Optional Token

**Files:**
- Modify: `crates/river-gateway/Cargo.toml`
- Modify: `crates/river-gateway/src/main.rs`
- Modify: `crates/river-gateway/src/server.rs`
- Modify: `crates/river-gateway/src/state.rs`
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Add dotenvy to gateway Cargo.toml**

```toml
dotenvy = "0.15"
```

- [ ] **Step 2: Add dotenvy::dotenv().ok() to main.rs**

At the top of `main()`, before anything else:

```rust
dotenvy::dotenv().ok();
```

- [ ] **Step 3: Update server.rs token loading**

Replace the existing auth token loading block (lines 237-249) with:

```rust
    // Load auth token: env var first, then --auth-token-file fallback
    let auth_token = match river_core::require_auth_token() {
        Ok(token) => {
            tracing::info!("Auth token loaded from RIVER_AUTH_TOKEN");
            token
        }
        Err(_) => {
            // Fallback to file
            if let Some(ref token_file) = config.auth_token_file {
                let token = tokio::fs::read_to_string(token_file)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read auth token file: {}", e))?
                    .trim()
                    .to_string();
                if token.is_empty() {
                    return Err(anyhow::anyhow!("Auth token file is empty"));
                }
                tracing::info!("Auth token loaded from {:?}", token_file);
                token
            } else {
                return Err(anyhow::anyhow!(
                    "No auth token configured. Set RIVER_AUTH_TOKEN in .env or pass --auth-token-file"
                ));
            }
        }
    };
```

- [ ] **Step 4: Change AppState.auth_token from Option<String> to String**

In `crates/river-gateway/src/state.rs`:

Change field:
```rust
    pub auth_token: String,
```

Change constructor parameter:
```rust
    auth_token: String,
```

- [ ] **Step 5: Update validate_auth in routes.rs**

Replace the existing `validate_auth` function:

```rust
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
```

- [ ] **Step 6: Update all call sites from `.as_deref()` to `.as_str()`**

Replace all instances of `state.auth_token.as_deref()` with `&state.auth_token`:

- `handle_incoming`: `validate_auth(&headers, &state.auth_token)`
- `handle_bystander`: `validate_auth(&headers, &state.auth_token)`
- `register_adapter`: `validate_auth(&headers, &state.auth_token)`

- [ ] **Step 7: Add auth to list_tools**

Change `list_tools` handler to accept headers and validate:

```rust
async fn list_tools(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::tools::ToolSchema>>, StatusCode> {
    if let Err(status) = validate_auth(&headers, &state.auth_token) {
        return Err(status);
    }
    let executor = state.tool_executor.read().await;
    Ok(Json(executor.schemas()))
}
```

- [ ] **Step 8: Update test helpers**

In `routes.rs`, update `test_state()` to pass a token instead of `None`:

```rust
fn test_state() -> Arc<AppState> {
    // ... existing code but change auth_token from None to:
    "test-token".to_string(),
}
```

Update `test_state_with_auth` to pass `String` instead of `Some(String)`:

```rust
fn test_state_with_auth(token: &str) -> Arc<AppState> {
    // ... existing code but change auth_token from Some(token.to_string()) to:
    token.to_string(),
}
```

Update any tests that call `test_state()` and expect no auth — they now need to include `Authorization: Bearer test-token` in their requests.

- [ ] **Step 9: Build the authed reqwest client in server.rs**

After loading the auth token, build the shared client:

```rust
    let authed_http_client = river_core::build_authed_client(&auth_token);
```

Store it in `AppState` — add field:

```rust
    pub http_client: reqwest::Client,
```

Pass it in the constructor.

- [ ] **Step 10: Run tests**

Run: `cargo test -p river-gateway`
Expected: PASS

- [ ] **Step 11: Commit**

```bash
git add -A && git commit -m "feat(gateway): auth from env, non-optional token, shared authed client"
```

---

### Task 4: Gateway Outbound Auth — Adapter Calls and Heartbeat

**Files:**
- Modify: `crates/river-gateway/src/tools/adapters.rs`
- Modify: `crates/river-gateway/src/tools/communication.rs`
- Modify: `crates/river-gateway/src/tools/sync.rs`
- Modify: `crates/river-gateway/src/heartbeat.rs`
- Modify: `crates/river-gateway/src/server.rs`

The gateway makes outbound HTTP calls to adapters (via `send_to_adapter`) and to the orchestrator (via `HeartbeatClient`). Both need the authed client.

- [ ] **Step 1: Update send_to_adapter callers to use state.http_client**

Find every place that passes a `reqwest::Client` to `send_to_adapter` or constructs one inline. Replace with `state.http_client` (the authed client from AppState).

In `communication.rs` — `SendMessageTool` and any other tool that holds its own `http_client: reqwest::Client`:

Replace `http_client: reqwest::Client::new()` in constructors with accepting the client from the outside. The tools are constructed in `server.rs` — pass `authed_http_client.clone()` there.

- [ ] **Step 2: Update HeartbeatClient to use the authed client**

Change `HeartbeatClient::new` to accept a `reqwest::Client` instead of building its own:

```rust
pub fn new(client: reqwest::Client, orchestrator_url: String, agent_name: String, gateway_url: String) -> Self {
    Self {
        client,
        orchestrator_url,
        agent_name,
        gateway_url,
    }
}
```

In `server.rs`, pass `authed_http_client.clone()` when constructing the heartbeat client.

- [ ] **Step 3: Update heartbeat error handling for 401**

In `send_heartbeat`, distinguish 401 from other errors:

```rust
Ok(response) => {
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        tracing::error!("Orchestrator rejected heartbeat — auth token mismatch");
        Ok(())
    } else if response.status().is_success() {
        tracing::debug!("Heartbeat sent successfully");
        Ok(())
    } else {
        tracing::warn!("Heartbeat failed with status: {}", response.status());
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(gateway): outbound auth on adapter calls and heartbeat"
```

---

### Task 5: Orchestrator Auth

**Files:**
- Modify: `crates/river-orchestrator/Cargo.toml`
- Modify: `crates/river-orchestrator/src/main.rs`
- Modify: `crates/river-orchestrator/src/state.rs`
- Modify: `crates/river-orchestrator/src/api/routes.rs`

- [ ] **Step 1: Add dotenvy to orchestrator Cargo.toml**

```toml
dotenvy = "0.15"
```

- [ ] **Step 2: Add dotenvy and token loading to main.rs**

At the top of `main()`:

```rust
dotenvy::dotenv().ok();
```

After parsing args, before constructing state:

```rust
let auth_token = river_core::require_auth_token()
    .map_err(|e| anyhow::anyhow!("{}", e))?;
```

Pass `auth_token` to `OrchestratorState::new`.

- [ ] **Step 3: Add auth_token to OrchestratorState**

In `crates/river-orchestrator/src/state.rs`:

```rust
pub struct OrchestratorState {
    pub agents: RwLock<HashMap<String, AgentInfo>>,
    pub config: OrchestratorConfig,
    pub local_models: RwLock<HashMap<String, LocalModelEntry>>,
    pub external_models: Vec<ExternalModel>,
    pub resource_tracker: Arc<ResourceTracker>,
    pub process_manager: Arc<ProcessManager>,
    pub auth_token: String,
}
```

Update `new()` to accept `auth_token: String` and store it.

- [ ] **Step 4: Add validate_auth function to routes.rs**

```rust
use axum::http::{HeaderMap, header::AUTHORIZATION};

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
```

- [ ] **Step 5: Add auth to all non-health handlers**

Every handler except `health_check` gains `headers: HeaderMap` and calls `validate_auth`. Example for `handle_heartbeat`:

```rust
async fn handle_heartbeat(
    State(state): State<Arc<OrchestratorState>>,
    headers: HeaderMap,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, StatusCode> {
    if let Err(status) = validate_auth(&headers, &state.auth_token) {
        return Err(status);
    }
    tracing::debug!("Heartbeat from {} at {}", req.agent, req.gateway_url);
    state.heartbeat(req.agent, req.gateway_url).await;
    Ok(Json(HeartbeatResponse { acknowledged: true }))
}
```

Apply the same pattern to: `agents_status`, `models_available`, `model_request`, `model_release`, `resources`. Each gains `headers: HeaderMap` parameter and the `validate_auth` check at the top. Each returns `Result<Json<...>, StatusCode>` instead of `Json<...>`.

- [ ] **Step 6: Update test helpers**

Update `test_state()` in `routes.rs` to pass an auth token:

```rust
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
```

Update tests that make requests to include the auth header:

```rust
.header("authorization", "Bearer test-token")
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p river-orchestrator`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat(orchestrator): auth on all non-health endpoints"
```

---

### Task 6: Discord Adapter Auth — Inbound and Outbound

**Files:**
- Modify: `crates/river-discord/Cargo.toml`
- Modify: `crates/river-discord/src/main.rs`
- Modify: `crates/river-discord/src/outbound.rs`
- Modify: `crates/river-discord/src/gateway.rs`
- Modify: `crates/river-discord/src/adapter.rs`

- [ ] **Step 1: Add dotenvy to discord Cargo.toml**

```toml
dotenvy = "0.15"
```

- [ ] **Step 2: Add dotenvy and token loading to main.rs**

At the top of `main()`:

```rust
dotenvy::dotenv().ok();
```

After parsing args:

```rust
let auth_token = river_core::require_auth_token()
    .map_err(|e| anyhow::anyhow!("{}", e))?;
```

- [ ] **Step 3: Add auth_token to Discord AppState**

In `outbound.rs`:

```rust
pub struct AppState {
    pub channels: Arc<ChannelState>,
    pub discord: Arc<RwLock<Option<DiscordSender>>>,
    pub discord_connected: std::sync::atomic::AtomicBool,
    pub gateway_reachable: std::sync::atomic::AtomicBool,
    pub port: u16,
    pub bot_id: RwLock<Option<String>>,
    pub auth_token: String,
}
```

Update `AppState::new` to accept `auth_token: String`.

- [ ] **Step 4: Add validate_auth to outbound.rs**

Same pattern as gateway and orchestrator:

```rust
use axum::http::{HeaderMap, header::AUTHORIZATION};

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
```

- [ ] **Step 5: Add auth to all non-health handlers**

Every handler except `health_check` gains `headers: HeaderMap` and calls `validate_auth`. Apply to: `handle_send`, `handle_typing`, `handle_read`, `list_channels`, `add_channel`, `remove_channel`, `history`, `capabilities`.

Each handler gains the `headers: HeaderMap` parameter and `validate_auth` check at the top. Return types change to `Result<..., StatusCode>` where they aren't already.

- [ ] **Step 6: Update GatewayClient to use authed client**

In `gateway.rs`, change `GatewayClient::new` to accept a `reqwest::Client`:

```rust
pub fn new(client: Client, base_url: String) -> Self {
    Self { client, base_url }
}
```

In `main.rs`, build the authed client and pass it:

```rust
let authed_client = river_core::build_authed_client(&auth_token);
let gateway_client = Arc::new(GatewayClient::new(authed_client, config.gateway_url.clone()));
```

- [ ] **Step 7: Update register_with_gateway to use authed client**

In `adapter.rs`, change to accept a `&reqwest::Client`:

```rust
pub async fn register_with_gateway(
    client: &reqwest::Client,
    gateway_url: &str,
    info: AdapterInfo,
) -> Result<(), String> {
    let url = format!("{}/adapters/register", gateway_url);
    let response: RegisterResponse = client
        .post(&url)
        .json(&RegisterRequest { adapter: info })
        .send()
        .await
        .map_err(|e| format!("registration request failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("failed to parse registration response: {}", e))?;

    if response.accepted {
        Ok(())
    } else {
        Err(response.error.unwrap_or_else(|| "registration rejected".into()))
    }
}
```

In `main.rs`, pass the authed client to `register_with_gateway`.

- [ ] **Step 8: Update main.rs constructor calls**

Pass `auth_token` to `AppState::new`. Pass `authed_client` to `GatewayClient::new` and `register_with_gateway`.

- [ ] **Step 9: Update test helpers**

Update `AppState::new` calls in tests to include an auth token. Update HTTP test requests to include the auth header.

- [ ] **Step 10: Run tests**

Run: `cargo test -p river-discord`
Expected: PASS

- [ ] **Step 11: Commit**

```bash
git add -A && git commit -m "feat(discord): auth on all endpoints, authed outbound calls"
```

---

### Task 7: Full Integration Test

**Files:**
- All crates

- [ ] **Step 1: Run full test suite**

```bash
cargo test --workspace
```

Expected: all tests pass across all crates.

- [ ] **Step 2: Create a .env for local testing**

```bash
echo "RIVER_AUTH_TOKEN=$(openssl rand -hex 32)" > .env
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: auth token implementation complete — all services authenticated"
```

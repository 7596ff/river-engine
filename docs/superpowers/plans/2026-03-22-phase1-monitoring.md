# Phase 1 Monitoring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add observability to river-gateway with rich health endpoint, structured JSON logging, and shared metrics.

**Architecture:** Shared `AgentMetrics` struct updated by `AgentLoop`, read by `/health` endpoint. JSON logs written to daily files via `tracing-subscriber` with file appender.

**Tech Stack:** tracing-subscriber (json, file appender), chrono, serde

---

## File Structure

**New files:**
- `crates/river-gateway/src/metrics.rs` — `AgentMetrics`, `LoopStateLabel`, `get_rss_bytes()`
- `crates/river-gateway/src/logging.rs` — `LogConfig`, `init_logging()`, daily rotation

**Modified files:**
- `crates/river-gateway/src/lib.rs` — export new modules
- `crates/river-gateway/src/main.rs` — CLI flags, logging init
- `crates/river-gateway/src/state.rs` — add `metrics: Arc<RwLock<AgentMetrics>>`
- `crates/river-gateway/src/server.rs` — create and pass metrics
- `crates/river-gateway/src/api/routes.rs` — rich health endpoint
- `crates/river-gateway/src/loop/mod.rs` — inject metrics, update at transitions
- `crates/river-gateway/src/tools/executor.rs` — increment counters

---

## Task 1: Add Dependencies

**Files:**
- Modify: `crates/river-gateway/Cargo.toml`

- [ ] **Step 1: Add tracing-appender dependency**

```toml
# Add after tracing-subscriber.workspace = true
tracing-appender = "0.2"
```

- [ ] **Step 2: Verify dependencies resolve**

Run: `cargo check -p river-gateway`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/Cargo.toml
git commit -m "chore(gateway): add tracing-appender for file logging"
```

---

## Task 2: Create AgentMetrics Struct

**Files:**
- Create: `crates/river-gateway/src/metrics.rs`
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Write the test for AgentMetrics defaults**

Create `crates/river-gateway/src/metrics.rs`:

```rust
//! Shared agent metrics for observability

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Observable loop state (no associated data, for serialization)
#[derive(Debug, Clone, Copy, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LoopStateLabel {
    #[default]
    Sleeping,
    Waking,
    Thinking,
    Acting,
    Settling,
}

/// Shared metrics updated by AgentLoop, read by health endpoint
#[derive(Debug, Clone, Serialize)]
pub struct AgentMetrics {
    // Identity
    pub agent_name: String,
    pub agent_birth: DateTime<Utc>,
    pub start_time: DateTime<Utc>,

    // Loop state
    pub loop_state: LoopStateLabel,
    pub last_wake: Option<DateTime<Utc>>,
    pub last_settle: Option<DateTime<Utc>>,
    pub turns_since_restart: u64,

    // Context
    pub context_tokens: u64,
    pub context_limit: u64,
    pub context_id: Option<String>,
    pub rotations_since_restart: u64,

    // Resources (updated on health check, not continuously)
    pub db_size_bytes: u64,
    pub rss_bytes: u64,

    // Counters (reset on restart)
    pub model_calls: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
}

impl AgentMetrics {
    pub fn new(agent_name: String, agent_birth: DateTime<Utc>, context_limit: u64) -> Self {
        Self {
            agent_name,
            agent_birth,
            start_time: Utc::now(),
            loop_state: LoopStateLabel::default(),
            last_wake: None,
            last_settle: None,
            turns_since_restart: 0,
            context_tokens: 0,
            context_limit,
            context_id: None,
            rotations_since_restart: 0,
            db_size_bytes: 0,
            rss_bytes: 0,
            model_calls: 0,
            tool_calls: 0,
            tool_errors: 0,
        }
    }

    /// Get context usage as percentage
    pub fn context_usage_percent(&self) -> f64 {
        if self.context_limit == 0 {
            0.0
        } else {
            (self.context_tokens as f64 / self.context_limit as f64) * 100.0
        }
    }
}

/// Get resident set size in bytes (Linux only)
pub fn get_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map(|pages| pages * 4096)
            .unwrap_or(0)
    }

    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_state_label_default() {
        assert_eq!(LoopStateLabel::default(), LoopStateLabel::Sleeping);
    }

    #[test]
    fn test_loop_state_label_serializes_snake_case() {
        let json = serde_json::to_string(&LoopStateLabel::Thinking).unwrap();
        assert_eq!(json, "\"thinking\"");
    }

    #[test]
    fn test_agent_metrics_new() {
        let birth = Utc::now();
        let metrics = AgentMetrics::new("test".to_string(), birth, 100000);

        assert_eq!(metrics.agent_name, "test");
        assert_eq!(metrics.context_limit, 100000);
        assert_eq!(metrics.turns_since_restart, 0);
        assert_eq!(metrics.loop_state, LoopStateLabel::Sleeping);
    }

    #[test]
    fn test_context_usage_percent() {
        let birth = Utc::now();
        let mut metrics = AgentMetrics::new("test".to_string(), birth, 100000);
        metrics.context_tokens = 25000;

        assert!((metrics.context_usage_percent() - 25.0).abs() < 0.01);
    }

    #[test]
    fn test_context_usage_percent_zero_limit() {
        let birth = Utc::now();
        let metrics = AgentMetrics::new("test".to_string(), birth, 0);

        assert_eq!(metrics.context_usage_percent(), 0.0);
    }

    #[test]
    fn test_get_rss_bytes_returns_value() {
        let rss = get_rss_bytes();
        // On Linux, should be non-zero; elsewhere, 0
        #[cfg(target_os = "linux")]
        assert!(rss > 0, "RSS should be positive on Linux");

        #[cfg(not(target_os = "linux"))]
        assert_eq!(rss, 0, "RSS should be 0 on non-Linux");
    }
}
```

- [ ] **Step 2: Export metrics module**

Add to `crates/river-gateway/src/lib.rs`:

```rust
pub mod metrics;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway metrics`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/metrics.rs crates/river-gateway/src/lib.rs
git commit -m "feat(gateway): add AgentMetrics and LoopStateLabel"
```

---

## Task 3: Create Logging Module

**Files:**
- Create: `crates/river-gateway/src/logging.rs`
- Modify: `crates/river-gateway/src/lib.rs`

- [ ] **Step 1: Create logging module**

Create `crates/river-gateway/src/logging.rs`:

```rust
//! Structured JSON logging with daily file rotation

use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Logging configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Directory for log files (default: {data-dir}/logs/)
    pub log_dir: PathBuf,
    /// Override log file path
    pub log_file: Option<PathBuf>,
    /// Output JSON to stdout (default: false for tty, true otherwise)
    pub json_stdout: bool,
    /// Log level filter (default: "info")
    pub log_level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            log_dir: PathBuf::from("logs"),
            log_file: None,
            json_stdout: false,
            log_level: "info".to_string(),
        }
    }
}

/// Guard that flushes logs on drop - must be kept alive
pub struct LogGuard {
    _file_guard: WorkerGuard,
    _stdout_guard: Option<WorkerGuard>,
}

/// Initialize logging with JSON output to file and stdout
///
/// Returns a guard that must be kept alive for the duration of the program.
pub fn init_logging(config: &LogConfig) -> Result<LogGuard, std::io::Error> {
    // Ensure log directory exists
    let log_dir = config.log_file
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.log_dir.clone());

    std::fs::create_dir_all(&log_dir)?;

    // Create daily rolling file appender
    let file_appender = tracing_appender::rolling::daily(&log_dir, "gateway");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    // Build filter from config or env
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    // File layer: always JSON
    let file_layer = fmt::layer()
        .json()
        .with_writer(file_writer)
        .with_file(true)
        .with_line_number(true);

    // Stdout layer: JSON if configured or non-tty, pretty otherwise
    let use_json_stdout = config.json_stdout || !atty::is(atty::Stream::Stdout);

    if use_json_stdout {
        let (stdout_writer, stdout_guard) = tracing_appender::non_blocking(std::io::stdout());
        let stdout_layer = fmt::layer()
            .json()
            .with_writer(stdout_writer);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(stdout_layer)
            .init();

        Ok(LogGuard {
            _file_guard: file_guard,
            _stdout_guard: Some(stdout_guard),
        })
    } else {
        let stdout_layer = fmt::layer()
            .pretty();

        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(stdout_layer)
            .init();

        Ok(LogGuard {
            _file_guard: file_guard,
            _stdout_guard: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert_eq!(config.log_level, "info");
        assert!(!config.json_stdout);
    }

    #[test]
    fn test_init_logging_creates_dir() {
        let dir = TempDir::new().unwrap();
        let log_dir = dir.path().join("logs");

        let config = LogConfig {
            log_dir: log_dir.clone(),
            log_file: None,
            json_stdout: false,
            log_level: "info".to_string(),
        };

        // Can't actually init logging in tests (global state), but we can verify the dir creation logic
        std::fs::create_dir_all(&log_dir).unwrap();
        assert!(log_dir.exists());
    }
}
```

- [ ] **Step 2: Add atty dependency for TTY detection**

Add to `crates/river-gateway/Cargo.toml` in `[dependencies]`:

```toml
atty = "0.2"
```

- [ ] **Step 3: Export logging module**

Add to `crates/river-gateway/src/lib.rs`:

```rust
pub mod logging;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p river-gateway`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/logging.rs crates/river-gateway/src/lib.rs crates/river-gateway/Cargo.toml
git commit -m "feat(gateway): add structured JSON logging with daily rotation"
```

---

## Task 4: Add Metrics to AppState

**Files:**
- Modify: `crates/river-gateway/src/state.rs`

- [ ] **Step 1: Add metrics field to AppState**

In `crates/river-gateway/src/state.rs`, add import at top:

```rust
use crate::metrics::AgentMetrics;
```

Add field to `AppState` struct (after `subagent_manager`):

```rust
    /// Shared metrics for observability
    pub metrics: Arc<RwLock<AgentMetrics>>,
```

- [ ] **Step 2: Update AppState::new to accept metrics**

Update the `new` function signature and body:

```rust
    pub fn new(
        config: GatewayConfig,
        db: Arc<Mutex<Database>>,
        registry: ToolRegistry,
        embedding_client: Option<EmbeddingClient>,
        redis_client: Option<RedisClient>,
        loop_tx: mpsc::Sender<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        auth_token: Option<String>,
        subagent_manager: Arc<RwLock<SubagentManager>>,
        metrics: Arc<RwLock<AgentMetrics>>,
    ) -> Self {
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(config.agent_birth));
        let executor = ToolExecutor::new(registry);

        Self {
            snowflake_gen,
            db,
            tool_executor: Arc::new(RwLock::new(executor)),
            embedding_client: embedding_client.map(Arc::new),
            redis_client: redis_client.map(Arc::new),
            loop_tx,
            message_queue,
            config,
            auth_token,
            subagent_manager,
            metrics,
        }
    }
```

- [ ] **Step 3: Add db_path helper to GatewayConfig**

Add method to `GatewayConfig`:

```rust
impl GatewayConfig {
    pub fn db_path(&self) -> std::path::PathBuf {
        self.data_dir.join("river.db")
    }
}
```

- [ ] **Step 4: Update test helper**

Update `test_state_creation` test to pass metrics:

```rust
    #[tokio::test]
    async fn test_state_creation() {
        use crate::metrics::AgentMetrics;

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
            agent_name: "test".to_string(),
            embedding: None,
            redis: None,
        };

        let birth_dt = chrono::Utc::now();
        let metrics = Arc::new(RwLock::new(AgentMetrics::new(
            config.agent_name.clone(),
            birth_dt,
            config.context_limit,
        )));

        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let (loop_tx, _loop_rx) = mpsc::channel(256);
        let message_queue = Arc::new(MessageQueue::new());
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));
        let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)));
        let state = AppState::new(
            config,
            db,
            registry,
            None,
            None,
            loop_tx,
            message_queue,
            None,
            subagent_manager,
            metrics,
        );

        assert_eq!(state.config.port, 3000);
        assert_eq!(state.config.context_limit, 65536);
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-gateway state`
Expected: Tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/state.rs
git commit -m "feat(gateway): add shared AgentMetrics to AppState"
```

---

## Task 5: Update API Routes Test Helpers

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Update test_state helper**

In `crates/river-gateway/src/api/routes.rs`, update the `test_state` function in the tests module:

```rust
    fn test_state() -> (Arc<AppState>, mpsc::Receiver<LoopEvent>) {
        use crate::metrics::AgentMetrics;

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

        let birth_dt = chrono::Utc::now();
        let metrics = Arc::new(tokio::sync::RwLock::new(AgentMetrics::new(
            config.agent_name.clone(),
            birth_dt,
            config.context_limit,
        )));

        let db = Arc::new(std::sync::Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let (loop_tx, loop_rx) = mpsc::channel(256);
        let message_queue = Arc::new(MessageQueue::new());
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));
        let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)));
        (Arc::new(AppState::new(config, db, registry, None, None, loop_tx, message_queue, None, subagent_manager, metrics)), loop_rx)
    }
```

- [ ] **Step 2: Update test_state_with_auth helper**

Similarly update `test_state_with_auth`:

```rust
    fn test_state_with_auth(token: &str) -> (Arc<AppState>, mpsc::Receiver<LoopEvent>) {
        use crate::metrics::AgentMetrics;

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

        let birth_dt = chrono::Utc::now();
        let metrics = Arc::new(tokio::sync::RwLock::new(AgentMetrics::new(
            config.agent_name.clone(),
            birth_dt,
            config.context_limit,
        )));

        let db = Arc::new(std::sync::Mutex::new(Database::open_in_memory().unwrap()));
        let registry = ToolRegistry::new();
        let (loop_tx, loop_rx) = mpsc::channel(256);
        let message_queue = Arc::new(MessageQueue::new());
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));
        let subagent_manager = Arc::new(RwLock::new(SubagentManager::new(snowflake_gen)));
        (Arc::new(AppState::new(config, db, registry, None, None, loop_tx, message_queue, Some(token.to_string()), subagent_manager, metrics)), loop_rx)
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway routes`
Expected: Tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/api/routes.rs
git commit -m "test(gateway): update route test helpers for metrics"
```

---

## Task 6: Implement Rich Health Endpoint

**Files:**
- Modify: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Add health response types**

Add after existing `HealthResponse` struct (replace it):

```rust
use crate::metrics::{get_rss_bytes, LoopStateLabel};
use chrono::{DateTime, Utc};

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
```

- [ ] **Step 2: Update health_check handler**

Replace the `health_check` function:

```rust
async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let metrics = state.metrics.read().await;

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

    Json(HealthResponse {
        status: "healthy",
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
    })
}
```

- [ ] **Step 3: Add test for rich health response**

Add test:

```rust
    #[tokio::test]
    async fn test_health_returns_rich_response() {
        let (state, _rx) = test_state();
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway routes`
Expected: Tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/api/routes.rs
git commit -m "feat(gateway): implement rich /health endpoint with metrics"
```

---

## Task 7: Update Server to Create Metrics

**Files:**
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Import metrics module**

Add at top of file:

```rust
use crate::metrics::AgentMetrics;
```

- [ ] **Step 2: Create metrics before AppState**

After `let snowflake_gen = ...` and before creating subagent_manager, add:

```rust
    // Create shared metrics
    let metrics = Arc::new(RwLock::new(AgentMetrics::new(
        gateway_config.agent_name.clone(),
        chrono::DateTime::from_timestamp(
            agent_birth.to_timestamp() as i64,
            0,
        ).unwrap_or_else(|| Utc::now()),
        gateway_config.context_limit,
    )));
```

- [ ] **Step 3: Pass metrics to AppState::new**

Update the `AppState::new` call to include metrics as the last argument:

```rust
    // Create app state
    let state = Arc::new(AppState::new(
        gateway_config,
        db_arc.clone(),
        registry,
        embedding_client,
        redis_client,
        loop_tx,
        message_queue.clone(),
        auth_token,
        subagent_manager,
        metrics.clone(),
    ));
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p river-gateway`
Expected: Compiles (AgentLoop changes come next)

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/server.rs
git commit -m "feat(gateway): create and wire AgentMetrics in server"
```

---

## Task 8: Update AgentLoop to Update Metrics

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Add metrics imports and field**

Add import at top:

```rust
use crate::metrics::{AgentMetrics, LoopStateLabel};
```

Add field to `AgentLoop` struct (after `last_prompt_tokens`):

```rust
    /// Shared metrics for observability
    metrics: Arc<RwLock<AgentMetrics>>,
```

- [ ] **Step 2: Update AgentLoop::new**

Add `metrics` parameter and store it:

```rust
    pub fn new(
        event_rx: mpsc::Receiver<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
        heartbeat_scheduler: Arc<HeartbeatScheduler>,
        context_rotation: Arc<ContextRotation>,
        config: LoopConfig,
        metrics: Arc<RwLock<AgentMetrics>>,
    ) -> Self {
        let git = GitOps::new(&config.workspace);
        Self {
            state: LoopState::Sleeping,
            event_rx,
            message_queue,
            model_client,
            context: ContextBuilder::new(),
            tool_executor,
            db,
            snowflake_gen,
            heartbeat_scheduler,
            context_rotation,
            shutdown_requested: false,
            git,
            config,
            pending_notifications: Vec::new(),
            needs_context_reset: true,
            context_id: None,
            context_file: None,
            last_prompt_tokens: 0,
            metrics,
        }
    }
```

- [ ] **Step 3: Add helper to update metrics state**

Add method to `AgentLoop`:

```rust
    /// Update shared metrics with current loop state
    async fn update_metrics_state(&self, new_state: LoopStateLabel) {
        let mut m = self.metrics.write().await;
        m.loop_state = new_state;
        match new_state {
            LoopStateLabel::Waking => {
                m.last_wake = Some(Utc::now());
            }
            LoopStateLabel::Settling => {
                m.last_settle = Some(Utc::now());
                m.turns_since_restart += 1;
            }
            _ => {}
        }
    }

    /// Update context metrics
    async fn update_metrics_context(&self) {
        let mut m = self.metrics.write().await;
        m.context_tokens = self.last_prompt_tokens;
        m.context_id = self.context_id.map(|id| id.to_string());
    }
```

- [ ] **Step 4: Call update_metrics_state in phase transitions**

In `sleep_phase`, after the match on event, add before state transitions:

```rust
                    Some(LoopEvent::InboxUpdate(paths)) => {
                        tracing::info!(event = "loop.wake", trigger = "inbox", file_count = paths.len(), "Wake: inbox update");
                        self.update_metrics_state(LoopStateLabel::Waking).await;
                        self.state = LoopState::Waking {
                            trigger: WakeTrigger::Inbox(paths)
                        };
                    }
                    Some(LoopEvent::Heartbeat) => {
                        tracing::info!(event = "loop.wake", trigger = "heartbeat", "Wake: heartbeat");
                        self.update_metrics_state(LoopStateLabel::Waking).await;
                        self.state = LoopState::Waking {
                            trigger: WakeTrigger::Heartbeat
                        };
                    }
```

And in the heartbeat timer branch:

```rust
            _ = tokio::time::sleep(heartbeat_delay) => {
                tracing::info!(event = "loop.wake", trigger = "heartbeat_timer", "Wake: heartbeat timer");
                self.update_metrics_state(LoopStateLabel::Waking).await;
                self.state = LoopState::Waking {
                    trigger: WakeTrigger::Heartbeat
                };
            }
```

- [ ] **Step 5: Update remaining phases**

At the end of `wake_phase`, before setting state to Thinking:

```rust
        self.update_metrics_state(LoopStateLabel::Thinking).await;
        self.state = LoopState::Thinking;
```

At the start of `think_phase`, increment model_calls and update after response:

```rust
    async fn think_phase(&mut self) {
        // Increment model call counter
        {
            let mut m = self.metrics.write().await;
            m.model_calls += 1;
        }
```

After receiving response (after `self.last_prompt_tokens = ...`):

```rust
        self.update_metrics_context().await;
```

Before transitioning to Acting:

```rust
            self.update_metrics_state(LoopStateLabel::Acting).await;
            self.state = LoopState::Acting {
                pending: response.tool_calls,
            };
```

Before transitioning to Settling from think_phase:

```rust
            self.update_metrics_state(LoopStateLabel::Settling).await;
            self.state = LoopState::Settling;
```

At start of `settle_phase`:

```rust
    async fn settle_phase(&mut self) {
        self.update_metrics_state(LoopStateLabel::Settling).await;
```

Handle context rotation counter in settle_phase (after successful rotation):

```rust
                if let Err(e) = result {
                    tracing::error!(error = %e, "Failed to create new context");
                } else {
                    // Increment rotation counter
                    let mut m = self.metrics.write().await;
                    m.rotations_since_restart += 1;
                }
```

At end of settle_phase before sleeping:

```rust
        self.update_metrics_state(LoopStateLabel::Sleeping).await;
        self.state = LoopState::Sleeping;
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check -p river-gateway`
Expected: Compiles (need to update server.rs to pass metrics to AgentLoop)

- [ ] **Step 7: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "feat(gateway): update AgentLoop to track metrics"
```

---

## Task 9: Pass Metrics to AgentLoop in Server

**Files:**
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Pass metrics to AgentLoop::new**

Update the `AgentLoop::new` call:

```rust
    // Spawn the agent loop
    let mut agent_loop = AgentLoop::new(
        loop_rx,
        message_queue,
        model_client,
        state.tool_executor.clone(),
        db_arc,
        snowflake_gen,
        heartbeat_scheduler,
        context_rotation,
        loop_config,
        metrics,
    );
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/server.rs
git commit -m "feat(gateway): pass metrics to AgentLoop"
```

---

## Task 10: Update Tool Executor to Track Counters

**Files:**
- Modify: `crates/river-gateway/src/tools/executor.rs`

- [ ] **Step 1: Add metrics to ToolExecutor**

Add import and field:

```rust
use crate::metrics::AgentMetrics;
use std::sync::Arc;
use tokio::sync::RwLock;
```

Update struct:

```rust
pub struct ToolExecutor {
    registry: ToolRegistry,
    metrics: Option<Arc<RwLock<AgentMetrics>>>,
}
```

- [ ] **Step 2: Update constructor**

```rust
impl ToolExecutor {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            metrics: None,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<RwLock<AgentMetrics>>) -> Self {
        self.metrics = Some(metrics);
        self
    }
```

- [ ] **Step 3: Increment counters in execute**

In the `execute` method, after getting the result, add counter updates:

```rust
        // Update metrics counters
        if let Some(ref metrics) = self.metrics {
            if let Ok(mut m) = metrics.try_write() {
                m.tool_calls += 1;
                if result.is_err() {
                    m.tool_errors += 1;
                }
            }
        }
```

Add this just before creating the `ToolCallResponse`.

- [ ] **Step 4: Update server to use with_metrics**

In `crates/river-gateway/src/server.rs`, after creating the executor, update it:

Find where `ToolExecutor::new(registry)` is used (in `AppState::new`) and instead update `server.rs` to create the executor with metrics before passing to AppState.

Actually, looking at the code, the executor is created inside `AppState::new`. We need to pass metrics there too. Update `AppState::new`:

```rust
    pub fn new(
        config: GatewayConfig,
        db: Arc<Mutex<Database>>,
        registry: ToolRegistry,
        embedding_client: Option<EmbeddingClient>,
        redis_client: Option<RedisClient>,
        loop_tx: mpsc::Sender<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        auth_token: Option<String>,
        subagent_manager: Arc<RwLock<SubagentManager>>,
        metrics: Arc<RwLock<AgentMetrics>>,
    ) -> Self {
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(config.agent_birth));
        let executor = ToolExecutor::new(registry).with_metrics(metrics.clone());

        Self {
            snowflake_gen,
            db,
            tool_executor: Arc::new(RwLock::new(executor)),
            embedding_client: embedding_client.map(Arc::new),
            redis_client: redis_client.map(Arc::new),
            loop_tx,
            message_queue,
            config,
            auth_token,
            subagent_manager,
            metrics,
        }
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/tools/executor.rs crates/river-gateway/src/state.rs
git commit -m "feat(gateway): track tool call counts in metrics"
```

---

## Task 11: Add CLI Flags for Logging

**Files:**
- Modify: `crates/river-gateway/src/main.rs`

- [ ] **Step 1: Add CLI arguments**

Add to `Args` struct:

```rust
    /// Log file directory (default: {data-dir}/logs/)
    #[arg(long)]
    log_dir: Option<PathBuf>,

    /// Override log file path
    #[arg(long)]
    log_file: Option<PathBuf>,

    /// Output JSON logs to stdout (default: pretty for tty, json otherwise)
    #[arg(long)]
    json_stdout: bool,

    /// Log level (default: info, or RUST_LOG env)
    #[arg(long, default_value = "info")]
    log_level: String,
```

- [ ] **Step 2: Initialize logging before server**

Replace `tracing_subscriber::fmt::init();` with:

```rust
    use river_gateway::logging::{LogConfig, init_logging};

    let log_config = LogConfig {
        log_dir: args.log_dir.unwrap_or_else(|| data_dir.join("logs")),
        log_file: args.log_file,
        json_stdout: args.json_stdout,
        log_level: args.log_level.clone(),
    };

    let _log_guard = init_logging(&log_config)
        .map_err(|e| anyhow::anyhow!("Failed to initialize logging: {}", e))?;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p river-gateway`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/main.rs
git commit -m "feat(gateway): add CLI flags for structured logging"
```

---

## Task 12: Add Structured Event Logging

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Add structured event fields to existing logs**

Update the tracing calls throughout the loop to use standardized event names.

In `sleep_phase`, the wake events already have `event = "loop.wake"`.

In `think_phase`, before model call:

```rust
        tracing::info!(
            event = "loop.think",
            message_count = message_count,
            tool_count = tool_count,
            "Calling model"
        );
```

After model response:

```rust
        tracing::info!(
            event = "loop.response",
            tokens_total = response.usage.total_tokens,
            tokens_prompt = response.usage.prompt_tokens,
            tokens_completion = response.usage.completion_tokens,
            tool_calls = response.tool_calls.len(),
            has_content = response.content.is_some(),
            "Model response received"
        );
```

In `act_phase`, for tool execution:

```rust
                tracing::info!(
                    event = "loop.tool",
                    tool_name = %tc.function.name,
                    call_id = %tc.id,
                    success = success,
                    "Tool execution complete"
                );
```

In `settle_phase`:

```rust
        tracing::info!(event = "loop.settle", "Turn complete, settling");
```

At end before sleeping:

```rust
        tracing::debug!(event = "loop.sleep", "Entering sleep");
```

- [ ] **Step 2: Add context warning events**

In `wake_phase` where context warning is added, also log:

```rust
        if context_percent >= 80.0 && context_percent < 90.0 {
            tracing::warn!(
                event = "context.warning",
                usage_percent = format!("{:.1}", context_percent),
                threshold = 80,
                "Context usage high"
            );
```

In `settle_phase` for rotation:

```rust
            tracing::info!(
                event = "context.rotate",
                has_summary = summary_opt.is_some(),
                old_tokens = self.last_prompt_tokens,
                "Processing context rotation"
            );
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "feat(gateway): add structured event logging"
```

---

## Task 13: Final Integration Test

**Files:**
- Test manually

- [ ] **Step 1: Build the gateway**

Run: `cargo build -p river-gateway`
Expected: Builds successfully

- [ ] **Step 2: Verify health endpoint structure**

Run a quick test (if you have a test database):

```bash
# From project root, if test DB exists:
curl -s http://localhost:3000/health | jq .
```

Or verify via unit tests:

Run: `cargo test -p river-gateway test_health_returns_rich_response`
Expected: Test passes

- [ ] **Step 3: Verify log file creation**

The daily log files will be created in `{data-dir}/logs/gateway.YYYY-MM-DD` when the server runs.

- [ ] **Step 4: Run full test suite**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "test(gateway): verify Phase 1 monitoring integration"
```

---

## Summary

This plan implements Phase 1 Monitoring with:

1. **AgentMetrics** — Shared state struct with loop state, context, counters
2. **Rich /health** — JSON response with full observability data
3. **Structured logging** — JSON to daily files + stdout with event taxonomy
4. **CLI flags** — `--log-dir`, `--log-file`, `--json-stdout`, `--log-level`

Total: 13 tasks, ~65 steps

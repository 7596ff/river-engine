# Minimal Orchestrator Design Specification

**Version:** 1.0
**Date:** 2026-03-16
**Status:** Draft

---

## 1. Overview

The orchestrator is a lightweight coordination service for River Engine. This minimal implementation focuses on:

1. **Agent health monitoring** - Track gateway heartbeats, detect failures
2. **Agent status API** - Expose agent health to external systems
3. **Model registry** - Static list of available models

### Out of Scope (Future Plans)

- Dynamic model server lifecycle (spin up/down)
- GPU resource allocation
- Priority queue management
- Agent restart automation

---

## 2. Architecture

### 2.1 New Crate

```
river-engine/
├── crates/
│   ├── river-core/          # Shared types (existing)
│   ├── river-gateway/       # Per-agent gateway (existing)
│   └── river-orchestrator/  # NEW: Coordination service
```

### 2.2 Runtime Topology

```
┌─────────────────────────────────────────────────────────┐
│                    Orchestrator                          │
│  - Tracks agent health via heartbeats                   │
│  - Serves model registry                                │
│  - Single instance per deployment                       │
└─────────────────────────────────────────────────────────┘
          ▲                              ▲
          │ POST /heartbeat              │ GET /models/available
          │                              │
┌─────────────────┐            ┌─────────────────┐
│  Gateway: thomas │            │  Gateway: river  │
└─────────────────┘            └─────────────────┘
```

### 2.3 State Model

In-memory state (no persistence). On orchestrator restart, agents re-register via their next heartbeat.

---

## 3. File Structure

```
crates/river-orchestrator/
├── Cargo.toml
└── src/
    ├── lib.rs              # Public API exports
    ├── main.rs             # CLI entry point
    ├── config.rs           # Configuration types
    ├── state.rs            # Shared application state
    ├── agents/
    │   ├── mod.rs          # Agent registry
    │   └── health.rs       # Health check logic
    ├── models/
    │   └── mod.rs          # Static model registry
    └── api/
        ├── mod.rs          # Router setup
        └── routes.rs       # HTTP handlers
```

---

## 4. Data Structures

### 4.1 Orchestrator State

```rust
pub struct OrchestratorState {
    pub agents: RwLock<HashMap<String, AgentInfo>>,
    pub models: Vec<ModelInfo>,
    pub config: OrchestratorConfig,
}
```

### 4.2 Agent Info

```rust
pub struct AgentInfo {
    pub name: String,
    pub gateway_url: String,
    pub last_heartbeat: Instant,
    pub registered_at: Instant,
}

impl AgentInfo {
    pub fn is_healthy(&self, threshold: Duration) -> bool {
        self.last_heartbeat.elapsed() < threshold
    }
}
```

### 4.3 Model Info

```rust
pub struct ModelInfo {
    pub name: String,
    pub provider: ModelProvider,
    pub status: ModelStatus,
}

pub enum ModelProvider {
    Local,    // llama-server
    LiteLLM,  // API proxy
}

pub enum ModelStatus {
    Available,
    Unavailable,
}
```

### 4.4 Configuration

```rust
pub struct OrchestratorConfig {
    pub port: u16,
    pub health_threshold_seconds: u64,  // Default: 120
    pub models: Vec<ModelConfig>,
}

pub struct ModelConfig {
    pub name: String,
    pub provider: String,
}
```

---

## 5. API Endpoints

### 5.1 POST /heartbeat

Agent sends periodic heartbeat.

**Request:**
```json
{
  "agent": "thomas",
  "gateway_url": "http://localhost:3000"
}
```

**Response:**
```json
{
  "acknowledged": true
}
```

**Behavior:**
- Creates agent entry if not exists
- Updates `last_heartbeat` timestamp
- Updates `gateway_url` if changed

### 5.2 GET /agents/status

List all registered agents with health status.

**Response:**
```json
[
  {
    "name": "thomas",
    "gateway_url": "http://localhost:3000",
    "healthy": true,
    "last_heartbeat_seconds_ago": 15,
    "registered_at": "2026-03-16T14:00:00Z"
  }
]
```

### 5.3 GET /models/available

List configured models.

**Response:**
```json
[
  {
    "name": "qwen3-32b-q4_k_m",
    "provider": "local",
    "status": "available"
  },
  {
    "name": "claude-sonnet-4-20250514",
    "provider": "litellm",
    "status": "available"
  }
]
```

### 5.4 GET /health

Orchestrator health check.

**Response:**
```json
{
  "status": "ok",
  "version": "0.1.0",
  "agents_registered": 2
}
```

---

## 6. Gateway Integration

### 6.1 New CLI Flag

```bash
river-gateway \
  --workspace /path/to/workspace \
  --data-dir /path/to/data \
  --orchestrator-url http://localhost:5000  # NEW
```

### 6.2 Heartbeat Task

Gateway spawns a background task on startup:

```rust
async fn heartbeat_loop(orchestrator_url: String, agent_name: String, gateway_url: String) {
    let client = reqwest::Client::new();
    loop {
        let result = client.post(format!("{}/heartbeat", orchestrator_url))
            .json(&HeartbeatRequest { agent: agent_name.clone(), gateway_url: gateway_url.clone() })
            .send()
            .await;

        if let Err(e) = result {
            tracing::warn!("Failed to send heartbeat: {}", e);
        }

        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}
```

### 6.3 Graceful Degradation

- If `--orchestrator-url` not provided, gateway operates standalone (no heartbeats)
- If orchestrator unreachable, gateway logs warning but continues normal operation
- Heartbeat failures do not affect gateway functionality

---

## 7. Health Logic

### 7.1 Threshold

Default: 120 seconds (2 minutes)

Agent marked unhealthy if `now - last_heartbeat > threshold`.

### 7.2 Staleness Calculation

```rust
pub fn seconds_since_heartbeat(&self) -> u64 {
    self.last_heartbeat.elapsed().as_secs()
}

pub fn is_healthy(&self, threshold: Duration) -> bool {
    self.last_heartbeat.elapsed() < threshold
}
```

### 7.3 Agent Cleanup

Agents are NOT automatically removed when unhealthy. They remain in the registry with `healthy: false` until:
- Orchestrator restarts (in-memory state cleared)
- Future: explicit DELETE endpoint

---

## 8. CLI Interface

```bash
river-orchestrator \
  --port 5000 \
  --health-threshold 120 \
  --models-config /path/to/models.json
```

**models.json:**
```json
{
  "models": [
    { "name": "qwen3-32b-q4_k_m", "provider": "local" },
    { "name": "claude-sonnet-4-20250514", "provider": "litellm" }
  ]
}
```

---

## 9. Error Handling

### 9.1 Orchestrator Errors

Add to `river-core::RiverError`:
- Already exists: `Orchestrator(String)` variant

### 9.2 Gateway Heartbeat Errors

- Network errors: Log warning, retry on next interval
- 4xx/5xx responses: Log warning, retry on next interval
- No crash, no panic, graceful degradation

---

## 10. Testing Strategy

### 10.1 Unit Tests

- `agents/health.rs`: Threshold calculations, staleness detection
- `config.rs`: Config parsing, defaults

### 10.2 Integration Tests

- Register agent via heartbeat, verify in status
- Verify health status changes after threshold
- Model registry returns configured models

### 10.3 API Tests

- Each endpoint with valid/invalid inputs
- Content-type headers
- Error responses

---

## 11. Dependencies

```toml
[dependencies]
river-core = { path = "../river-core" }
axum = "0.8"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
clap = { version = "4", features = ["derive"] }
chrono = { version = "0.4", features = ["serde"] }
```

---

## 12. Future Extensions

After this minimal implementation, future plans can add:

1. **Model lifecycle** - Spin up/down llama-server processes
2. **Resource allocation** - GPU memory tracking, request queuing
3. **Agent restart** - Detect unhealthy agents, trigger restart via systemd
4. **Priority queue** - Interactive > Scheduled > Background
5. **Persistence** - SQLite for historical data, crash recovery

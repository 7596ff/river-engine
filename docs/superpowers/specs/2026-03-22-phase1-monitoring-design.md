# Phase 1 Monitoring: Visibility

**Status:** Approved
**Date:** 2026-03-22
**Author:** Cassie + Claude

## Problem

River agents are opaque at runtime:
- No visibility into loop state (sleeping? thinking? stuck?)
- Context usage unknown until overflow
- DB growth (222MB+) untracked
- Logs semi-structured, hard to parse programmatically
- Manual restarts required when issues occur

## Goals

1. **Health endpoint** — Machine-readable state for monitoring agents (William)
2. **Structured logging** — JSON logs parseable with `jq`
3. **Context tracking** — Expose token usage, warn before overflow
4. **Resource visibility** — DB size, RSS memory

## Non-Goals (Future Phases)

- Systemd watchdog integration
- External alerting (Discord webhooks)
- Cross-agent monitoring tools
- Prometheus metrics endpoint

---

## Design

### 1. Shared AgentMetrics Struct

Single source of truth shared between `AgentLoop` and health endpoint:

```rust
// src/metrics.rs

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

    // Resources
    pub db_size_bytes: u64,
    pub rss_bytes: u64,

    // Counters (reset on restart)
    pub model_calls: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoopStateLabel {
    #[default]
    Sleeping,
    Waking,
    Thinking,
    Acting,
    Settling,
}
```

The loop updates metrics at phase transitions. Health endpoint reads via `Arc<RwLock<AgentMetrics>>`.

### 2. Health Endpoint

Rich `/health` response with computed fields:

```rust
// src/api/routes.rs

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

Example response:

```json
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_seconds": 3600,
  "agent": {
    "name": "thomas",
    "birth": "2026-03-15T10:00:00Z"
  },
  "loop_state": {
    "state": "sleeping",
    "last_wake": "2026-03-22T14:30:00Z",
    "last_settle": "2026-03-22T14:31:00Z",
    "turns_since_restart": 42
  },
  "context": {
    "current_tokens": 45000,
    "limit_tokens": 200000,
    "usage_percent": 22.5,
    "context_id": "ctx_abc123",
    "rotations": 3
  },
  "resources": {
    "db_size_bytes": 222000000,
    "rss_bytes": 150000000
  },
  "counters": {
    "model_calls": 142,
    "tool_calls": 350,
    "tool_errors": 2
  }
}
```

### 3. Structured Logging

JSON logs to daily files plus stdout:

```rust
// src/logging.rs

pub struct LogConfig {
    pub log_dir: PathBuf,          // default: {data-dir}/logs/
    pub log_file: Option<PathBuf>, // --log-file override
    pub json_stdout: bool,         // false for tty, true otherwise
}

pub fn init_logging(config: &LogConfig) -> Result<LogGuard, RiverError> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // File: JSON, daily rotation
    let log_path = daily_log_path(config);  // gateway-2026-03-22.jsonl
    let file = File::create(&log_path)?;
    let file_layer = fmt::layer()
        .json()
        .with_writer(file);

    // Stdout: JSON in prod, pretty in dev
    let stdout_layer = if config.json_stdout {
        fmt::layer().json().boxed()
    } else {
        fmt::layer().pretty().boxed()
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(stdout_layer)
        .init();

    Ok(LogGuard { /* flush on drop */ })
}

fn daily_log_path(config: &LogConfig) -> PathBuf {
    let date = Utc::now().format("%Y-%m-%d");
    let dir = config.log_file.as_ref()
        .and_then(|p| p.parent())
        .unwrap_or(&config.log_dir);
    dir.join(format!("gateway-{}.jsonl", date))
}
```

### 4. Log Events & Levels

| Level | Event | Fields | When |
|-------|-------|--------|------|
| INFO | `loop.wake` | `trigger`, `queued_messages` | Agent wakes |
| INFO | `loop.think` | `prompt_tokens`, `model` | Model call starts |
| INFO | `loop.response` | `response_tokens`, `tool_calls`, `duration_ms` | Model returns |
| INFO | `loop.tool` | `tool_name`, `call_id`, `duration_ms`, `success` | Tool completes |
| INFO | `loop.settle` | `total_tokens`, `turn_duration_ms` | Turn complete |
| DEBUG | `loop.sleep` | `next_heartbeat_mins` | Going idle |
| WARN | `context.warning` | `usage_percent`, `threshold` | 80%+ context |
| INFO | `context.rotate` | `reason`, `old_tokens`, `new_context_id` | Rotation |
| ERROR | `error` | `error_type`, `message`, `source` | Any error |
| DEBUG | `tool.args` | `tool_name`, `arguments` | Tool input |
| TRACE | `model.request` | `messages_count`, `tools_count` | Full request |

Usage:

```bash
# Production
RUST_LOG=info river-gateway ...

# Debugging tools
RUST_LOG=river_gateway::tools=debug river-gateway ...

# Full visibility
RUST_LOG=trace river-gateway ...

# Query logs
cat gateway-2026-03-22.jsonl | jq 'select(.fields.event == "loop.think")'
```

### 5. Loop Integration

`AgentLoop` updates shared metrics at phase transitions:

```rust
pub struct AgentLoop {
    // ... existing fields
    metrics: Arc<RwLock<AgentMetrics>>,
}

impl AgentLoop {
    async fn update_state(&self, new_state: LoopStateLabel) {
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

    async fn sleep_phase(&mut self) {
        self.update_state(LoopStateLabel::Sleeping).await;
        tracing::debug!(event = "loop.sleep", "Entering sleep");
        // ... existing logic
    }

    // Similar for wake_phase, think_phase, act_phase, settle_phase
}
```

### 6. RSS Tracking

```rust
// src/metrics.rs

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
```

### 7. CLI Configuration

New flags for `river-gateway`:

```rust
#[derive(Parser)]
struct Args {
    // ... existing flags

    /// Log file path (default: {data-dir}/logs/gateway-{date}.jsonl)
    #[arg(long)]
    log_file: Option<PathBuf>,

    /// Output JSON logs to stdout (default: pretty for tty, json otherwise)
    #[arg(long)]
    json_stdout: bool,

    /// Log level (default: info, or RUST_LOG env)
    #[arg(long, default_value = "info")]
    log_level: String,
}
```

Auto-detects TTY for stdout format.

---

## Files Changed

**New files:**
- `crates/river-gateway/src/metrics.rs` — `AgentMetrics`, `LoopStateLabel`, `get_rss_bytes()`
- `crates/river-gateway/src/logging.rs` — `LogConfig`, `init_logging()`, daily rotation

**Modified files:**
- `crates/river-gateway/src/lib.rs` — export new modules
- `crates/river-gateway/src/main.rs` — CLI flags, logging init
- `crates/river-gateway/src/state.rs` — add `metrics: Arc<RwLock<AgentMetrics>>` to `AppState`
- `crates/river-gateway/src/api/routes.rs` — rich health endpoint
- `crates/river-gateway/src/loop/mod.rs` — inject metrics, call `update_state()` at transitions
- `crates/river-gateway/src/tools/executor.rs` — increment `tool_calls`, `tool_errors`

---

## Testing

1. **Unit tests** — `AgentMetrics` defaults, `get_rss_bytes()` non-zero on Linux
2. **Integration test** — start gateway, GET `/health`, verify JSON structure
3. **Log verification** — run with `RUST_LOG=debug`, confirm JSONL parseable with `jq`

```rust
#[tokio::test]
async fn test_health_returns_metrics() {
    let app = test_app();
    let resp = app.get("/health").await;
    let health: HealthResponse = resp.json().await;

    assert_eq!(health.status, "healthy");
    assert!(health.context.usage_percent >= 0.0);
    assert!(health.resources.rss_bytes > 0);
}
```

---

## What This Enables

- William polls `/health`, gets machine-readable loop state and context usage
- Humans query logs with `jq` for debugging
- Context warnings at 80%+, visible in health and logs
- Debug production issues by bumping `RUST_LOG`, no code changes needed

# Monitoring & Observability

**Status:** Draft
**Author:** Cassie (based on William's doc)
**Date:** 2026-03-21

## Problem

River agents are unstable and opaque:
- Session DB grows unbounded (222MB+)
- Context overflows cause crashes
- No visibility into what's happening
- Manual restarts required
- No metrics, no alerts

## Goals

1. **Health visibility** — Know when an agent is down
2. **Structured logging** — Parseable logs for debugging and analysis
3. **Context tracking** — See growth patterns, prevent overflows
4. **Self-healing** — Auto-restart on failure
5. **Cross-agent monitoring** — Agents can monitor each other

---

## 1. Health Endpoint

**Current:** `/health` exists but minimal.

**Proposed:** Rich health response with metrics.

```json
GET /health

{
  "status": "healthy",
  "uptime_seconds": 3600,
  "agent": {
    "name": "thomas",
    "birth": "2026-03-15T10:00:00Z"
  },
  "loop": {
    "state": "sleeping",
    "last_wake": "2026-03-21T14:30:00Z",
    "turns_since_restart": 42
  },
  "context": {
    "current_tokens": 45000,
    "limit_tokens": 200000,
    "usage_percent": 22.5
  },
  "database": {
    "size_bytes": 222000000,
    "messages_count": 1500
  },
  "memory": {
    "rss_bytes": 150000000
  }
}
```

### Implementation

```rust
// src/api/routes.rs
async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let db_size = std::fs::metadata(&state.config.db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Json(HealthResponse {
        status: "healthy",
        uptime_seconds: state.start_time.elapsed().as_secs(),
        agent: AgentHealth {
            name: state.config.agent_name.clone(),
            birth: state.config.agent_birth,
        },
        loop_state: state.loop_state.read().await.clone(),
        context: state.context_metrics.read().await.clone(),
        database: DatabaseHealth {
            size_bytes: db_size,
            messages_count: state.db.message_count()?,
        },
        memory: MemoryHealth {
            rss_bytes: get_rss_bytes(),
        },
    })
}
```

---

## 2. Structured Logging

**Current:** Tracing to stdout/journald, semi-structured.

**Proposed:** JSON logging with consistent fields.

### Log Events

| Event | Fields | Purpose |
|-------|--------|---------|
| `loop.wake` | `trigger`, `queued_messages` | Agent woke up |
| `loop.think` | `prompt_tokens`, `model` | Sent request to model |
| `loop.response` | `response_tokens`, `tool_calls` | Got model response |
| `loop.tool` | `tool_name`, `duration_ms`, `success` | Tool execution |
| `loop.settle` | `total_tokens`, `duration_ms` | Turn complete |
| `loop.sleep` | `next_heartbeat_mins` | Going to sleep |
| `context.rotate` | `reason`, `summary_length`, `old_tokens` | Context rotated |
| `context.warning` | `usage_percent`, `threshold` | Approaching limit |
| `error` | `error_type`, `message`, `stack` | Any error |

### Format

```json
{"ts":"2026-03-21T14:30:00Z","event":"loop.think","prompt_tokens":45000,"model":"claude-haiku-4-5"}
{"ts":"2026-03-21T14:30:05Z","event":"loop.response","response_tokens":500,"tool_calls":2}
{"ts":"2026-03-21T14:30:10Z","event":"loop.tool","tool_name":"read","duration_ms":50,"success":true}
```

### Implementation

```rust
// Add to existing tracing setup
tracing_subscriber::fmt()
    .json()
    .with_file(true)
    .with_line_number(true)
    .init();

// Structured log macros
tracing::info!(
    event = "loop.think",
    prompt_tokens = tokens,
    model = model_name,
    "Sending request to model"
);
```

### Log File

```bash
# In addition to journald
--log-file /var/log/river/thomas.jsonl
```

---

## 3. Context Tracking

**Problem:** Context grows until overflow, then crash.

**Solution:** Track and expose context metrics, warn before overflow.

### Metrics to Track

```rust
pub struct ContextMetrics {
    pub current_tokens: u64,
    pub limit_tokens: u64,
    pub usage_percent: f64,
    pub last_rotation: Option<DateTime<Utc>>,
    pub rotations_since_restart: u32,
}
```

### Thresholds

| Threshold | Action |
|-----------|--------|
| 80% | Log warning |
| 90% | Trigger auto-rotation |
| 95% | Hard gate, force rotation |

### Logging

```json
{"event":"context.warning","usage_percent":82,"threshold":80}
{"event":"context.rotate","reason":"auto_threshold","old_tokens":185000}
```

---

## 4. Self-Healing

### Systemd Watchdog

```ini
# river-thomas-gateway.service
[Service]
WatchdogSec=60
Restart=on-failure
RestartSec=5
```

The gateway pings the watchdog on each heartbeat:
```rust
sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog])?;
```

If no ping for 60s, systemd restarts the service.

### Health-Based Restart

External monitor (William or cron):
```bash
#!/bin/bash
if ! curl -sf http://localhost:3000/health > /dev/null; then
    systemctl --user restart river-thomas-gateway
    # Alert to Discord
    curl -X POST "$DISCORD_WEBHOOK" -d '{"content":"Thomas restarted (health check failed)"}'
fi
```

---

## 5. Cross-Agent Monitoring

William (on OpenClaw) monitors Thomas (on river-engine).

### William's Tools

1. **Health polling** — Check Thomas's `/health` endpoint
2. **Log reading** — Parse Thomas's log file for errors/patterns
3. **Restart** — SSH command to restart Thomas's service
4. **Alerting** — Post to Discord when issues detected

### Log Analysis

William can read Thomas's logs and identify:
- Repeated errors (same error 3+ times)
- Context growth rate (tokens/hour)
- Tool failure patterns
- Unusual quiet periods

### Report Format

William posts daily/on-demand:
```
📊 Thomas Health Report (2026-03-21)

Uptime: 23h 45m
Turns: 142
Context rotations: 3
Errors: 2 (both tool timeouts)
DB size: 225MB (+3MB today)

⚠️ Context growth rate elevated (12k tokens/hour)
```

---

## 6. Metrics Export (Future)

For dashboards and long-term analysis:

```
GET /metrics (Prometheus format)

river_uptime_seconds 3600
river_loop_turns_total 142
river_context_tokens 45000
river_context_limit_tokens 200000
river_db_size_bytes 222000000
river_tool_duration_seconds{tool="read"} 0.05
river_tool_errors_total{tool="bash"} 2
```

---

## Implementation Phases

### Phase 1: Visibility (Immediate)

- [ ] Rich `/health` endpoint with context metrics
- [ ] JSON structured logging
- [ ] Log file output (not just journald)
- [ ] Context threshold warnings

### Phase 2: Self-Healing (Short-term)

- [ ] Systemd watchdog integration
- [ ] Health check script for manual/cron use
- [ ] Discord webhook alerts

### Phase 3: Cross-Agent (Medium-term)

- [ ] William health check tool
- [ ] Log reading and pattern detection
- [ ] Automated restart capability
- [ ] Daily health reports

### Phase 4: Metrics (Long-term)

- [ ] Prometheus metrics endpoint
- [ ] Grafana dashboard
- [ ] Historical analysis

---

## Files to Modify

| File | Change |
|------|--------|
| `src/api/routes.rs` | Rich health endpoint |
| `src/main.rs` | JSON logging, log file option |
| `src/loop/mod.rs` | Context metrics tracking, structured logs |
| `src/state.rs` | Add metrics state |
| `systemd/` | Watchdog configuration |

---

## Notes

- William monitors from OpenClaw; Claude Code does development
- The 222MB DB is mostly message history — needs rotation/archival
- Context rotation (already implemented) should log events
- Eventually agents could self-report to a shared metrics store

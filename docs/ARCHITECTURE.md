# River Engine Architecture

River Engine is a Rust-based agentic AI system designed for continuous autonomous operation. It provides a modular architecture for running AI agents that can interact with users through various communication adapters, execute tools, maintain persistent memory, and coordinate with an orchestration layer.

## Table of Contents

1. [Overview](#overview)
2. [Crate Structure](#crate-structure)
3. [Core Concepts](#core-concepts)
4. [Gateway Runtime](#gateway-runtime)
5. [Tool System](#tool-system)
6. [Memory Architecture](#memory-architecture)
7. [Subagent System](#subagent-system)
8. [Database Schema](#database-schema)
9. [API Reference](#api-reference)
10. [Configuration](#configuration)
11. [Runtime Behavior](#runtime-behavior)

---

## Overview

River Engine consists of four main crates working together:

```
┌─────────────────────────────────────────────────────────────┐
│                    Communication Layer                       │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐          │
│  │   Discord   │  │    Slack    │  │   Custom    │  ...     │
│  │   Adapter   │  │   Adapter   │  │   Adapter   │          │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘          │
└─────────┼────────────────┼────────────────┼─────────────────┘
          │                │                │
          └────────────────┼────────────────┘
                           │ HTTP
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                      river-gateway                           │
│  ┌─────────────────────────────────────────────────────┐    │
│  │                    Agent Loop                        │    │
│  │  Sleeping → Waking → Thinking → Acting → Settling   │    │
│  └─────────────────────────────────────────────────────┘    │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │  Tools   │ │  Memory  │ │ Subagents│ │   Git    │       │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
└─────────────────────────────────────────────────────────────┘
          │                │                │
          ▼                ▼                ▼
    ┌──────────┐     ┌──────────┐     ┌──────────────┐
    │  SQLite  │     │  Redis   │     │ Orchestrator │
    │ (persist)│     │ (cache)  │     │ (coordinate) │
    └──────────┘     └──────────┘     └──────────────┘
```

---

## Crate Structure

### river-core

Foundational types and utilities shared across all crates.

**Key Components:**

| Component | Description |
|-----------|-------------|
| `Snowflake` | 128-bit distributed unique ID generator |
| `AgentBirth` | 36-bit packed timestamp identifying agent creation |
| `RiverError` | Unified error type covering all subsystems |
| `ContextStatus` | Token usage tracking with limit awareness |
| `Priority` | Background < Scheduled < Interactive |

**Snowflake ID Structure:**
```
┌─────────────────────────────────────────────────────────────┐
│                        128 bits                              │
├─────────────────────────────┬───────────────────────────────┤
│     high (64 bits)          │        low (64 bits)          │
│  timestamp (microseconds    │  [36] AgentBirth              │
│  since agent birth)         │  [8]  SnowflakeType           │
│                             │  [20] sequence number         │
└─────────────────────────────┴───────────────────────────────┘

SnowflakeType:
  0x01 = Message
  0x02 = Embedding
  0x03 = Session
  0x04 = Subagent
  0x05 = ToolCall
```

### river-gateway

The core agent runtime. Executes the continuous agent loop, manages tools, handles memory, and coordinates subagents.

**Module Structure:**
```
river-gateway/src/
├── main.rs              # CLI entry point
├── server.rs            # Initialization and startup
├── state.rs             # AppState (shared runtime state)
├── api/
│   └── routes.rs        # HTTP API endpoints
├── loop/
│   ├── mod.rs           # AgentLoop state machine
│   ├── state.rs         # LoopState enum
│   ├── context.rs       # ChatMessage, ContextBuilder
│   ├── model.rs         # ModelClient for LLM API
│   └── queue.rs         # Message buffering
├── db/
│   ├── schema.rs        # SQLite migrations
│   ├── messages.rs      # Conversation storage
│   └── memories.rs      # Embedding storage
├── memory/
│   ├── embedding.rs     # Vector generation
│   └── search.rs        # Semantic search
├── redis/
│   ├── working.rs       # Short-term memory (minutes)
│   ├── medium_term.rs   # Session memory (hours)
│   └── coordination.rs  # Locks and counters
├── tools/               # 12 tool categories
├── subagent/            # Child agent management
├── session/             # Session tracking
├── heartbeat.rs         # Orchestrator keep-alive
└── git.rs               # Version control operations
```

### river-orchestrator

Coordination service for multi-agent deployments and model management.

**Responsibilities:**
- Agent registry and health monitoring
- Model discovery (GGUF files on disk)
- Process management (llama-server instances)
- Resource tracking (GPU/VRAM/RAM allocation)
- Dynamic port allocation

### river-discord

Discord protocol adapter using the Twilight library.

**Features:**
- Gateway event handling
- Slash command registration
- Channel-to-session mapping
- Outbound message routing

---

## Core Concepts

### Agent Loop State Machine

The agent operates as a state machine with five phases:

```
┌──────────┐
│ Sleeping │◄─────────────────────────────────┐
└────┬─────┘                                  │
     │ event (message/heartbeat)              │
     ▼                                        │
┌──────────┐                                  │
│  Waking  │ assemble context from DB         │
└────┬─────┘                                  │
     │                                        │
     ▼                                        │
┌──────────┐                                  │
│ Thinking │ call model, get tool_calls       │
└────┬─────┘                                  │
     │                                        │
     ▼                                        │
┌──────────┐                                  │
│  Acting  │ execute tools sequentially       │
└────┬─────┘                                  │
     │                                        │
     ▼                                        │
┌──────────┐                                  │
│ Settling │ commit to DB, send responses     │
└────┬─────┘                                  │
     │                                        │
     └────────────────────────────────────────┘
```

**Message Queuing:** During Thinking and Acting phases, incoming messages are queued and processed in the next cycle.

### Context Management

The agent tracks token usage and automatically handles context rotation:

- **90% threshold:** Triggers rotation warning
- **Automatic rotation:** Summarizes current context, clears, restarts
- **Manual rotation:** `rotate_context` tool available

### Wake Triggers

| Trigger | Source | Description |
|---------|--------|-------------|
| Message | HTTP `/incoming` | User or adapter message |
| Heartbeat | Timer | Scheduled wake for autonomous tasks |

---

## Tool System

### Architecture

```rust
trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;  // JSON Schema
    fn execute(&self, args: Value) -> Result<ToolResult, RiverError>;
}
```

Tools are registered at startup and executed through `ToolExecutor` which tracks context usage.

### Available Tools

| Category | Tools | Description |
|----------|-------|-------------|
| **File** | `read`, `write`, `edit`, `glob`, `grep` | Filesystem operations |
| **Shell** | `bash` | Command execution with timeout |
| **Memory** | `embed`, `memory_search`, `memory_delete`, `memory_delete_by_source` | Semantic memory |
| **Web** | `webfetch`, `websearch` | URL fetching, search |
| **Communication** | `send_message`, `list_adapters`, `read_channel` | Adapter messaging |
| **Model** | `request_model`, `release_model`, `switch_model` | Model management |
| **Scheduling** | `schedule_heartbeat`, `rotate_context` | Loop control |
| **Logging** | `log_read` | System log access |
| **Subagent** | `spawn_subagent`, `list_subagents`, `subagent_status`, `stop_subagent`, `internal_send`, `internal_receive`, `wait_for_subagent` | Child agents |

### Tool Execution Flow

```
Model generates: ToolCall { id, name, arguments }
                         │
                         ▼
              ToolExecutor.execute()
                         │
          ┌──────────────┼──────────────┐
          │              │              │
          ▼              ▼              ▼
     [lookup]      [execute]     [track context]
          │              │              │
          └──────────────┼──────────────┘
                         ▼
              ToolCallResponse { id, result, context_status }
```

---

## Memory Architecture

### Four-Layer Hierarchy

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 1: Long-term Memory (SQLite)                          │
│ - Semantic embeddings with vector search                    │
│ - Permanent storage with optional expiration                │
│ - Source tags for categorization                            │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│ Layer 2: Medium-term Memory (Redis)                         │
│ - Session-level state                                       │
│ - TTL: hours                                                │
│ - Key pattern: river:{agent}:medium:{key}                   │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│ Layer 3: Working Memory (Redis)                             │
│ - Current task state                                        │
│ - TTL: minutes                                              │
│ - Key pattern: river:{agent}:working:{key}                  │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│ Layer 4: Coordination (Redis)                               │
│ - Distributed locks                                         │
│ - Atomic counters                                           │
│ - Multi-agent coordination                                  │
└─────────────────────────────────────────────────────────────┘
```

### Embedding System

```
Text input
    │
    ▼
EmbeddingClient → llama-server /embeddings
    │
    ▼
f32 vector (768 dimensions default)
    │
    ▼
Store in SQLite memories table (BLOB)

Search:
Query → Embedding → Cosine similarity → Top-k results
```

---

## Subagent System

Parent agents can spawn child agents for parallel task execution.

### Types

| Type | Behavior |
|------|----------|
| `TaskWorker` | Executes task, terminates on completion |
| `LongRunning` | Persists, processes multiple requests |

### Lifecycle

```
Parent: spawn_subagent(task, model, type)
                    │
                    ▼
        SubagentManager.register()
                    │
                    ▼
        SubagentRunner spawned (tokio task)
                    │
                    ├─── Independent agent loop
                    ├─── Shared workspace access
                    ├─── Filtered tool registry (no subagent tools)
                    │
                    ▼
        InternalQueue for parent↔child messaging
                    │
                    ├─── internal_send (child → parent)
                    └─── internal_receive (parent ← child)
```

### Communication

- **Parent → Child:** `internal_send` with target subagent ID
- **Child → Parent:** `internal_send` (no target, goes to parent)
- **Wait:** `wait_for_subagent` blocks until TaskWorker completes

---

## Database Schema

### messages

Stores all conversation history.

```sql
CREATE TABLE messages (
    id           BLOB PRIMARY KEY,  -- Snowflake ID
    session_id   TEXT NOT NULL,
    role         TEXT NOT NULL,     -- system|user|assistant|tool
    content      TEXT,
    tool_calls   TEXT,              -- JSON array
    tool_call_id TEXT,
    name         TEXT,
    created_at   INTEGER NOT NULL,
    metadata     TEXT               -- JSON object
);

CREATE INDEX idx_messages_session ON messages(session_id, created_at);
```

### sessions

Tracks agent sessions.

```sql
CREATE TABLE sessions (
    id             TEXT PRIMARY KEY,
    agent_name     TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    last_active    INTEGER NOT NULL,
    context_tokens INTEGER DEFAULT 0,
    metadata       TEXT
);
```

### memories

Stores semantic embeddings.

```sql
CREATE TABLE memories (
    id         BLOB PRIMARY KEY,    -- Snowflake ID
    content    TEXT NOT NULL,
    embedding  BLOB NOT NULL,       -- f32 vector as bytes
    source     TEXT NOT NULL,       -- category tag
    timestamp  INTEGER NOT NULL,
    expires_at INTEGER,             -- optional TTL
    metadata   TEXT
);

CREATE INDEX idx_memories_source ON memories(source);
CREATE INDEX idx_memories_expires ON memories(expires_at);
```

---

## API Reference

### Gateway Endpoints

#### Health Check
```
GET /health

Response: {
    "status": "ok",
    "version": "0.1.0"
}
```

#### Incoming Message
```
POST /incoming
Authorization: Bearer <token>

Request: {
    "adapter": "discord",
    "event_type": "message",
    "channel": "#general",
    "author": {
        "id": "user123",
        "name": "Alice"
    },
    "content": "Hello, agent!",
    "message_id": "msg123",
    "metadata": {}
}

Response: {
    "status": "delivered"
}
```

#### List Tools
```
GET /tools

Response: [
    {
        "name": "read",
        "description": "Read file contents",
        "parameters": { ... }
    },
    ...
]
```

#### Context Status
```
GET /context/status

Response: {
    "used": 45000,
    "limit": 200000,
    "percent": "22.5%"
}
```

---

## Configuration

### Gateway Startup

```bash
river-gateway \
    --workspace /home/agent/workspace \
    --data-dir /var/lib/river \
    --agent-name main-agent \
    --port 3000 \
    --model-url http://localhost:8080 \
    --model-name claude-3-opus \
    --embedding-url http://localhost:8081 \
    --redis-url redis://localhost:6379 \
    --orchestrator-url http://localhost:5000 \
    --auth-token-file /etc/river/token
```

### Orchestrator Startup

```bash
river-orchestrator \
    --port 5000 \
    --model-dirs /models/gguf,/home/models \
    --external-models /etc/river/external-models.json \
    --llama-server-path /usr/local/bin/llama-server \
    --port-range 8080-8180 \
    --reserve-vram-mb 500 \
    --reserve-ram-mb 2000
```

### Discord Adapter Startup

```bash
river-discord \
    --token $DISCORD_BOT_TOKEN \
    --guild-id 123456789 \
    --gateway-url http://localhost:3000 \
    --listen-port 8000 \
    --initial-channels general,ai-chat
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `DISCORD_BOT_TOKEN` | Discord bot authentication token |
| `RUST_LOG` | Logging level (trace, debug, info, warn, error) |

---

## Runtime Behavior

### Guarantees

| Aspect | Guarantee |
|--------|-----------|
| **Message ordering** | Within session, ordered by timestamp |
| **ID uniqueness** | Snowflake IDs globally unique per agent birth |
| **Context safety** | Automatic rotation at 90% threshold |
| **Tool execution** | Sequential (no parallel tool calls) |
| **Durability** | Messages persisted to SQLite immediately |

### Concurrency Model

- `Arc<RwLock<>>` for shared mutable state
- Message queue via `mpsc` channel
- Tools execute sequentially within a cycle
- Multiple gateway instances can share Redis/SQLite

### Graceful Degradation

| Service Down | Behavior |
|--------------|----------|
| Embedding server | Memory tools fail gracefully |
| Redis | Working memory tools fail |
| Orchestrator | Heartbeats fail silently |
| Model server | Thinking phase fails with error |

### Error Handling

All errors flow through `RiverError`:

```rust
pub enum RiverError {
    Config(String),
    Database(String),
    Tool(String),
    Model(String),
    Auth(String),
    Session(String),
    Workspace(String),
    Adapter(String),
    Orchestrator(String),
    Embedding(String),
    Redis(String),
    Serialization(String),
    IO(String),
}
```

---

## Development

### Building

```bash
cargo build --workspace
```

### Testing

```bash
cargo test --workspace
```

### Running Tests for a Single Crate

```bash
cargo test -p river-gateway
```

### Current Test Count

- **river-gateway:** 142 tests
- **river-core:** (core types, minimal tests)
- **river-orchestrator:** (integration tests)
- **river-discord:** (adapter tests)

---

## License

See LICENSE file in repository root.

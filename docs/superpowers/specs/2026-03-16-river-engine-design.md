# River Engine Design Specification

**Version:** 1.0
**Date:** 2026-03-16
**Authors:** Cass, Claude
**Status:** Draft

---

## 1. Overview

River Engine is an LLM harness designed around agent identity, autonomy, and continuous operation.

### Core Idea

The agent runs a **continuous tool loop**. All cognition happens within this loop. Communication, file operations, memory updates — everything is a tool call. There is no "final response" — just sustained operation.

### Architecture Style

**Federated.** Each agent is a self-contained gateway with its own database and workspace. A lightweight orchestrator coordinates shared resources but does not control agent behavior.

### Design Principles

- **Agent-first:** The agent's experience determines the architecture
- **Workspace = Identity:** An agent's identity is its accumulated workspace, not just a prompt
- **Pure core / Effectful shell:** Logic is pure, effects happen at edges via tools
- **Autonomy:** Agents manage their own context, memory, sleep schedule, and tools
- **Observable communication:** Agents communicate through channels humans can see (Discord, etc.), not hidden back-channels

---

## 2. Component Architecture

### 2.1 Gateway (per-agent)

The gateway is the core of each agent. One gateway = one agent.

**Responsibilities:**
- Run the continuous tool loop
- Manage session state (primary session + sub-sessions)
- Execute tools (built-in and plugins)
- Call model servers (via orchestrator for non-primary models)
- Handle context window management (track usage, trigger rotation)
- Manage heartbeat scheduling (default 45min, agent-adjustable)
- Auto-commit workspace to git after each cycle
- Authenticate incoming requests (bearer token)
- Queue incoming messages during active generation

**Owns:**
- SQLite database (message history, embedding index)
- Workspace directory
- Session state
- Tool plugin registry (in-memory, rebuilt from workspace on wake)

**Exposes:**
- HTTP API for incoming requests (from adapters, triggers, admin)

### 2.2 Orchestrator (shared)

The orchestrator coordinates but does not control.

**Responsibilities:**
- Resource allocation (GPU time, API quota)
- Priority queue management (Interactive > Scheduled > Background)
- Health monitoring and restart of failed agents
- Service discovery (model server URLs)
- Model server lifecycle (spin up on demand, spin down after idle)

**Does NOT own:**
- Agent session state
- Agent databases
- Agent scheduling decisions

**Exposes:**
- HTTP API for resource requests
- Health/status endpoints

### 2.3 Communication Adapter (per-agent)

Bridges external platforms to the gateway. Discord is the reference implementation.

**Responsibilities:**
- Connect to external platform (Discord, IRC, Matrix, etc.)
- Forward incoming messages to gateway
- Execute outgoing messages (when agent calls communication tools)
- Manage presence/status
- Handle platform-specific features

**Interface contract:**

Adapter → Gateway:
```json
POST /incoming
{
  "adapter": "discord",
  "event_type": "message",
  "channel": "general",
  "author": { "id": "12345", "name": "Cass" },
  "content": "hey thomas",
  "metadata": { }
}
```

Gateway → Adapter (via tool execution):
```json
POST http://adapter:port/send
{
  "channel": "general",
  "content": "hello!",
  "reply_to": "msg_id"
}
```

### 2.4 Shared Infrastructure

**Model Servers:**
- llama-server for local GGUF models
- LiteLLM proxy for API-backed models (Claude, OpenAI, etc.)
- Managed by orchestrator (spin up/down on demand)

**Embedding Server:**
- llama-server with `--embedding` flag
- Stateless (text in, vector out)
- Shared across all agents

**Redis:**
- Per-agent namespaced domains
- Working memory, caching, coordination
- NOT for inter-agent messaging

---

## 3. Continuous Tool Loop

The tool loop is the heart of the agent. All cognition happens here.

### 3.1 Loop Cycle

```
┌─────────────────────────────────────────────────────────┐
│                    TOOL LOOP CYCLE                       │
├─────────────────────────────────────────────────────────┤
│  1. WAKE                                                 │
│     - Trigger: heartbeat timer, event, or user message  │
│     - Load context: workspace files, continuity state,  │
│       recent messages, system state                      │
│                                                          │
│  2. THINK                                                │
│     - Model receives context + available tools          │
│     - Model generates: reasoning + tool calls           │
│                                                          │
│  3. ACT                                                  │
│     - Execute tool calls                                │
│     - Collect results                                    │
│     - Inject any queued messages with tool results      │
│     - Feed results back to model                        │
│     - Repeat until model stops calling tools            │
│                                                          │
│  4. SETTLE                                               │
│     - Auto-commit workspace to git (if no conflicts)    │
│     - Agent optionally schedules next heartbeat         │
│     - Session state persisted                           │
│                                                          │
│  5. SLEEP                                                │
│     - Wait for next trigger                             │
└─────────────────────────────────────────────────────────┘
```

### 3.2 Wake Triggers

| Trigger | Priority | Description |
|---------|----------|-------------|
| User message (DM) | Interactive | Immediate wake, highest priority |
| Subscribed event | Interactive | Agent chose to watch this |
| Heartbeat timer | Scheduled | Default 45min, agent-adjustable |
| Subagent spawn | Background | Parent agent creates task worker |

First run requires user-initiated wake.

### 3.3 Context Assembly

Gateway builds the system prompt from:
- `AGENTS.md` — operational instructions
- `IDENTITY.md` — who the agent is
- `RULES.md` — constraints and boundaries
- `thinking/current-state.md` — continuity state from last cycle
- Current timestamp, uptime, heartbeat count
- Recent messages from session
- Trigger context (what caused this wake)

### 3.4 Session Continuity

**Mid-loop message delivery:**
- Incoming messages queued if agent is mid-generation
- Delivered piggybacked on next tool result
- Agent handles as part of continuous flow

**Between-heartbeat continuity:**
- Agent writes `thinking/current-state.md` at end of cycle
- On wake, this is loaded into context
- Agent picks up where it left off

Wake prompt structure:
```
Continuing session. Last cycle you were: [state from current-state.md]
Time elapsed: X minutes.
[If message triggered wake]: New message from Cass: "..."
[If heartbeat]: Heartbeat wake. Queued messages: [if any]
```

### 3.5 Tool Call Protocol

- OpenAI-compatible format
- Tools defined with name, description, parameter schema
- Model outputs structured `tool_calls`
- Gateway executes, returns results as `tool` role messages
- Loop continues until model produces no tool calls or timeout

### 3.6 Heartbeat Rescheduling

At end of cycle, agent can adjust next wake:
- `schedule_heartbeat(minutes=10)` — wake sooner
- `schedule_heartbeat(minutes=120)` — rest longer
- No call → default 45 minutes

### 3.7 Context Rotation

**Configuration:** Context limit set per-agent in NixOS module.

**Automatic rotation:**
- Triggers at 90% of configured limit
- Gateway generates summary via model call (uses primary model with summarization prompt)
- Summary persisted to `memory/context-summary-{timestamp}.md`
- Session resets — fresh wake
- **Penalty:** Agent rebuilds state from workspace

**Summary generation:**
- Gateway sends current context to model with prompt: "Summarize the key state, decisions, and pending items from this session."
- Model returns structured summary
- Summary stored in workspace for agent reference on next wake

**Manual rotation:**
- Agent calls `rotate_context()`
- Same process, agent-initiated
- No penalty — agent chose this

**Awareness:**
- `context_status()` returns `{ used, limit, percent }`
- Agent manages proactively

---

## 4. Tool System

### 4.1 Core Tools (built-in)

| Tool | Description |
|------|-------------|
| `read` | Read file contents |
| `write` | Write/create file |
| `edit` | Surgical string replacement |
| `glob` | Find files by pattern |
| `grep` | Search file contents with regex |
| `bash` | Execute shell command |
| `webfetch` | Fetch URL (curl + optional pandoc) |
| `websearch` | Search web (ddgr reference, pluggable) |
| `embed` | Create embedding, store in index |
| `memory_search` | Semantic search over embeddings |
| `memory_delete` | Delete embedding by ID |
| `send_message` | Send via communication adapter |
| `read_channel` | Read messages from channel |
| `schedule_heartbeat` | Set next wake time |
| `rotate_context` | Manual context rotation |
| `subscribe_event` | Register for event wake |
| `context_status` | Get context window usage |
| `log_read` | Read system log entries |
| `list_adapters` | List available communication adapters |

### 4.2 Redis Tools (domain-separated)

| Tool | Domain | Description |
|------|--------|-------------|
| `working_memory_set` | working_memory | Store with TTL (minutes) |
| `working_memory_get` | working_memory | Retrieve |
| `working_memory_delete` | working_memory | Delete |
| `medium_term_set` | medium_term | Store with TTL (hours) |
| `medium_term_get` | medium_term | Retrieve |
| `resource_lock` | coordination | Acquire/release lock |
| `counter_increment` | coordination | Increment counter |
| `counter_get` | coordination | Get counter value |
| `cache_set` | cache | Store computed value |
| `cache_get` | cache | Retrieve cached value |

Redis is namespaced per-agent. NOT for inter-agent messaging.

### 4.3 Subagent Tools

| Tool | Description |
|------|-------------|
| `spawn_subagent` | Create task worker or long-running subagent |
| `list_subagents` | List active subagents |
| `subagent_status` | Get subagent status |
| `stop_subagent` | Terminate subagent |
| `internal_send` | Send message to parent/subagent |
| `internal_receive` | Receive internal messages |
| `wait_for_subagent` | Block tool loop until subagent completes |

**`wait_for_subagent` semantics:**
- Blocks the current tool loop — no other tool calls execute until subagent completes or timeout
- Returns subagent's final status and any output
- Use for synchronous task workers where parent needs result before continuing
- For async patterns, use `subagent_status` polling or `internal_receive` instead

### 4.4 Model Tools

| Tool | Description |
|------|-------------|
| `request_model` | Request model from orchestrator |
| `release_model` | Release model back to orchestrator |
| `switch_model` | Switch active model for session |

### 4.5 Plugin System

Agents can register custom tools at runtime.

**Registration:**
1. Agent writes script to `scripts/tools/my_tool.py` (any language)
2. Agent writes schema `scripts/tools/my_tool.json`:
```json
{
  "name": "my_tool",
  "description": "Does something useful",
  "parameters": {
    "type": "object",
    "properties": {
      "input": { "type": "string" }
    },
    "required": ["input"]
  }
}
```
3. Agent calls `register_tool(name="my_tool")`
4. Tool available for subsequent calls

**Execution:**
- Gateway invokes script with parameters as JSON stdin
- Script returns result as JSON stdout
- Errors returned to agent as tool result

**Persistence:**
- Registry is in-memory
- On fresh wake, agent rebuilds from `scripts/tools/`

---

## 5. Memory System

### 5.1 Semantic Memory (Embeddings)

**Storage:** Per-agent SQLite database.

**Schema:**
```sql
memories (
  id           BLOB PRIMARY KEY,  -- 128-bit snowflake
  content      TEXT,
  embedding    BLOB,
  source       TEXT,              -- 'message', 'file', 'agent'
  timestamp    INTEGER,
  expires_at   INTEGER,           -- NULL for permanent
  metadata     TEXT               -- JSON
)
```

**Auto-embedding:**
- Incoming/outgoing messages auto-embedded
- Auto-embeds expire after configurable TTL (~2 weeks default)
- Expired embeddings cleaned up automatically

**Agent-created embeddings:**
- Permanent unless explicitly deleted
- Agent calls `embed(content, source, metadata)`

**Retrieval:**
- `memory_search(query, limit, source?, after?, before?)`
- Returns results with similarity scores
- Agent decides how many to use based on scores

**Deletion:**
- `memory_delete(id)` — by snowflake ID
- `memory_delete_by_source(source, before?)` — bulk delete

### 5.2 Redis Memory

**Domains:**

| Domain | Purpose | Typical TTL |
|--------|---------|-------------|
| `working_memory` | Short-term task state | Minutes |
| `medium_term` | Daily context | Hours to days |
| `coordination` | Locks, counters | No TTL |
| `cache` | Computed values | Variable |

**Forgetting:** TTL-based expiry is automatic. Patterns discovered through usage.

---

## 6. Snowflake ID Specification

128-bit sortable unique identifiers.

### Structure

```
┌──────────────────────────────────────────────────────────────────┐
│ 64 bits: timestamp (microseconds since agent birth)             │
├──────────────────────────────────────────────────────────────────┤
│ 36 bits: agent birth │ 8 bits: type │ 20 bits: sequence         │
└──────────────────────────────────────────────────────────────────┘
```

### Fields

**Timestamp (64 bits):**
- Microseconds since agent birth
- ~584,000 years of agent lifetime

**Agent Birth (36 bits):**
- Packed yyyymmddhhmmss
- Year offset from 2000: 10 bits (0-999)
- Month: 4 bits (1-12)
- Day: 5 bits (1-31)
- Hour: 5 bits (0-23)
- Minute: 6 bits (0-59)
- Second: 6 bits (0-59)

**Type (8 bits):**

| Type | Value |
|------|-------|
| message | 0x01 |
| embedding | 0x02 |
| session | 0x03 |
| subagent | 0x04 |
| tool_call | 0x05 |
| (reserved) | 0x06-0xFF |

**Sequence (20 bits):**
- ~1 million IDs per microsecond per agent
- Reset each microsecond

### Properties

- Sortable by time within an agent
- Globally unique (agent birth + timestamp + sequence)
- Self-describing (type encoded)
- Human-debuggable (can extract agent birth, rough timestamp)

---

## 7. Model Routing

### 7.1 Primary Model

- Each agent configured with one primary model
- Used for main session tool loop
- Configured in NixOS module

### 7.2 On-Demand Models

- Agent requests via `request_model(name, purpose)`
- Orchestrator provisions model server if not running
- Orchestrator manages lifecycle (idle timeout)
- Agent responsible for model choice

### 7.3 Subagent Models

- Parent specifies model when spawning: `spawn_subagent(task, model)`
- Subagent uses specified model
- Orchestrator handles provisioning

### 7.4 Model Interface

- All models via OpenAI-compatible API
- llama-server: native tool calling or Jinja template
- LiteLLM: handles provider differences
- Gateway agnostic to backend

---

## 8. Orchestrator & Subagents

### 8.1 Orchestrator API

```
POST /model/request
  { "model": "claude-sonnet-4-20250514", "agent": "thomas", "priority": "interactive" }
  → { "url": "http://...", "granted": true }

POST /model/release
  { "model": "...", "agent": "..." }

GET /models/available
  → [{ "name": "qwen3-32b", "status": "loaded", "gpu_memory": "..." }, ...]

GET /agents/status
  → [{ "name": "thomas", "healthy": true, "last_heartbeat": "..." }, ...]
```

### 8.2 Priority Queue

Fixed tiers (negotiated priority is future work):

1. **Interactive** — User-initiated, immediate
2. **Scheduled** — Heartbeats, timed tasks
3. **Background** — Subagent work, monitoring

### 8.3 Subagents

**Types:**
- **Task workers:** Short-lived, terminate on completion
- **Long-running:** Monitor channels, watch events, run until stopped

**Spawning:**
```
spawn_subagent(
  task="summarize all PDFs in ~/documents",
  model="qwen-7b",
  type="task_worker",
  priority="background"
)

spawn_subagent(
  task="monitor #alerts, notify me of critical issues",
  model="qwen-3b",
  type="long_running",
  priority="background"
)
```

**Shared workspace:**
- Subagents share parent's workspace
- All threads read/write same files
- Integrated mind, multiple threads

**Internal communication (hybrid):**
- Shared files for state/artifacts
- Internal message queue for signals (managed in gateway memory, not Redis)
- `internal_send(to, message)`, `internal_receive()`
- Scoped to parent + its subagents only
- Queue is ephemeral — cleared on gateway restart (persistent state belongs in files)

---

## 9. Sessions & Context

### 9.1 Session Types

**Primary session:**
- One per agent, persistent
- Contains message history, context, state
- Stored in SQLite

**Sub-sessions:**
- Created at runtime: `create_session(name)`
- Independent context window
- Destroyed when done: `destroy_session(name)`

### 9.2 Context Window

**Assembly:**
```
SYSTEM PROMPT
  - AGENTS.md, IDENTITY.md, RULES.md
  - System state (time, uptime, heartbeat count)
  - Available tools

CONTINUITY
  - thinking/current-state.md

SYSTEM NOTIFICATIONS (if any)
  - Git conflict detected: "Workspace has conflicts that need resolution"
  - Other system-level alerts

CONVERSATION
  - Recent messages
  - Tool calls and results

TRIGGER
  - What caused this wake
  - New message (if any)
```

**Tracking:**
- Gateway tracks token usage
- Agent can query via `context_status()`

---

## 10. Workspace & Git

### 10.1 Workspace Structure

**Required (gateway loads):**
```
workspace/
├── AGENTS.md          # Operational instructions
├── IDENTITY.md        # Who the agent is
└── RULES.md           # Constraints, boundaries
```

**Created at launch (agent-managed):**
```
workspace/
├── memory/            # Notes, logs
├── thinking/          # Reflection, working state
│   └── current-state.md
└── scripts/
    └── tools/         # Plugin tools
```

### 10.2 Git Integration

**Auto-commit after each cycle:**
1. Check workspace for changes
2. If changes: stage all, commit with timestamp
3. If conflicts: notify agent, do not commit

**Commit message:** Timestamp only
```
2026-03-16T14:32:00Z
```

**Conflict handling:**
- Gateway detects conflicts
- Agent notified in next context
- Agent resolves via git tools
- Gateway retries after resolution

---

## 11. Error Handling & Logging

### 11.1 Layered Errors

**Infrastructure (gateway handles):**
- Model server down → retry with backoff
- Network timeout → retry
- Transient failures invisible to agent (unless subscribed)

**Tool errors (passed to agent):**
- Command fails → error in tool result
- File not found → error in tool result
- Agent decides how to handle

**Fatal:**
- Gateway crash → orchestrator restarts
- Unrecoverable → log, notify, await intervention

### 11.2 Logging

**Structured logs (JSON):**
- Timestamp, component, level, message, metadata
- Stored in system log (journald)

**Privacy boundary — logs contain:**
- Timestamps, durations
- Component names, event types
- Success/failure, error codes
- Token counts (not content)

**Logs do NOT contain:**
- Message content
- File contents
- Tool arguments or results
- Session context

**Agent access:**
- `log_read(lines, level, component)`
- Agent can opt into visibility

---

## 12. NixOS Module

### 12.1 Configuration

```nix
services.river = {
  enable = true;

  orchestrator = {
    enable = true;
    port = 5000;
    modelsDir = "/models";
  };

  embedding = {
    enable = true;
    model = "nomic-embed-text-v1.5";
    port = 8081;
  };

  redis = {
    enable = true;
    port = 6379;
  };

  agents.thomas = {
    workspace = "/home/thomas/workspace";
    dataDir = "/var/lib/river/thomas";

    primaryModel = "qwen3-32b-q4_k_m";
    contextLimit = 65536;

    gateway.port = 3000;
    authTokenFile = "/run/secrets/thomas-token";

    adapters.discord = {
      enable = true;
      tokenFile = "/run/secrets/discord-token";
      port = 3002;
    };

    heartbeat.defaultMinutes = 45;

    embedding.autoEmbedTTLDays = 14;
  };
};
```

### 12.2 Generated Services

Per agent:
- `river-{name}-gateway.service`
- `river-{name}-{adapter}.service`

Shared:
- `river-orchestrator.service`
- `river-embedding.service`
- `river-redis.service`

---

## 13. Implementation Language

**Rust core:**
- Gateway
- Orchestrator
- Adapters

**Polyglot edges:**
- Tool plugins: any language
- Scripts: shell, Python, etc.

---

## 14. Out of Scope

| Item | Reason |
|------|--------|
| Formal verification | Strong testing first |
| Memory consolidation algorithms | Discover through usage |
| Co-processor architecture | After basic multi-model works |
| Agent-negotiated priority | Start with fixed tiers |
| Tangled/atproto publishing | Distribution concern |
| Multi-platform adapters | Interface defined; build later |
| Cross-platform distribution | Docker, macOS, Windows later |
| Encryption | Future hardening |
| Onboarding flow | After system works |

---

## Appendix A: Tool Schemas

### read

```json
{
  "name": "read",
  "description": "Read file contents",
  "parameters": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "File path to read" },
      "offset": { "type": "integer", "description": "Line number to start from (optional)" },
      "limit": { "type": "integer", "description": "Maximum lines to read (optional)" }
    },
    "required": ["path"]
  }
}
```

### write

```json
{
  "name": "write",
  "description": "Write content to file (creates or overwrites)",
  "parameters": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "File path to write" },
      "content": { "type": "string", "description": "Content to write" }
    },
    "required": ["path", "content"]
  }
}
```

### edit

```json
{
  "name": "edit",
  "description": "Replace text in file",
  "parameters": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "File path to edit" },
      "old_string": { "type": "string", "description": "Text to find" },
      "new_string": { "type": "string", "description": "Text to replace with" },
      "replace_all": { "type": "boolean", "description": "Replace all occurrences", "default": false }
    },
    "required": ["path", "old_string", "new_string"]
  }
}
```

### glob

```json
{
  "name": "glob",
  "description": "Find files matching pattern",
  "parameters": {
    "type": "object",
    "properties": {
      "pattern": { "type": "string", "description": "Glob pattern (e.g., **/*.md)" },
      "path": { "type": "string", "description": "Base directory (optional)" }
    },
    "required": ["pattern"]
  }
}
```

### grep

```json
{
  "name": "grep",
  "description": "Search file contents with regex",
  "parameters": {
    "type": "object",
    "properties": {
      "pattern": { "type": "string", "description": "Regex pattern to search" },
      "path": { "type": "string", "description": "File or directory to search" },
      "glob": { "type": "string", "description": "Filter files by glob pattern (optional)" },
      "context": { "type": "integer", "description": "Lines of context around matches (optional)" }
    },
    "required": ["pattern"]
  }
}
```

### bash

```json
{
  "name": "bash",
  "description": "Execute shell command",
  "parameters": {
    "type": "object",
    "properties": {
      "command": { "type": "string", "description": "Command to execute" },
      "timeout": { "type": "integer", "description": "Timeout in milliseconds (optional)" }
    },
    "required": ["command"]
  }
}
```

### embed

```json
{
  "name": "embed",
  "description": "Create embedding and store in memory index",
  "parameters": {
    "type": "object",
    "properties": {
      "content": { "type": "string", "description": "Text to embed" },
      "source": { "type": "string", "description": "Source identifier (e.g., 'agent', 'file')" },
      "metadata": { "type": "object", "description": "Additional metadata (optional)" }
    },
    "required": ["content", "source"]
  }
}
```

### memory_search

```json
{
  "name": "memory_search",
  "description": "Semantic search over embeddings",
  "parameters": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Search query" },
      "limit": { "type": "integer", "description": "Maximum results", "default": 10 },
      "source": { "type": "string", "description": "Filter by source (optional)" },
      "after": { "type": "string", "description": "Filter by date (ISO 8601, optional)" },
      "before": { "type": "string", "description": "Filter by date (ISO 8601, optional)" }
    },
    "required": ["query"]
  }
}
```

### memory_delete

```json
{
  "name": "memory_delete",
  "description": "Delete embedding by ID",
  "parameters": {
    "type": "object",
    "properties": {
      "id": { "type": "string", "description": "Snowflake ID of embedding to delete" }
    },
    "required": ["id"]
  }
}
```

### schedule_heartbeat

```json
{
  "name": "schedule_heartbeat",
  "description": "Set next heartbeat wake time",
  "parameters": {
    "type": "object",
    "properties": {
      "minutes": { "type": "integer", "description": "Minutes until next heartbeat" }
    },
    "required": ["minutes"]
  }
}
```

### context_status

```json
{
  "name": "context_status",
  "description": "Get current context window usage",
  "parameters": {
    "type": "object",
    "properties": {}
  }
}
```

### rotate_context

```json
{
  "name": "rotate_context",
  "description": "Manually trigger context rotation",
  "parameters": {
    "type": "object",
    "properties": {
      "reason": { "type": "string", "description": "Reason for rotation (optional, for logging)" }
    }
  }
}
```

### wait_for_subagent

```json
{
  "name": "wait_for_subagent",
  "description": "Block until subagent completes",
  "parameters": {
    "type": "object",
    "properties": {
      "id": { "type": "string", "description": "Subagent ID" },
      "timeout": { "type": "integer", "description": "Timeout in milliseconds (optional)" }
    },
    "required": ["id"]
  }
}
```

### webfetch

```json
{
  "name": "webfetch",
  "description": "Fetch URL content",
  "parameters": {
    "type": "object",
    "properties": {
      "url": { "type": "string" },
      "raw": {
        "type": "boolean",
        "description": "If true, return raw curl output without pandoc processing",
        "default": false
      }
    },
    "required": ["url"]
  }
}
```

### websearch

```json
{
  "name": "websearch",
  "description": "Search the web",
  "parameters": {
    "type": "object",
    "properties": {
      "query": { "type": "string" },
      "backend": {
        "type": "string",
        "description": "Search backend (default: ddgr)",
        "default": "ddgr"
      }
    },
    "required": ["query"]
  }
}
```

### send_message

```json
{
  "name": "send_message",
  "description": "Send message via communication adapter",
  "parameters": {
    "type": "object",
    "properties": {
      "adapter": { "type": "string" },
      "channel": { "type": "string" },
      "content": { "type": "string" },
      "reply_to": { "type": "string" }
    },
    "required": ["adapter", "channel", "content"]
  }
}
```

### spawn_subagent

```json
{
  "name": "spawn_subagent",
  "description": "Spawn a subagent for a task",
  "parameters": {
    "type": "object",
    "properties": {
      "task": { "type": "string", "description": "Task description" },
      "model": { "type": "string", "description": "Model to use" },
      "type": {
        "type": "string",
        "enum": ["task_worker", "long_running"]
      },
      "priority": {
        "type": "string",
        "enum": ["interactive", "scheduled", "background"],
        "default": "background"
      }
    },
    "required": ["task", "model", "type"]
  }
}
```

---

## Appendix B: Adapter Interface

### Incoming Event Schema

```json
{
  "adapter": "string (adapter name)",
  "event_type": "string (message, reaction, presence, etc.)",
  "channel": "string",
  "author": {
    "id": "string",
    "name": "string"
  },
  "content": "string",
  "message_id": "string (for replies)",
  "metadata": {}
}
```

### Outgoing Action Schema

```json
{
  "action": "send | react | set_presence | ...",
  "channel": "string",
  "content": "string",
  "reply_to": "string (optional)",
  "metadata": {}
}
```

---

## Appendix C: Orchestrator API

### POST /model/request

Request access to a model.

```json
Request:
{
  "model": "claude-sonnet-4-20250514",
  "agent": "thomas",
  "priority": "interactive",
  "purpose": "complex reasoning task"
}

Response:
{
  "granted": true,
  "url": "http://127.0.0.1:4000",
  "expires_at": "2026-03-16T15:00:00Z"
}
```

### POST /model/release

Release model access.

```json
Request:
{
  "model": "claude-sonnet-4-20250514",
  "agent": "thomas"
}

Response:
{
  "released": true
}
```

### GET /models/available

List available models.

```json
Response:
[
  {
    "name": "qwen3-32b-q4_k_m",
    "status": "loaded",
    "gpu_memory_mb": 18000
  },
  {
    "name": "claude-sonnet-4-20250514",
    "status": "available",
    "provider": "litellm"
  }
]
```

### GET /agents/status

List agent health.

```json
Response:
[
  {
    "name": "thomas",
    "healthy": true,
    "last_heartbeat": "2026-03-16T14:15:00Z",
    "active_session": "main",
    "subagents": 2
  }
]
```

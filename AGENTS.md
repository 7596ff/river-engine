# River Engine

A modular runtime for AI agents with persistent memory, model orchestration, and platform adapters.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         Orchestrator                             │
│  - Model discovery (GGUF scanning)                               │
│  - GPU/CPU resource tracking                                     │
│  - llama-server lifecycle management                             │
│  - Agent health monitoring                                       │
└─────────────────────────────────────────────────────────────────┘
        │                           │
        │ heartbeat                 │ model request
        ▼                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                          Gateway                                 │
│  - Message storage (SQLite)                                      │
│  - Tool execution (read, write, bash, glob, grep, list)          │
│  - Semantic memory (embeddings)                                  │
│  - Ephemeral memory (Redis)                                      │
└─────────────────────────────────────────────────────────────────┘
        │
        │ /incoming, /send
        ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Adapters                                 │
│  - Discord (Twilight-based)                                      │
│  - More coming: Slack, Matrix, CLI, Web...                       │
└─────────────────────────────────────────────────────────────────┘
```

## Building

```bash
# Build all binaries
cargo build --release

# Run tests
cargo test

# Binaries are in target/release/
ls target/release/river-*
```

## Components

### 1. Orchestrator (`river-orchestrator`)

Central coordination service for model management and agent health.

**CLI Options:**

```
river-orchestrator [OPTIONS]

Options:
  -p, --port <PORT>                 Port to listen on [default: 5000]
      --health-threshold <SECS>     Health threshold in seconds [default: 120]
      --model-dirs <DIRS>           Directories to scan for GGUF models (comma-separated)
      --external-models <PATH>      Path to external models config JSON
      --models-config <PATH>        Path to legacy models config JSON
      --idle-timeout <SECS>         Idle timeout before unloading models [default: 900]
      --llama-server-path <PATH>    Path to llama-server binary [default: llama-server]
      --port-range <RANGE>          Port range for llama-server [default: 8080-8180]
      --reserve-vram-mb <MB>        Reserved VRAM in MB [default: 500]
      --reserve-ram-mb <MB>         Reserved RAM in MB [default: 2000]
```

**API Endpoints:**

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check |
| `/heartbeat` | POST | Register agent heartbeat |
| `/agents/status` | GET | List all agents and their health |
| `/models/available` | GET | List available models (local + external) |
| `/model/request` | POST | Request a model (blocks until loaded) |
| `/model/release` | POST | Mark model as releasable for eviction |
| `/resources` | GET | GPU/CPU resource status |

**Example - Start orchestrator with local models:**

```bash
river-orchestrator \
  --port 5000 \
  --model-dirs /data/models/gguf \
  --llama-server-path /usr/bin/llama-server
```

**External Models Config (`external-models.json`):**

```json
{
  "models": [
    {
      "id": "gpt-4",
      "provider": "openai",
      "api_key_env": "OPENAI_API_KEY"
    },
    {
      "id": "claude-3",
      "provider": "anthropic",
      "endpoint": "https://api.anthropic.com/v1"
    }
  ]
}
```

---

### 2. Gateway (`river-gateway`)

Agent runtime with message storage, tools, and memory systems.

**CLI Options:**

```
river-gateway [OPTIONS]

Options:
  -w, --workspace <PATH>           Workspace directory (required)
  -d, --data-dir <PATH>            Data directory for database (required)
  -p, --port <PORT>                Gateway port [default: 3000]
      --agent-name <NAME>          Agent name for Redis namespacing [default: default]
      --model-url <URL>            Model server URL (e.g., http://localhost:8080/v1)
      --model-name <NAME>          Model name
      --embedding-url <URL>        Embedding server URL (enables semantic memory)
      --redis-url <URL>            Redis URL (enables ephemeral memory)
      --orchestrator-url <URL>     Orchestrator URL (enables heartbeats)
```

**API Endpoints:**

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check |
| `/incoming` | POST | Receive messages from adapters |
| `/tools` | GET | List available tools |
| `/context/status` | GET | Current context status |

**Example - Start gateway with full features:**

```bash
river-gateway \
  --workspace ~/agents/myagent/workspace \
  --data-dir ~/.local/share/river/myagent \
  --agent-name myagent \
  --port 3000 \
  --orchestrator-url http://localhost:5000 \
  --embedding-url http://localhost:8200/v1 \
  --redis-url redis://localhost:6379
```

**Built-in Tools:**

| Tool | Description |
|------|-------------|
| `read` | Read file contents |
| `write` | Write content to file |
| `bash` | Execute shell commands |
| `glob` | Find files by pattern |
| `grep` | Search file contents |
| `list` | List directory contents |
| `embed` | Store text with embedding |
| `memory_search` | Semantic memory search |
| `memory_delete` | Delete memory by ID |
| `memory_delete_by_source` | Delete memories by source |
| `working_memory_*` | Short-term Redis memory (minutes TTL) |
| `medium_term_*` | Medium-term Redis memory (hours TTL) |
| `cache_*` | Redis cache operations |
| `coordination_*` | Distributed locks and counters |

---

### 3. Discord Adapter (`river-discord`)

Routes messages between Discord and a gateway instance.

**CLI Options:**

```
river-discord [OPTIONS]

Options:
      --token-file <PATH>          Discord bot token file (required)
      --gateway-url <URL>          River gateway URL [default: http://localhost:3000]
      --listen-port <PORT>         Adapter HTTP server port [default: 3002]
      --guild-id <ID>              Guild ID for slash commands (required)
      --channels <IDS>             Initial channel IDs (comma-separated)
      --state-file <PATH>          State file for channel persistence
```

**API Endpoints:**

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check with Discord/gateway status |
| `/send` | POST | Send message to Discord (called by gateway) |
| `/channels` | GET | List monitored channels |
| `/channels` | POST | Add channel to monitor |
| `/channels/{id}` | DELETE | Remove channel from monitoring |

**Slash Commands:**

| Command | Description |
|---------|-------------|
| `/listen #channel` | Add channel to listen set (admin only) |
| `/unlisten #channel` | Remove channel from listen set (admin only) |
| `/channels` | List currently monitored channels |

**Example - Start Discord adapter:**

```bash
# Create token file
echo "your-bot-token" > ~/.config/river/discord-token
chmod 600 ~/.config/river/discord-token

# Start adapter
river-discord \
  --token-file ~/.config/river/discord-token \
  --gateway-url http://localhost:3000 \
  --guild-id 123456789012345678 \
  --state-file ~/.local/share/river/discord-channels.json
```

**Discord Bot Setup:**

1. Create application at https://discord.com/developers/applications
2. Create bot and copy token
3. Enable these intents in Bot settings:
   - Server Members Intent
   - Message Content Intent
4. Generate OAuth2 URL with scopes: `bot`, `applications.commands`
5. Required bot permissions: Send Messages, Read Message History, Add Reactions

---

## Deployment

### Manual Deployment

```bash
# Terminal 1: Orchestrator
river-orchestrator --model-dirs /data/models

# Terminal 2: Embedding server (llama.cpp)
llama-server --embedding --model /data/models/nomic-embed.gguf --port 8200

# Terminal 3: Redis
redis-server --port 6379

# Terminal 4: Gateway
river-gateway \
  --workspace ~/agent/workspace \
  --data-dir ~/.local/share/river/agent \
  --orchestrator-url http://localhost:5000 \
  --embedding-url http://localhost:8200/v1 \
  --redis-url redis://localhost:6379

# Terminal 5: Discord adapter
river-discord \
  --token-file ~/.config/river/discord-token \
  --gateway-url http://localhost:3000 \
  --guild-id 123456789
```

### NixOS Deployment

```nix
# configuration.nix
{ config, pkgs, ... }:
{
  imports = [ /path/to/river-engine/nix/nixos-module.nix ];

  services.river = {
    orchestrator = {
      enable = true;
      modelDirs = [ /data/models ];
      cudaSupport = true;  # Enable CUDA for GPU inference
    };

    embedding = {
      enable = true;
      modelPath = /data/models/nomic-embed-text-v1.5.Q8_0.gguf;
      cudaSupport = true;
    };

    redis.enable = true;

    agents.myagent = {
      enable = true;
      workspace = /srv/river/myagent;
      dataDir = /var/lib/river/myagent;
      orchestratorUrl = "http://localhost:5000";
      embeddingUrl = "http://localhost:8200/v1";
      redisUrl = "redis://localhost:6379";

      discord = {
        enable = true;
        tokenFile = /run/secrets/discord-token;  # Use agenix/sops-nix
        guildId = 123456789012345678;
      };
    };
  };
}
```

### Home-Manager Deployment

```nix
# home.nix
{ config, pkgs, ... }:
{
  imports = [ /path/to/river-engine/nix/home-module.nix ];

  services.river = {
    orchestrator = {
      enable = true;
      modelDirs = [ "${config.home.homeDirectory}/models" ];
    };

    embedding = {
      enable = true;
      modelPath = "${config.home.homeDirectory}/models/nomic-embed.gguf";
    };

    redis.enable = true;

    agents.personal = {
      enable = true;
      workspace = "${config.home.homeDirectory}/agent/workspace";
      dataDir = "${config.xdg.dataHome}/river/personal";
      orchestratorUrl = "http://localhost:5000";
      embeddingUrl = "http://localhost:8200/v1";
      redisUrl = "redis://localhost:6379";
    };
  };
}
```

---

## Message Flow

### Inbound (User → Agent)

```
Discord Message
    ↓
Discord Adapter (filter by channel)
    ↓ POST /incoming
Gateway (store message, queue for processing)
    ↓
Agent processes with tools
    ↓
Response generated
```

### Outbound (Agent → User)

```
Agent calls send_message tool
    ↓
Gateway POST /send to adapter
    ↓
Discord Adapter sends via Twilight
    ↓
Discord Message appears
```

### Incoming Event Format

```json
{
  "adapter": "discord",
  "event_type": "message",
  "channel": "123456789",
  "author": {
    "id": "987654321",
    "name": "username"
  },
  "content": "Hello agent!",
  "message_id": "111222333",
  "metadata": {
    "guild_id": "444555666",
    "thread_id": null,
    "reply_to": null
  }
}
```

### Outbound Message Format

```json
{
  "channel": "123456789",
  "content": "Hello human!",
  "reply_to": "111222333",
  "thread_id": null,
  "create_thread": null,
  "reaction": null
}
```

---

## Memory Systems

### Semantic Memory (SQLite + Embeddings)

Long-term memory with vector similarity search.

```
# Store memory
embed(text="User prefers dark mode", source="preferences")

# Search memory
memory_search(query="user preferences", limit=5)

# Delete memory
memory_delete(id="abc123")
memory_delete_by_source(source="preferences")
```

### Ephemeral Memory (Redis)

Short-term memory with TTL.

**Working Memory** (minutes):
```
working_memory_set(key="current_task", value="...", ttl_minutes=30)
working_memory_get(key="current_task")
```

**Medium-Term Memory** (hours):
```
medium_term_set(key="session_context", value="...", ttl_hours=4)
medium_term_get(key="session_context")
```

**Coordination**:
```
coordination_lock(key="resource", ttl_seconds=60)
coordination_unlock(key="resource")
coordination_increment(key="counter")
```

**Cache**:
```
cache_set(key="api_result", value="...", ttl_seconds=300)
cache_get(key="api_result")
```

---

## Model Management

The orchestrator automatically:

1. **Discovers models** by scanning `--model-dirs` for GGUF files
2. **Parses GGUF headers** to extract architecture, parameters, quantization
3. **Estimates VRAM** requirements from file size + KV cache overhead
4. **Tracks GPU/CPU resources** via nvidia-smi and /proc/meminfo
5. **Spawns llama-server** instances on demand with appropriate device
6. **Evicts idle models** after `--idle-timeout` seconds
7. **Falls back to CPU** when GPU memory is insufficient

**Request a model:**
```bash
curl -X POST http://localhost:5000/model/request \
  -H "Content-Type: application/json" \
  -d '{"model_id": "qwen3-32b-q4_k_m"}'
```

**Response:**
```json
{
  "status": "ready",
  "endpoint": "http://127.0.0.1:8080/v1/chat/completions",
  "device": "gpu:0",
  "warning": null
}
```

---

## Troubleshooting

### Gateway won't start

- Check that `--workspace` and `--data-dir` directories exist
- Verify Redis is running if `--redis-url` is specified
- Check logs: `journalctl -u river-myagent-gateway -f`

### Discord adapter can't connect

- Verify token file exists and is readable
- Check bot has required intents enabled in Discord Developer Portal
- Verify guild ID is correct (enable Developer Mode to copy IDs)

### Models not loading

- Check `--llama-server-path` points to valid binary
- Verify GGUF files are not corrupted: `file /path/to/model.gguf`
- Check GPU memory: `nvidia-smi`
- Review orchestrator logs for VRAM estimation

### Memory tools not available

- Embedding tools require `--embedding-url`
- Redis tools require `--redis-url`
- Verify services are running and reachable

---

## Project Structure

```
river-engine/
├── crates/
│   ├── river-core/          # Shared types, errors, IDs
│   ├── river-gateway/       # Agent runtime
│   ├── river-orchestrator/  # Model coordination
│   └── river-discord/       # Discord adapter
├── nix/
│   ├── packages.nix         # Nix package definitions
│   ├── lib.nix              # Shared module library
│   ├── nixos-module.nix     # NixOS system module
│   └── home-module.nix      # Home-manager module
└── docs/
    └── superpowers/
        ├── specs/           # Design specifications
        ├── plans/           # Implementation plans
        ├── STATUS.md        # Implementation status
        └── FUTURE.md        # Future considerations
```

---

## License

[Add your license here]

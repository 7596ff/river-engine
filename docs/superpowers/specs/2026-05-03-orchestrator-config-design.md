# Orchestrator Config & Process Management

## Goal

The orchestrator reads a single JSON config file and starts the entire river-engine system: its own HTTP server, one or more gateways, and their adapters. One command, one config.

```
river-orchestrator --config river.json [--env-file river.env]
```

## Config Structure

```json
{
  "port": 5000,

  "models": {
    "claude-sonnet": {
      "provider": "anthropic",
      "endpoint": "https://api.anthropic.com/v1",
      "name": "claude-sonnet-4-20250514",
      "api_key": "$ANTHROPIC_API_KEY",
      "context_limit": 200000
    },
    "local-qwen": {
      "provider": "gguf",
      "path": "/models/qwen3-8b-q4.gguf",
      "context_limit": 32000
    },
    "nomic-embed": {
      "provider": "ollama",
      "endpoint": "http://localhost:11434/v1",
      "name": "nomic-embed-text",
      "dimensions": 768
    }
  },

  "agents": {
    "iris": {
      "workspace": "/home/cassie/stream",
      "data_dir": "/var/lib/river/iris",
      "port": 3000,
      "model": "claude-sonnet",
      "spectator_model": "claude-sonnet",
      "embedding_model": "nomic-embed",
      "context": {
        "limit": 200000,
        "compaction_threshold": 0.80,
        "fill_target": 0.40,
        "min_messages": 20
      },
      "adapters": [
        {
          "type": "discord",
          "bin": "river-discord",
          "port": 8081,
          "token": "$DISCORD_TOKEN",
          "guild_id": "$DISCORD_GUILD_ID",
          "channels": ["general", "bot"]
        }
      ]
    }
  },

  "resources": {
    "reserve_vram_mb": 500,
    "reserve_ram_mb": 2000,
    "llama_server_path": "llama-server",
    "port_range": "8100-8200"
  }
}
```

### models

A named map of model backends. Each key is a model ID referenced by agents. Three provider types:

- **External API** (`provider: "anthropic"`, `"openai"`, `"ollama"`, etc.) — has `endpoint`, `name`, optional `api_key`. The orchestrator passes the endpoint URL to the gateway as `--model-url`. The orchestrator does not proxy requests — agents talk directly to the model endpoint.
- **Local GGUF** (`provider: "gguf"`) — has `path` to the GGUF file. The orchestrator manages loading/unloading via llama-server. When a gateway needs this model, the orchestrator ensures it's loaded and returns the local llama-server endpoint.
- **Embedding** — any model with a `dimensions` field is an embedding model. Referenced by `embedding_model` in agent config. Passed to the gateway as `--embedding-url`.

### agents

A named map of agents. Each agent becomes one `river-gateway` process. Fields:

- `workspace` — path to agent's workspace directory (required)
- `data_dir` — path to agent's data directory containing `river.db` (required)
- `port` — gateway HTTP port (required)
- `model` — key into `models` map for the agent's primary model (required)
- `spectator_model` — key into `models` map for the spectator/bystander model (optional, defaults to `model`)
- `embedding_model` — key into `models` map for embeddings (optional)
- `context` — context window configuration (all fields optional, defaults below):
  - `limit` — total context window size in tokens (default: 128000)
  - `compaction_threshold` — fraction of limit that triggers compaction (default: 0.80)
  - `fill_target` — post-compaction fill target as fraction of limit (default: 0.40)
  - `min_messages` — minimum messages always kept in context (default: 20)
- `adapters` — list of adapter configurations (see below)

### agents.\<name\>.adapters

Each adapter entry describes a process the orchestrator spawns to connect the agent to an external service.

- `type` — adapter type, determines which binary to run (required)
- `bin` — path to the adapter binary (optional, defaults to `river-{type}`)
- `port` — HTTP port for the adapter's outbound server (required)
- Remaining fields are adapter-specific and passed as CLI args

For `type: "discord"`:
- `token` — Discord bot token (required)
- `guild_id` — Discord guild/server ID (required)
- `channels` — list of channel names to listen in (optional)

### resources

Global resource management for local model serving. Only relevant if any model has `provider: "gguf"`.

- `reserve_vram_mb` — VRAM to keep free (default: 500)
- `reserve_ram_mb` — RAM to keep free (default: 2000)
- `llama_server_path` — path to llama-server binary (default: `"llama-server"`)
- `port_range` — port range for managed llama-server instances, format `"start-end"` (default: `"8080-8180"`)

## Environment Variable Handling

### --env-file

The `--env-file` flag loads a key-value file into the process environment before config parsing. Format:

```
# Comments and blank lines ignored
ANTHROPIC_API_KEY=sk-ant-...
DISCORD_TOKEN=...
DISCORD_GUILD_ID=1234567890
```

**Existing environment wins.** If a variable is already set in the process environment, the env file value is ignored. This matches systemd `EnvironmentFile=` semantics and means you can override env file defaults by exporting variables in your shell.

### $VAR expansion

After the environment is assembled (env file + existing env), the JSON config is loaded as a string and all `$VAR` references are expanded before JSON parsing. A `$VAR` that cannot be resolved is a fatal error — the orchestrator logs which variable is missing, which line of the config references it, and exits.

Expansion happens on the raw string, not on parsed JSON values. This means `$VAR` works in any string position.

## Process Management

### Startup sequence

1. Load `--env-file` if provided (existing env wins)
2. Read config file, expand `$VAR` references, parse JSON
3. Validate config: all model references resolve, no port conflicts, required fields present
4. Start orchestrator HTTP server on configured port
5. For each agent:
   a. Check `data_dir` for `river.db` with a valid birth memory
   b. If no birth: log an error with the exact command to run (`river-gateway birth --data-dir <path> --name <name>`), skip this agent
   c. Resolve the agent's model to an endpoint URL (for external models this is immediate; for GGUF models the orchestrator loads the model first)
   d. Spawn `river-gateway` with CLI args translated from config
   e. Spawn each adapter with CLI args translated from config
6. If no agents could start, exit with error

### Gateway CLI translation

The orchestrator translates agent config into `river-gateway` CLI args:

```
river-gateway \
  --workspace /home/cassie/stream \
  --data-dir /var/lib/river/iris \
  --port 3000 \
  --agent-name iris \
  --model-url <resolved endpoint> \
  --model-name <model name> \
  --context-limit 200000 \
  --orchestrator-url http://127.0.0.1:5000 \
  --adapter discord:http://127.0.0.1:8081/send:http://127.0.0.1:8081/read \
  [--embedding-url <resolved endpoint>] \
  [--spectator-model-url <resolved endpoint>] \
  [--spectator-model-name <model name>] \
  [--auth-token-file <generated>]
```

Context window shape parameters (`compaction_threshold`, `fill_target`, `min_messages`) are not currently accepted as CLI args by the gateway. The implementation plan will need to add these as CLI args to the gateway, or the orchestrator writes a small config fragment the gateway reads. Recommendation: add CLI args to the gateway — it's simpler and keeps the gateway independently runnable.

### Adapter CLI translation

For `type: "discord"`:

```
river-discord \
  --token <token> \
  --gateway-url http://127.0.0.1:<agent_port> \
  --guild-id <guild_id> \
  --listen-port <adapter_port> \
  [--channels general,bot]
```

### Child process monitoring

- Stdout and stderr from child processes are captured and forwarded to the orchestrator's log with prefixes (`[iris/gateway]`, `[iris/discord]`)
- If a child exits, the orchestrator restarts it with exponential backoff: 1s, 2s, 4s, 8s, 16s, 32s, 60s cap
- The backoff counter resets after 5 minutes of the child running without crashing
- The orchestrator logs each restart with the attempt number and backoff delay

### Shutdown

On SIGTERM or SIGINT:
1. Send SIGTERM to all child processes
2. Wait up to 10 seconds for graceful shutdown
3. Send SIGKILL to any remaining children
4. Exit

## What Changes

- **`river-orchestrator`** — new `--config` and `--env-file` CLI args, config parsing module, process spawner, child monitor. The existing CLI args become a fallback mode (direct CLI usage without config file still works).
- **`river-gateway`** — add CLI args for context window shape parameters (`--compaction-threshold`, `--fill-target`, `--min-messages`). No other changes.
- **`river-discord`** — no changes.

## Out of Scope

- Hot-reloading the config file (restart the orchestrator to apply changes)
- Web UI for config management
- Orchestrator-to-gateway communication for runtime config changes (the orchestrator spawns processes with args, it does not reconfigure them after start)
- Agent birth automation (the human runs `river-gateway birth` manually)

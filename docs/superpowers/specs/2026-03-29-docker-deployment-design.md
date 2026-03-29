# Docker Deployment Design Spec

> Containerized deployment for river-engine gateway stack
>
> Date: 2026-03-29
> Authors: Cass, Claude

---

## 1. Summary

Docker Compose setup that runs the full river-engine stack: gateway, discord adapter, orchestrator, and Ollama. Single `.env` file configures all services. Supports both local models (Ollama) and external APIs (Anthropic/OpenAI) with separate configs for actor and spectator models.

---

## 2. Services

| Service | Image | Purpose |
|---------|-------|---------|
| `gateway` | `river-engine` | Agent runtime, HTTP API, tools |
| `discord` | `river-engine` | Discord adapter bridge |
| `orchestrator` | `river-engine` | Model lifecycle management |
| `ollama` | `ollama/ollama` | LLM inference + embeddings |

All river services use the same image with different entrypoints.

---

## 3. File Structure

```
river-engine/
├── Dockerfile              # Multi-stage Alpine build
├── docker-compose.yml      # 4-service stack
├── docker-entrypoint.sh    # Passthrough entrypoint
├── .env.example            # Environment template
└── .dockerignore           # Build exclusions
```

**Runtime directories (user creates):**
```
./data/          # SQLite DB, logs
./workspace/     # Conversations, inbox
./models/        # Ollama model cache
./secrets/       # Discord token file
```

---

## 4. Environment Variables

Single flat `.env` file. Gateway uses CLI args, which reference these env vars via shell expansion.

```bash
# === Agent Identity ===
AGENT_NAME=river

# === Discord ===
DISCORD_GUILD_ID=123456789
DISCORD_CHANNELS=111,222,333

# === Actor Model (main agent) ===
# For Ollama (default):
MODEL_URL=http://ollama:11434
MODEL_NAME=llama3.2

# For external API (e.g., Anthropic via orchestrator):
# MODEL_URL=http://orchestrator:5000
# MODEL_NAME=claude-sonnet-4-20250514
# ANTHROPIC_API_KEY=sk-ant-...

# === Spectator Model (observer/summarizer) ===
# Defaults to same as actor if not set
SPECTATOR_MODEL_URL=http://ollama:11434
SPECTATOR_MODEL_NAME=llama3.2

# === Embeddings ===
EMBEDDING_URL=http://ollama:11434

# === Ports ===
GATEWAY_PORT=3000
DISCORD_PORT=3002
ORCHESTRATOR_PORT=5000
OLLAMA_PORT=11434
```

**Note:** `ANTHROPIC_API_KEY` and `OPENROUTER_API_KEY` are read from environment by the gateway when using external APIs.

---

## 5. Dockerfile

Multi-stage Alpine build:

```dockerfile
# Stage 1: Build (musl)
FROM rust:1.83-alpine AS builder
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static
WORKDIR /app
COPY . .
RUN cargo build --release \
    --bin river-gateway \
    --bin river-discord \
    --bin river-orchestrator

# Stage 2: Runtime
FROM alpine:3.21
RUN apk add --no-cache ca-certificates
COPY --from=builder /app/target/release/river-gateway /usr/local/bin/
COPY --from=builder /app/target/release/river-discord /usr/local/bin/
COPY --from=builder /app/target/release/river-orchestrator /usr/local/bin/
COPY docker-entrypoint.sh /usr/local/bin/

WORKDIR /app
ENTRYPOINT ["docker-entrypoint.sh"]
```

**Image size:** ~30-40MB

---

## 6. docker-compose.yml

```yaml
services:
  ollama:
    image: ollama/ollama:latest
    volumes:
      - ./models:/root/.ollama
    ports:
      - "${OLLAMA_PORT:-11434}:11434"

  orchestrator:
    image: river-engine
    build: .
    command: river-orchestrator --port 5000
    depends_on:
      - ollama
    ports:
      - "${ORCHESTRATOR_PORT:-5000}:5000"
    env_file: .env

  gateway:
    image: river-engine
    command: >
      river-gateway
        --workspace /app/workspace
        --data-dir /app/data
        --agent-name ${AGENT_NAME:-river}
        --port 3000
        --model-url ${MODEL_URL:-http://ollama:11434}
        --model-name ${MODEL_NAME:-llama3.2}
        --spectator-model-url ${SPECTATOR_MODEL_URL:-http://ollama:11434}
        --spectator-model-name ${SPECTATOR_MODEL_NAME:-llama3.2}
        --embedding-url ${EMBEDDING_URL:-http://ollama:11434}
        --orchestrator-url http://orchestrator:5000
    depends_on:
      - orchestrator
      - ollama
    volumes:
      - ./data:/app/data
      - ./workspace:/app/workspace
    ports:
      - "${GATEWAY_PORT:-3000}:3000"
    env_file: .env

  discord:
    image: river-engine
    command: >
      river-discord
        --token-file /app/secrets/discord-token
        --guild-id ${DISCORD_GUILD_ID}
        --channels ${DISCORD_CHANNELS}
        --gateway-url http://gateway:3000
        --listen-port 3002
    depends_on:
      - gateway
    volumes:
      - ./secrets:/app/secrets:ro
      - ./data:/app/data
    ports:
      - "${DISCORD_PORT:-3002}:3002"
    env_file: .env
```

---

## 7. Entrypoint Script

Simple passthrough (no auto-magic):

```bash
#!/bin/sh
exec "$@"
```

---

## 8. Usage

### Initial Setup

```bash
# 1. Copy and edit env file
cp .env.example .env
# Edit .env with your values

# 2. Create directories
mkdir -p data workspace models secrets

# 3. Add Discord token
echo "your-discord-bot-token" > secrets/discord-token

# 4. Pull Ollama models
docker compose run --rm ollama ollama pull llama3.2
docker compose run --rm ollama ollama pull nomic-embed-text

# 5. Birth the agent (once)
docker compose run --rm gateway river-gateway birth \
  --data-dir /app/data --name my-agent
```

### Running

```bash
# Start all services
docker compose up -d

# View logs
docker compose logs -f gateway

# Stop
docker compose down
```

### Rebuilding

```bash
docker compose build --no-cache
```

---

## 9. Data Persistence

All state persists in bind mounts:

| Mount | Contents |
|-------|----------|
| `./data/` | `river.db` (SQLite), logs |
| `./workspace/` | Conversations, inbox files |
| `./models/` | Ollama model weights |
| `./secrets/` | Discord token (read-only mount) |

To reset: delete `./data/river.db` and re-run birth command.

---

## 10. Network

Services communicate via Docker network using service names:

- Gateway → Orchestrator: `http://orchestrator:5000`
- Gateway → Ollama: `http://ollama:11434`
- Discord → Gateway: `http://gateway:3000`

External access via mapped ports on host.

---

## 11. Testing

Manual verification:

1. `docker compose up -d`
2. Check health: `curl http://localhost:3000/health`
3. Check Discord adapter: `curl http://localhost:3002/health`
4. Check Ollama: `curl http://localhost:11434/api/tags`

---

## 12. File Summary

| File | Purpose |
|------|---------|
| `Dockerfile` | Multi-stage Alpine build for all river binaries |
| `docker-compose.yml` | 4-service stack definition |
| `docker-entrypoint.sh` | Passthrough entrypoint |
| `.env.example` | Environment variable template |
| `.dockerignore` | Exclude target/, .git, etc. from build context |

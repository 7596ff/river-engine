# Docker Deployment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create Docker Compose setup for running the full river-engine stack with a single env file.

**Architecture:** Multi-stage Alpine Dockerfile builds all river binaries into one image. docker-compose.yml defines 4 services (gateway, discord, orchestrator, ollama) that communicate via Docker network. Single .env file configures all services.

**Tech Stack:** Docker, docker-compose, Alpine Linux, Rust (musl target)

**Spec:** `docs/superpowers/specs/2026-03-29-docker-deployment-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `Dockerfile` | Create | Multi-stage build: rust:alpine builder, alpine runtime |
| `docker-compose.yml` | Create | 4-service stack definition with volumes and networks |
| `docker-entrypoint.sh` | Create | Simple passthrough entrypoint |
| `.env.example` | Create | Environment variable template with all options |
| `.dockerignore` | Create | Exclude build artifacts from context |
| `DOCKER.md` | Create | User-facing README for Docker deployment |

---

## Task 1: Create .dockerignore

**Files:**
- Create: `.dockerignore`

- [ ] **Step 1: Create .dockerignore file**

```
target/
.git/
.gitignore
*.md
docs/
tests/
.env
data/
workspace/
models/
secrets/
*.log
.DS_Store
```

- [ ] **Step 2: Verify file exists**

Run: `cat .dockerignore`
Expected: File contents shown

- [ ] **Step 3: Commit**

```bash
git add .dockerignore && git commit -m "$(cat <<'EOF'
build: add .dockerignore for Docker builds

Excludes target/, .git/, docs, and runtime directories
from Docker build context.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Create docker-entrypoint.sh

**Files:**
- Create: `docker-entrypoint.sh`

- [ ] **Step 1: Create entrypoint script**

```bash
#!/bin/sh
exec "$@"
```

- [ ] **Step 2: Make executable**

Run: `chmod +x docker-entrypoint.sh`

- [ ] **Step 3: Verify**

Run: `ls -la docker-entrypoint.sh`
Expected: `-rwxr-xr-x` permissions

- [ ] **Step 4: Commit**

```bash
git add docker-entrypoint.sh && git commit -m "$(cat <<'EOF'
build: add Docker entrypoint script

Simple passthrough entrypoint for container commands.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Create Dockerfile

**Files:**
- Create: `Dockerfile`

- [ ] **Step 1: Create Dockerfile**

```dockerfile
# Stage 1: Build (musl for static linking)
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

RUN chmod +x /usr/local/bin/docker-entrypoint.sh

WORKDIR /app

ENTRYPOINT ["docker-entrypoint.sh"]
```

- [ ] **Step 2: Verify file created**

Run: `head -5 Dockerfile`
Expected: Shows FROM rust:1.83-alpine AS builder

- [ ] **Step 3: Commit**

```bash
git add Dockerfile && git commit -m "$(cat <<'EOF'
build: add multi-stage Dockerfile for river-engine

Two-stage Alpine build:
- Stage 1: rust:1.83-alpine builds gateway, discord, orchestrator
- Stage 2: alpine:3.21 runtime (~30-40MB image)

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Create .env.example

**Files:**
- Create: `.env.example`

- [ ] **Step 1: Create env template**

```bash
# River Engine Docker Configuration
# Copy to .env and fill in your values

# === Agent Identity ===
AGENT_NAME=river

# === Discord ===
DISCORD_GUILD_ID=123456789012345678
DISCORD_CHANNELS=111111111111111111,222222222222222222

# === Actor Model (main agent) ===
# For Ollama (default):
MODEL_URL=http://ollama:11434
MODEL_NAME=llama3.2

# For external API (uncomment and configure):
# MODEL_URL=http://orchestrator:5000
# MODEL_NAME=claude-sonnet-4-20250514
# ANTHROPIC_API_KEY=sk-ant-...

# === Spectator Model (observer/summarizer) ===
# Defaults to same as actor if not set
SPECTATOR_MODEL_URL=http://ollama:11434
SPECTATOR_MODEL_NAME=llama3.2

# === Embeddings ===
EMBEDDING_URL=http://ollama:11434

# === Ports (change if conflicts) ===
GATEWAY_PORT=3000
DISCORD_PORT=3002
ORCHESTRATOR_PORT=5000
OLLAMA_PORT=11434
```

- [ ] **Step 2: Verify file**

Run: `head -5 .env.example`
Expected: Shows header comments

- [ ] **Step 3: Commit**

```bash
git add .env.example && git commit -m "$(cat <<'EOF'
build: add .env.example for Docker configuration

Template includes all configurable options:
- Agent identity and Discord settings
- Actor/spectator model URLs and names
- Embedding URL and port mappings

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Create docker-compose.yml

**Files:**
- Create: `docker-compose.yml`

- [ ] **Step 1: Create docker-compose.yml**

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

- [ ] **Step 2: Validate syntax**

Run: `docker compose config --quiet 2>&1 || echo "Validation requires .env"`
Expected: Either quiet success or note about .env

- [ ] **Step 3: Commit**

```bash
git add docker-compose.yml && git commit -m "$(cat <<'EOF'
build: add docker-compose.yml for full stack

Four services:
- ollama: LLM inference and embeddings
- orchestrator: Model lifecycle management
- gateway: Agent runtime with HTTP API
- discord: Discord adapter bridge

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Create DOCKER.md

**Files:**
- Create: `DOCKER.md`

- [ ] **Step 1: Create README**

```markdown
# River Engine Docker Deployment

Run the full river-engine stack with Docker Compose.

## Services

| Service | Port | Purpose |
|---------|------|---------|
| gateway | 3000 | Agent runtime, HTTP API |
| discord | 3002 | Discord adapter |
| orchestrator | 5000 | Model management |
| ollama | 11434 | LLM inference |

## Quick Start

```bash
# 1. Configure environment
cp .env.example .env
# Edit .env with your Discord guild ID, channel IDs, etc.

# 2. Create directories
mkdir -p data workspace models secrets

# 3. Add Discord bot token
echo "your-bot-token" > secrets/discord-token

# 4. Build the image
docker compose build

# 5. Pull Ollama models
docker compose run --rm ollama ollama pull llama3.2
docker compose run --rm ollama ollama pull nomic-embed-text

# 6. Birth the agent (first time only)
docker compose run --rm gateway river-gateway birth \
  --data-dir /app/data --name river

# 7. Start everything
docker compose up -d
```

## Commands

```bash
# View logs
docker compose logs -f gateway

# Stop all services
docker compose down

# Rebuild after code changes
docker compose build --no-cache

# Shell into gateway container
docker compose exec gateway sh
```

## Configuration

All configuration is in `.env`. Key settings:

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENT_NAME` | river | Agent identifier |
| `DISCORD_GUILD_ID` | - | Your Discord server ID |
| `DISCORD_CHANNELS` | - | Comma-separated channel IDs |
| `MODEL_URL` | http://ollama:11434 | LLM server URL |
| `MODEL_NAME` | llama3.2 | Model to use |

### Using External APIs

To use Anthropic/OpenAI instead of Ollama:

```bash
MODEL_URL=http://orchestrator:5000
MODEL_NAME=claude-sonnet-4-20250514
ANTHROPIC_API_KEY=sk-ant-...
```

## Data Persistence

| Directory | Contents |
|-----------|----------|
| `./data/` | SQLite database, logs |
| `./workspace/` | Conversations, inbox |
| `./models/` | Ollama model cache |
| `./secrets/` | Discord token |

## Health Checks

```bash
# Gateway
curl http://localhost:3000/health

# Discord adapter
curl http://localhost:3002/health

# Ollama
curl http://localhost:11434/api/tags
```

## Troubleshooting

**"No such file: /app/data/river.db"**
Run the birth command first (step 6 above).

**Discord not connecting**
Check `secrets/discord-token` contains your bot token with no extra whitespace.

**Model not responding**
Verify Ollama has the model: `docker compose exec ollama ollama list`
```

- [ ] **Step 2: Verify file**

Run: `head -20 DOCKER.md`
Expected: Shows title and services table

- [ ] **Step 3: Commit**

```bash
git add DOCKER.md && git commit -m "$(cat <<'EOF'
docs: add DOCKER.md for containerized deployment

User-facing guide covering:
- Quick start with step-by-step setup
- Common commands and configuration
- External API setup (Anthropic/OpenAI)
- Troubleshooting tips

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Create .dockerignore | `.dockerignore` |
| 2 | Create entrypoint script | `docker-entrypoint.sh` |
| 3 | Create Dockerfile | `Dockerfile` |
| 4 | Create env template | `.env.example` |
| 5 | Create docker-compose | `docker-compose.yml` |
| 6 | Create README | `DOCKER.md` |

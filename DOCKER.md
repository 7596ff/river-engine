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

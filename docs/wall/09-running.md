# 09 — Running

One config file describes the whole system. Two runners consume it: the
`river` CLI for development and any non-nix machine, and a NixOS module
for production. Secrets live in a `.env` file. That is the entire
operational story.

## The config file

JSON. One file, however many agents.

```json
{
  "models": {
    "sonnet": {
      "provider": "anthropic",
      "endpoint": "https://api.anthropic.com/v1",
      "name": "claude-sonnet-4-20250514",
      "api_key_env": "ANTHROPIC_KEY",
      "context_limit": 200000
    },
    "local": {
      "provider": "openai",
      "endpoint": "http://localhost:11434/v1",
      "name": "qwen3:8b",
      "context_limit": 32000
    },
    "embed": {
      "provider": "openai",
      "endpoint": "http://localhost:11434/v1",
      "name": "nomic-embed-text",
      "dimensions": 768
    }
  },
  "agents": {
    "ada": {
      "workspace": "$HOME/ada",
      "data_dir": "$HOME/.local/state/river/ada",
      "model": "sonnet",
      "witness_model": "local",
      "embedding_model": "embed",
      "context": { "limit": 200000 },
      "tools": ["read","write","edit","glob","grep","bash","speak","search"],
      "heartbeat_minutes": 45,
      "adapters": [
        { "type": "discord", "guild_id": "$GUILD", "channels": ["general"],
          "token_env": "DISCORD_TOKEN_ADA" },
        { "type": "local", "port": 7700 }
      ]
    }
  }
}
```

- **models** — named backends. `provider` selects the client protocol
  (`anthropic` | `openai`-compatible). A model with `dimensions` is an
  embedding model. `api_key_env` / `token_env` *name an environment
  variable*; the value never appears in config.
- **agents** — each entry is one gateway. Workspace, data dir, model
  assignments (witness defaults to the agent's model), context knobs
  (ch. 03 — all optional), tool profile (ch. 07), heartbeat,
  adapter bindings (ch. 06).
- **`$VAR` expansion** — the raw config text is expanded against the
  environment before parsing, for *non-secret* values (paths, IDs). An
  unresolvable `$VAR` is fatal, with the variable name and config line
  reported.
- **Validation before spawn** — model references resolve, ports don't
  collide, workspaces are not shared between agents, required fields
  present. All errors reported together, then exit.

## Secrets

Secrets live in a `.env` file — gitignored, plain `KEY=value` lines:

```
ANTHROPIC_KEY=sk-ant-...
DISCORD_TOKEN_ADA=...
```

The CLI loads it with `--env-file river.env`; systemd loads it with
`EnvironmentFile=`. Already-set environment variables win over the
file. Two guards keep this simple scheme safe:

1. **Secrets never pass through `$VAR` expansion.** Config text names
   the variable (`api_key_env`); the client reads the value from the
   environment at call time. Expanded config — which is validated,
   logged, and debuggable — never contains a secret.
2. **Tool children are scrubbed** (ch. 07). Every variable named by
   any `*_env` field is stripped from child-process environments. The
   agent's shell never inherits a key.

## The `river` CLI

```
river --config river.json [--env-file river.env]      # run everything
river status                                          # health of each agent
river-gateway birth --data-dir <dir> --name <name>    # the birth ritual
```

The CLI parses and validates, then spawns one `river-gateway` process
per agent and supervises: stdout/stderr forwarded with `[name]`
prefixes, crash restart with exponential backoff (1s doubling to 60s,
reset after 5 healthy minutes), and on Ctrl-C/SIGTERM a graceful
cascade — SIGTERM to each gateway, a grace period (default 30s, long
enough to finish a turn; ch. 01), then SIGKILL for stragglers. An
unbirthed agent is reported (with the birth command) and skipped;
others start.

## The NixOS module

Production runs without the CLI: the module renders the same config
file and generates **one systemd service per agent** —
`river-ada.service` — with `EnvironmentFile=` for secrets, restart
policy in systemd (`Restart=on-failure`, backoff via systemd's own
knobs), the gateway's watchdog integration (`Type=notify`,
`WatchdogSec`), and `TimeoutStopSec` aligned to the turn-finishing
grace period. Per-agent services mean per-agent restarts, journals,
and resource limits, with systemd as the supervisor instead of a
second homegrown one.

## Health

Each gateway serves `/health` on its local surface (ch. 06). Every
field is written by the live path: current turn number, last
settle time, context usage percent, witness lag (agent turn minus
witness cursor), adapter task states, queue depth. The CLI's
`river status` and the watchdog both read this. **A health field
nobody writes may not exist** — observability that lies with
confidence is worse than none.

## Contracts

- **One config, two runners.** CLI and nix module accept the same
  file; behavior differs only in who supervises.
- **Secrets:** `.env` file; named-variable indirection; never in
  config text, logs, or tool child environments; existing env wins
  over the file.
- **`$VAR` is for non-secrets**; unresolvable is fatal with line
  context.
- **Validate everything before spawning anything**; report all errors
  at once.
- **Restart backoff:** 1s → 60s cap, reset after 5 healthy minutes
  (CLI); systemd equivalents in the module.
- **Graceful stop:** SIGTERM → grace period sized to a turn → SIGKILL.
- **Live-path health only.**

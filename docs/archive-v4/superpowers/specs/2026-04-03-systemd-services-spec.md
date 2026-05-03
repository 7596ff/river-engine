# River Engine systemd Services — Design Spec

> Authors: Cass, Claude
> Date: 2026-04-03

## Overview

systemd service files for running the River Engine orchestrator as a managed service. Two variants: system-level (production) and user-level (development).

## Components

Only the orchestrator needs a service file — it spawns and manages workers, adapters, and embed services.

## Paths

| Variant | Binary | Config | Environment |
|---------|--------|--------|-------------|
| System | `/usr/local/bin/river-orchestrator` | `/etc/river/river.json` | `/etc/river/river.env` |
| User | `/usr/local/bin/river-orchestrator` | `~/.config/river/river.json` | `~/.config/river/river.env` |

## Service Behavior

| Setting | Value | Rationale |
|---------|-------|-----------|
| Type | `simple` | Orchestrator runs in foreground |
| Restart | `on-failure` | Auto-recover from crashes, not clean exits |
| RestartSec | `5` | Wait 5s between restart attempts |
| StartLimitBurst | `5` | Max 5 restarts per interval |
| StartLimitIntervalSec | `60` | Interval window for restart limit |
| After | `network-online.target` | Wait for network (Discord, HTTP) |
| Logging | journald | `journalctl -u river-orchestrator` |

## Files

- `deploy/river-orchestrator.service` — system service
- `deploy/river-orchestrator.user.service` — user service

## Installation

**System:**
```bash
sudo cp deploy/river-orchestrator.service /etc/systemd/system/
sudo useradd -r -s /bin/false river  # if user doesn't exist
sudo mkdir -p /etc/river
sudo systemctl daemon-reload
sudo systemctl enable river-orchestrator
sudo systemctl start river-orchestrator
```

**User:**
```bash
mkdir -p ~/.config/systemd/user ~/.config/river
cp deploy/river-orchestrator.user.service ~/.config/systemd/user/river-orchestrator.service
systemctl --user daemon-reload
systemctl --user enable river-orchestrator
systemctl --user start river-orchestrator
```

## Environment File

Example `/etc/river/river.env` or `~/.config/river/river.env`:
```bash
RUST_LOG=info,river_orchestrator=debug
DISCORD_TOKEN=your_token_here
ANTHROPIC_API_KEY=your_key_here
```

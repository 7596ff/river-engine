# NixOS Module Design Specification

**Date:** 2026-03-16
**Status:** Draft

## Overview

NixOS and home-manager modules for deploying River Engine services. System-level modules manage shared infrastructure (orchestrator, embedding server), while user-level modules manage per-user agents (gateway, discord adapter).

## Architecture

### Deployment Model

- **System level (NixOS module)**: Orchestrator, embedding server, Redis - shared infrastructure running as system services
- **User level (home-manager module)**: Agents with gateway and optional adapters - per-user services running in user sessions

Users compose these layers: system services start at boot, user agents start on login and connect to system services via explicit URLs.

### File Structure

```
nix/
├── lib.nix              # Shared option types and builders
├── nixos-module.nix     # System services (orchestrator, embedding, redis)
├── home-module.nix      # User agents (gateway, discord)
└── packages.nix         # Package definitions for River binaries
```

## NixOS Module (System Services)

### Import

```nix
# configuration.nix
imports = [ /path/to/river-engine/nix/nixos-module.nix ];
```

### Options

```nix
services.river = {
  orchestrator = {
    enable = mkEnableOption "River orchestrator";
    port = mkOption { type = port; default = 5000; };
    healthThreshold = mkOption { type = int; default = 120; };
    modelDirs = mkOption { type = listOf path; default = []; };
    externalModelsFile = mkOption { type = nullOr path; default = null; };
    modelsConfigFile = mkOption { type = nullOr path; default = null; };  # Legacy models JSON
    llamaServerPath = mkOption { type = nullOr path; default = null; };
    idleTimeout = mkOption { type = int; default = 900; };
    portRange = mkOption { type = str; default = "8080-8180"; };
    reserveVramMb = mkOption { type = int; default = 500; };
    reserveRamMb = mkOption { type = int; default = 2000; };
    cudaSupport = mkOption { type = bool; default = false; };
    environment = mkOption { type = attrsOf str; default = {}; };  # Extra env vars
  };

  embedding = {
    enable = mkEnableOption "River embedding server";
    port = mkOption { type = port; default = 8200; };  # Outside orchestrator port range
    modelPath = mkOption { type = path; };
    gpuLayers = mkOption { type = int; default = 99; };
    cudaSupport = mkOption { type = bool; default = false; };
  };

  redis = {
    enable = mkEnableOption "Redis for River";
    port = mkOption { type = port; default = 6379; };
  };
};
```

### Generated Services

**river-orchestrator.service:**
- Runs `river-orchestrator` binary with configured options
- `DynamicUser = true` (no manual user creation)
- `StateDirectory = "river-orchestrator"`
- `Restart = "on-failure"` with 5 second delay
- `WantedBy = ["multi-user.target"]`

**river-embedding.service:**
- Runs `llama-server --embedding` with model path and port
- Uses CUDA-enabled llama-cpp if `cudaSupport = true`
- `DynamicUser = true`
- `Restart = "on-failure"`
- `WantedBy = ["multi-user.target"]`

**Redis:**
- When `services.river.redis.enable = true`, configures `services.redis.servers.river` using the standard NixOS redis module

### Example Configuration

```nix
services.river = {
  orchestrator = {
    enable = true;
    port = 5000;
    modelDirs = [ /models/gguf ];
    cudaSupport = true;
  };

  embedding = {
    enable = true;
    port = 8200;
    modelPath = /models/nomic-embed-text-v1.5.Q8_0.gguf;
    cudaSupport = true;
  };

  redis.enable = true;
};
```

## Home-Manager Module (User Agents)

### Import

```nix
# home.nix
imports = [ /path/to/river-engine/nix/home-module.nix ];
```

### Options

```nix
services.river = {
  agents = mkOption {
    type = attrsOf (submodule ({ name, ... }: {
      options = {
        enable = mkEnableOption "this River agent";

        # Core settings
        workspace = mkOption { type = path; };           # Required
        dataDir = mkOption { type = path; };             # Required
        agentName = mkOption { type = str; default = name; };  # Defaults to attr name
        port = mkOption { type = port; default = 3000; };

        # Model configuration (optional - can rely on orchestrator)
        modelUrl = mkOption { type = nullOr str; default = null; };
        modelName = mkOption { type = nullOr str; default = null; };

        # Optional service URLs
        orchestratorUrl = mkOption { type = nullOr str; default = null; };
        embeddingUrl = mkOption { type = nullOr str; default = null; };
        redisUrl = mkOption { type = nullOr str; default = null; };

        # Extra environment variables
        environment = mkOption { type = attrsOf str; default = {}; };

        # Discord adapter
        discord = {
          enable = mkEnableOption "Discord adapter";
          tokenFile = mkOption { type = path; };                    # Required if enabled
          guildId = mkOption { type = str; };                       # Discord snowflake (string)
          gatewayUrl = mkOption { type = nullOr str; default = null; };  # Auto-derives from agent port if null
          port = mkOption { type = port; default = 3002; };
          channels = mkOption { type = listOf str; default = []; }; # Discord snowflakes (strings)
          stateFile = mkOption { type = nullOr path; default = null; };
        };
      };
    }));
    default = {};
  };
};
```

### Generated Services

For each agent `name` with `enable = true`:

**river-{name}-gateway.service (user unit):**
- Runs `river-gateway` with workspace, data-dir, model settings
- Passes optional service URLs as CLI args when configured
- `Restart = "on-failure"` with 5 second delay
- `WantedBy = ["default.target"]`

**river-{name}-discord.service (user unit, if discord.enable):**
- Runs `river-discord` with token file, guild ID, gateway URL
- `After = ["river-{name}-gateway.service"]`
- `BindsTo = ["river-{name}-gateway.service"]` (stops if gateway stops)
- `Restart = "on-failure"`
- `WantedBy = ["default.target"]`

### Example Configuration

```nix
services.river.agents.thomas = {
  enable = true;
  workspace = "${config.home.homeDirectory}/workspace/thomas";
  dataDir = "${config.xdg.dataHome}/river/thomas";
  port = 3000;

  modelUrl = "http://localhost:8080/v1";
  modelName = "qwen3-32b-q4_k_m";

  orchestratorUrl = "http://localhost:5000";
  embeddingUrl = "http://localhost:8200/v1";
  redisUrl = "redis://localhost:6379";

  discord = {
    enable = true;
    tokenFile = "/run/user/1000/secrets/discord-token";
    guildId = "123456789012345678";  # Discord snowflake as string
    port = 3002;
  };
};
```

## Shared Library (lib.nix)

Defines common option types used by both modules:

- `agentOptions` - Submodule options for agent configuration
- `discordOptions` - Submodule options for Discord adapter
- `mkGatewayService` - Function to generate gateway service config
- `mkDiscordService` - Function to generate Discord adapter service config
- `serviceUrlOption` - Reusable option type for service URLs

This ensures option definitions are identical between NixOS and home-manager modules.

## Package Definitions (packages.nix)

```nix
{ pkgs, cudaSupport ? false }:

let
  llama-cpp = pkgs.llama-cpp.override {
    inherit cudaSupport;
  };

  # Common build inputs for River Rust packages
  commonBuildInputs = with pkgs; [ openssl ];
  commonNativeBuildInputs = with pkgs; [ pkg-config ];
in {
  inherit llama-cpp;

  river-gateway = pkgs.rustPlatform.buildRustPackage {
    pname = "river-gateway";
    version = "0.1.0";
    src = ./..;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-gateway" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };

  river-orchestrator = pkgs.rustPlatform.buildRustPackage {
    pname = "river-orchestrator";
    version = "0.1.0";
    src = ./..;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-orchestrator" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };

  river-discord = pkgs.rustPlatform.buildRustPackage {
    pname = "river-discord";
    version = "0.1.0";
    src = ./..;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-discord" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };
}
```

## Service Discovery

User-level agents discover system-level services via explicit URLs. No automatic discovery or magic defaults. Users specify:

- `orchestratorUrl` - URL to system orchestrator (e.g., `http://localhost:5000`)
- `embeddingUrl` - URL to system embedding server (e.g., `http://localhost:8200/v1`)
- `redisUrl` - URL to system Redis (e.g., `redis://localhost:6379`)

This keeps configuration explicit and transparent.

## Secrets Handling

Secrets (like Discord bot tokens) are handled via file paths:

- User provides path to secret file (e.g., `tokenFile = "/run/secrets/discord-token"`)
- User manages the secret file themselves (via agenix, sops-nix, or manual placement)
- Module passes the path to the binary via `--token-file` argument

This works with any secrets manager without requiring module changes.

## Service Dependencies

### System Services (NixOS)

```
network.target
    ↓
┌───────────────────────────────────────┐
│ river-orchestrator.service            │
│ river-embedding.service               │
│ redis-river.service                   │
│ (all independent, start in parallel)  │
└───────────────────────────────────────┘
```

### User Services (home-manager)

```
network.target + default.target
    ↓
river-{name}-gateway.service
    ↓ (After, BindsTo)
river-{name}-discord.service
```

The Discord adapter binds to its gateway - if the gateway stops or restarts, the adapter does too.

## Systemd Service Configuration

### Common Settings

All services use:
- `Restart = "on-failure"`
- `RestartSec = 5`
- Structured logging via systemd journal

### System Services

- `DynamicUser = true` - No manual user creation needed
- `StateDirectory` - Managed state under `/var/lib/`
- `WantedBy = ["multi-user.target"]` - Start at boot

### User Services

- Run as the user (no special user config)
- State in user-specified `dataDir`
- `WantedBy = ["default.target"]` - Start on login

## Full Example

### System Configuration (configuration.nix)

```nix
{ config, pkgs, ... }:

{
  imports = [ /path/to/river-engine/nix/nixos-module.nix ];

  services.river = {
    orchestrator = {
      enable = true;
      port = 5000;
      modelDirs = [ /data/models ];
      cudaSupport = true;
    };

    embedding = {
      enable = true;
      port = 8200;
      modelPath = /data/models/nomic-embed-text-v1.5.Q8_0.gguf;
      cudaSupport = true;
    };

    redis.enable = true;
  };
}
```

### User Configuration (home.nix)

```nix
{ config, pkgs, ... }:

{
  imports = [ /path/to/river-engine/nix/home-module.nix ];

  services.river.agents.myagent = {
    enable = true;
    workspace = "${config.home.homeDirectory}/agents/myagent";
    dataDir = "${config.xdg.dataHome}/river/myagent";
    port = 3000;

    modelUrl = "http://localhost:8080/v1";
    modelName = "qwen3-32b-q4_k_m";
    orchestratorUrl = "http://localhost:5000";
    embeddingUrl = "http://localhost:8200/v1";
    redisUrl = "redis://localhost:6379";

    discord = {
      enable = true;
      tokenFile = "${config.xdg.configHome}/secrets/discord-token";
      guildId = "123456789012345678";
    };
  };
}
```

## Testing Strategy

### Unit Tests
- Option type validation (invalid ports, missing required fields)
- Service generation (correct CLI args, dependencies)

### Integration Tests
- NixOS VM test: system services start and respond to health checks
- Home-manager activation: user services generate correctly

### Manual Testing
- Full deployment with actual model and Discord bot
- Service restart behavior
- CUDA acceleration verification

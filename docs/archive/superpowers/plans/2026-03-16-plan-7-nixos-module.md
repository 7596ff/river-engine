# NixOS Module Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create NixOS and home-manager modules for deploying the complete River Engine stack.

**Architecture:** Shared library (`lib.nix`) defines option types and service builders. Both `nixos-module.nix` and `home-module.nix` import this library and provide identical functionality, differing only in systemd target types. `packages.nix` defines Rust package builds with optional CUDA support.

**Tech Stack:** Nix module system, systemd services, rustPlatform.buildRustPackage

---

## Chunk 1: Foundation

### Task 1: Create nix directory and packages.nix

**Files:**
- Create: `nix/packages.nix`

- [ ] **Step 1: Create nix directory**

```bash
mkdir -p nix
```

- [ ] **Step 2: Write packages.nix**

```nix
# nix/packages.nix
# Package definitions for River Engine binaries
{ pkgs, cudaSupport ? false }:

let
  llama-cpp = pkgs.llama-cpp.override {
    inherit cudaSupport;
  };

  src = ./..;

  commonBuildInputs = with pkgs; [ openssl ];
  commonNativeBuildInputs = with pkgs; [ pkg-config ];

in {
  inherit llama-cpp;

  river-gateway = pkgs.rustPlatform.buildRustPackage {
    pname = "river-gateway";
    version = "0.1.0";
    inherit src;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-gateway" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };

  river-orchestrator = pkgs.rustPlatform.buildRustPackage {
    pname = "river-orchestrator";
    version = "0.1.0";
    inherit src;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-orchestrator" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };

  river-discord = pkgs.rustPlatform.buildRustPackage {
    pname = "river-discord";
    version = "0.1.0";
    inherit src;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-discord" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };
}
```

- [ ] **Step 3: Verify syntax**

Run: `nix-instantiate --parse nix/packages.nix`
Expected: No errors (outputs parsed expression)

- [ ] **Step 4: Commit**

```bash
git add nix/packages.nix
git commit -m "feat(nix): add package definitions for River binaries"
```

---

### Task 2: Create lib.nix with option definitions

**Files:**
- Create: `nix/lib.nix`

- [ ] **Step 1: Write lib.nix with option types**

```nix
# nix/lib.nix
# Shared option types and service builders for River Engine modules
{ lib }:

let
  inherit (lib) mkOption mkEnableOption types;

in {
  # Reusable option type for service URLs
  serviceUrlOption = mkOption {
    type = types.nullOr types.str;
    default = null;
    description = "URL to the service. If null, feature is disabled.";
  };

  # Orchestrator options
  orchestratorOptions = {
    enable = mkEnableOption "River orchestrator";

    port = mkOption {
      type = types.port;
      default = 5000;
      description = "Port for the orchestrator API.";
    };

    healthThreshold = mkOption {
      type = types.int;
      default = 120;
      description = "Health threshold in seconds.";
    };

    modelDirs = mkOption {
      type = types.listOf types.path;
      default = [];
      description = "Directories to scan for GGUF models.";
    };

    externalModelsFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "Path to external models config JSON file.";
    };

    modelsConfigFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "Path to legacy models config JSON file.";
    };

    llamaServerPath = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "Path to llama-server binary. If null, uses package default.";
    };

    idleTimeout = mkOption {
      type = types.int;
      default = 900;
      description = "Idle timeout in seconds before unloading models.";
    };

    portRange = mkOption {
      type = types.str;
      default = "8080-8180";
      description = "Port range for llama-server instances (start-end).";
    };

    reserveVramMb = mkOption {
      type = types.int;
      default = 500;
      description = "Reserved VRAM in MB.";
    };

    reserveRamMb = mkOption {
      type = types.int;
      default = 2000;
      description = "Reserved RAM in MB.";
    };

    cudaSupport = mkOption {
      type = types.bool;
      default = false;
      description = "Enable CUDA support for llama-cpp.";
    };

    environment = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Extra environment variables.";
    };
  };

  # Embedding server options
  embeddingOptions = {
    enable = mkEnableOption "River embedding server";

    port = mkOption {
      type = types.port;
      default = 8200;
      description = "Port for the embedding server.";
    };

    modelPath = mkOption {
      type = types.path;
      description = "Path to the embedding model GGUF file.";
    };

    gpuLayers = mkOption {
      type = types.int;
      default = 99;
      description = "Number of GPU layers to use.";
    };

    cudaSupport = mkOption {
      type = types.bool;
      default = false;
      description = "Enable CUDA support for llama-cpp.";
    };
  };

  # Redis options
  redisOptions = {
    enable = mkEnableOption "Redis for River";

    port = mkOption {
      type = types.port;
      default = 6379;
      description = "Port for Redis server.";
    };
  };

  # Discord adapter options (nested in agent)
  discordOptions = {
    enable = mkEnableOption "Discord adapter";

    tokenFile = mkOption {
      type = types.path;
      description = "Path to file containing Discord bot token.";
    };

    guildId = mkOption {
      type = types.int;
      description = "Discord guild ID for slash command registration.";
    };

    gatewayUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Gateway URL. If null, derives from agent port.";
    };

    port = mkOption {
      type = types.port;
      default = 3002;
      description = "Port for adapter HTTP server.";
    };

    channels = mkOption {
      type = types.listOf types.int;
      default = [];
      description = "Initial Discord channel IDs to listen on.";
    };

    stateFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "Path to state file for channel persistence.";
    };
  };

  # Agent submodule options
  mkAgentOptions = { name, ... }: {
    enable = mkEnableOption "this River agent";

    workspace = mkOption {
      type = types.path;
      description = "Workspace directory for the agent.";
    };

    dataDir = mkOption {
      type = types.path;
      description = "Data directory for the agent database.";
    };

    agentName = mkOption {
      type = types.str;
      default = name;
      description = "Agent name (used for Redis namespacing).";
    };

    port = mkOption {
      type = types.port;
      default = 3000;
      description = "Gateway port.";
    };

    modelUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Model server URL.";
    };

    modelName = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Model name.";
    };

    orchestratorUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Orchestrator URL for heartbeats.";
    };

    embeddingUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Embedding server URL.";
    };

    redisUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Redis URL.";
    };

    environment = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Extra environment variables.";
    };
  };

  # Service builder: orchestrator
  mkOrchestratorCommand = { cfg, packages }: let
    llamaServer = if cfg.llamaServerPath != null
      then cfg.llamaServerPath
      else "${packages.llama-cpp}/bin/llama-server";
  in lib.concatStringsSep " " ([
    "${packages.river-orchestrator}/bin/river-orchestrator"
    "--port" (toString cfg.port)
    "--health-threshold" (toString cfg.healthThreshold)
    "--idle-timeout" (toString cfg.idleTimeout)
    "--llama-server-path" llamaServer
    "--port-range" cfg.portRange
    "--reserve-vram-mb" (toString cfg.reserveVramMb)
    "--reserve-ram-mb" (toString cfg.reserveRamMb)
  ] ++ lib.optionals (cfg.modelDirs != []) [
    "--model-dirs" (lib.concatStringsSep "," (map toString cfg.modelDirs))
  ] ++ lib.optionals (cfg.externalModelsFile != null) [
    "--external-models" (toString cfg.externalModelsFile)
  ] ++ lib.optionals (cfg.modelsConfigFile != null) [
    "--models-config" (toString cfg.modelsConfigFile)
  ]);

  # Service builder: embedding
  mkEmbeddingCommand = { cfg, packages }: lib.concatStringsSep " " [
    "${packages.llama-cpp}/bin/llama-server"
    "--embedding"
    "--model" (toString cfg.modelPath)
    "--port" (toString cfg.port)
    "--n-gpu-layers" (toString cfg.gpuLayers)
  ];

  # Service builder: gateway
  mkGatewayCommand = { cfg, packages }: lib.concatStringsSep " " ([
    "${packages.river-gateway}/bin/river-gateway"
    "--workspace" (toString cfg.workspace)
    "--data-dir" (toString cfg.dataDir)
    "--agent-name" cfg.agentName
    "--port" (toString cfg.port)
  ] ++ lib.optionals (cfg.modelUrl != null) [
    "--model-url" cfg.modelUrl
  ] ++ lib.optionals (cfg.modelName != null) [
    "--model-name" cfg.modelName
  ] ++ lib.optionals (cfg.orchestratorUrl != null) [
    "--orchestrator-url" cfg.orchestratorUrl
  ] ++ lib.optionals (cfg.embeddingUrl != null) [
    "--embedding-url" cfg.embeddingUrl
  ] ++ lib.optionals (cfg.redisUrl != null) [
    "--redis-url" cfg.redisUrl
  ]);

  # Service builder: discord
  mkDiscordCommand = { cfg, agentPort, packages }: let
    gatewayUrl = if cfg.gatewayUrl != null
      then cfg.gatewayUrl
      else "http://localhost:${toString agentPort}";
  in lib.concatStringsSep " " ([
    "${packages.river-discord}/bin/river-discord"
    "--token-file" (toString cfg.tokenFile)
    "--gateway-url" gatewayUrl
    "--listen-port" (toString cfg.port)
    "--guild-id" (toString cfg.guildId)
  ] ++ lib.optionals (cfg.channels != []) [
    "--channels" (lib.concatMapStringsSep "," toString cfg.channels)
  ] ++ lib.optionals (cfg.stateFile != null) [
    "--state-file" (toString cfg.stateFile)
  ]);
}
```

- [ ] **Step 2: Verify syntax**

Run: `nix-instantiate --parse nix/lib.nix`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add nix/lib.nix
git commit -m "feat(nix): add shared library with option types and service builders"
```

---

## Chunk 2: NixOS Module

### Task 3: Create nixos-module.nix

**Files:**
- Create: `nix/nixos-module.nix`

- [ ] **Step 1: Write nixos-module.nix**

```nix
# nix/nixos-module.nix
# NixOS module for River Engine system services
{ config, lib, pkgs, ... }:

let
  cfg = config.services.river;
  riverLib = import ./lib.nix { inherit lib; };
  packages = import ./packages.nix {
    inherit pkgs;
    cudaSupport = cfg.orchestrator.cudaSupport || cfg.embedding.cudaSupport;
  };

  # Common systemd service settings
  commonServiceConfig = {
    Restart = "on-failure";
    RestartSec = 5;
  };

in {
  options.services.river = {
    orchestrator = riverLib.orchestratorOptions;
    embedding = riverLib.embeddingOptions;
    redis = riverLib.redisOptions;

    agents = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule ({ name, ... }: {
        options = riverLib.mkAgentOptions { inherit name; } // {
          discord = riverLib.discordOptions;
        };
      }));
      default = {};
      description = "River agents to run as system services.";
    };
  };

  config = lib.mkMerge [
    # Orchestrator service
    (lib.mkIf cfg.orchestrator.enable {
      systemd.services.river-orchestrator = {
        description = "River Engine Orchestrator";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = commonServiceConfig // {
          DynamicUser = true;
          StateDirectory = "river-orchestrator";
          ExecStart = riverLib.mkOrchestratorCommand {
            cfg = cfg.orchestrator;
            inherit packages;
          };
        };

        environment = cfg.orchestrator.environment;
      };
    })

    # Embedding service
    (lib.mkIf cfg.embedding.enable {
      systemd.services.river-embedding = {
        description = "River Engine Embedding Server";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = commonServiceConfig // {
          DynamicUser = true;
          ExecStart = riverLib.mkEmbeddingCommand {
            cfg = cfg.embedding;
            packages = packages // {
              llama-cpp = (import ./packages.nix {
                inherit pkgs;
                cudaSupport = cfg.embedding.cudaSupport;
              }).llama-cpp;
            };
          };
        };
      };
    })

    # Redis via NixOS module
    (lib.mkIf cfg.redis.enable {
      services.redis.servers.river = {
        enable = true;
        port = cfg.redis.port;
      };
    })

    # Agent services
    (lib.mkMerge (lib.mapAttrsToList (name: agentCfg: lib.mkIf agentCfg.enable {
      # Gateway service
      systemd.services."river-${name}-gateway" = {
        description = "River Gateway - ${name}";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = commonServiceConfig // {
          DynamicUser = false;
          User = "river-${name}";
          Group = "river-${name}";
          StateDirectory = "river-${name}";
          ExecStart = riverLib.mkGatewayCommand {
            cfg = agentCfg;
            inherit packages;
          };
        };

        environment = agentCfg.environment;
      };

      # Create user for agent
      users.users."river-${name}" = {
        isSystemUser = true;
        group = "river-${name}";
        home = "/var/lib/river-${name}";
      };
      users.groups."river-${name}" = {};

      # Discord service (if enabled)
      systemd.services."river-${name}-discord" = lib.mkIf agentCfg.discord.enable {
        description = "River Discord Adapter - ${name}";
        wantedBy = [ "multi-user.target" ];
        after = [ "river-${name}-gateway.service" ];
        bindsTo = [ "river-${name}-gateway.service" ];

        serviceConfig = commonServiceConfig // {
          User = "river-${name}";
          Group = "river-${name}";
          ExecStart = riverLib.mkDiscordCommand {
            cfg = agentCfg.discord;
            agentPort = agentCfg.port;
            inherit packages;
          };
        };
      };
    }) cfg.agents))
  ];
}
```

- [ ] **Step 2: Verify syntax**

Run: `nix-instantiate --parse nix/nixos-module.nix`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add nix/nixos-module.nix
git commit -m "feat(nix): add NixOS module for system services"
```

---

## Chunk 3: Home-Manager Module

### Task 4: Create home-module.nix

**Files:**
- Create: `nix/home-module.nix`

- [ ] **Step 1: Write home-module.nix**

```nix
# nix/home-module.nix
# Home-manager module for River Engine user services
{ config, lib, pkgs, ... }:

let
  cfg = config.services.river;
  riverLib = import ./lib.nix { inherit lib; };
  packages = import ./packages.nix {
    inherit pkgs;
    cudaSupport = cfg.orchestrator.cudaSupport || cfg.embedding.cudaSupport;
  };

  # Common systemd user service settings
  commonServiceConfig = {
    Restart = "on-failure";
    RestartSec = 5;
  };

in {
  options.services.river = {
    orchestrator = riverLib.orchestratorOptions;
    embedding = riverLib.embeddingOptions;
    redis = riverLib.redisOptions;

    agents = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule ({ name, ... }: {
        options = riverLib.mkAgentOptions { inherit name; } // {
          discord = riverLib.discordOptions;
        };
      }));
      default = {};
      description = "River agents to run as user services.";
    };
  };

  config = lib.mkMerge [
    # Orchestrator service
    (lib.mkIf cfg.orchestrator.enable {
      systemd.user.services.river-orchestrator = {
        Unit = {
          Description = "River Engine Orchestrator";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = riverLib.mkOrchestratorCommand {
            cfg = cfg.orchestrator;
            inherit packages;
          };
          Environment = lib.mapAttrsToList (k: v: "${k}=${v}") cfg.orchestrator.environment;
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    })

    # Embedding service
    (lib.mkIf cfg.embedding.enable {
      systemd.user.services.river-embedding = {
        Unit = {
          Description = "River Engine Embedding Server";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = riverLib.mkEmbeddingCommand {
            cfg = cfg.embedding;
            packages = packages // {
              llama-cpp = (import ./packages.nix {
                inherit pkgs;
                cudaSupport = cfg.embedding.cudaSupport;
              }).llama-cpp;
            };
          };
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    })

    # Redis as user service
    (lib.mkIf cfg.redis.enable {
      systemd.user.services.river-redis = {
        Unit = {
          Description = "River Redis Server";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = "${pkgs.redis}/bin/redis-server --port ${toString cfg.redis.port} --dir %h/.local/share/river/redis";
          ExecStartPre = "${pkgs.coreutils}/bin/mkdir -p %h/.local/share/river/redis";
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    })

    # Agent services
    (lib.mkMerge (lib.mapAttrsToList (name: agentCfg: lib.mkIf agentCfg.enable {
      # Gateway service
      systemd.user.services."river-${name}-gateway" = {
        Unit = {
          Description = "River Gateway - ${name}";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = riverLib.mkGatewayCommand {
            cfg = agentCfg;
            inherit packages;
          };
          Environment = lib.mapAttrsToList (k: v: "${k}=${v}") agentCfg.environment;
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };

      # Discord service (if enabled)
      systemd.user.services."river-${name}-discord" = lib.mkIf agentCfg.discord.enable {
        Unit = {
          Description = "River Discord Adapter - ${name}";
          After = [ "river-${name}-gateway.service" ];
          BindsTo = [ "river-${name}-gateway.service" ];
        };

        Service = commonServiceConfig // {
          ExecStart = riverLib.mkDiscordCommand {
            cfg = agentCfg.discord;
            agentPort = agentCfg.port;
            inherit packages;
          };
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    }) cfg.agents))
  ];
}
```

- [ ] **Step 2: Verify syntax**

Run: `nix-instantiate --parse nix/home-module.nix`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add nix/home-module.nix
git commit -m "feat(nix): add home-manager module for user services"
```

---

## Chunk 4: Validation and Documentation

### Task 5: Validate module evaluation

**Files:**
- None (validation only)

- [ ] **Step 1: Create test evaluation file**

Create `nix/test-eval.nix` to test that modules evaluate correctly:

```nix
# nix/test-eval.nix
# Test that modules evaluate without errors
let
  pkgs = import <nixpkgs> {};
  lib = pkgs.lib;

  # Test NixOS module options exist
  nixosModule = import ./nixos-module.nix;
  homeModule = import ./home-module.nix;

  # Minimal test config
  testConfig = {
    services.river = {
      orchestrator.enable = true;
      embedding = {
        enable = true;
        modelPath = /tmp/test.gguf;
      };
      redis.enable = true;
      agents.test = {
        enable = true;
        workspace = /tmp/workspace;
        dataDir = /tmp/data;
      };
    };
  };

in {
  # Just verify the modules can be imported
  nixosModuleType = builtins.typeOf nixosModule;
  homeModuleType = builtins.typeOf homeModule;
  packagesType = builtins.typeOf (import ./packages.nix { inherit pkgs; });
  libType = builtins.typeOf (import ./lib.nix { inherit lib; });
}
```

- [ ] **Step 2: Run evaluation test**

Run: `nix-instantiate --eval nix/test-eval.nix --strict`
Expected: `{ homeModuleType = "lambda"; libType = "set"; nixosModuleType = "lambda"; packagesType = "set"; }`

- [ ] **Step 3: Clean up test file**

```bash
rm nix/test-eval.nix
```

- [ ] **Step 4: Commit all changes**

```bash
git add nix/
git commit -m "feat(nix): complete NixOS and home-manager module implementation"
```

---

### Task 6: Update STATUS.md

**Files:**
- Modify: `docs/superpowers/STATUS.md`

- [ ] **Step 1: Update STATUS.md**

Add Plan 7 to completed section:

```markdown
### Plan 7: NixOS Module ✅
- NixOS module (`nix/nixos-module.nix`):
  - Orchestrator, embedding, Redis, agents with Discord
  - System services with DynamicUser
  - Dedicated users for agents
- Home-manager module (`nix/home-module.nix`):
  - Identical functionality as user services
  - Redis as direct user service
- Shared library (`nix/lib.nix`):
  - Option type definitions
  - Command builders for all services
- Package definitions (`nix/packages.nix`):
  - CUDA-optional llama-cpp
  - All River binaries
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/STATUS.md
git commit -m "docs: mark Plan 7 NixOS module as complete"
```

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Package definitions | `nix/packages.nix` |
| 2 | Shared library | `nix/lib.nix` |
| 3 | NixOS module | `nix/nixos-module.nix` |
| 4 | Home-manager module | `nix/home-module.nix` |
| 5 | Validation | (temp test file) |
| 6 | Documentation | `docs/superpowers/STATUS.md` |

**Total new files:** 4
**Commits:** 6

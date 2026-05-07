# Nix Package & NixOS Module for River Engine

## Goal

A NixOS module and package that builds river-engine from local source and runs it as a systemd service. One `nixos-rebuild switch` gives you a running orchestrator with Redis, gateways, and adapters.

## Package

A single derivation that builds all river-engine binaries from a local source path.

```nix
rustPlatform.buildRustPackage {
  pname = "river-engine";
  version = "0.1.0";
  src = cfg.srcPath;
  cargoLock.lockFile = "${cfg.srcPath}/Cargo.lock";
}
```

Produces: `river-orchestrator`, `river-gateway`, `river-discord`, `river-migrate`.

`srcPath` is a module option — no hardcoded paths, no `fetchgit`. You point it at your local checkout and rebuild.

## Module Options

### Top-level

```nix
services.river-engine = {
  enable = lib.mkEnableOption "River Engine";

  srcPath = lib.mkOption {
    type = lib.types.path;
    description = "Path to river-engine source repository";
  };

  port = lib.mkOption {
    type = lib.types.port;
    default = 5000;
    description = "Orchestrator HTTP port";
  };

  user = lib.mkOption {
    type = lib.types.str;
    default = "cassie";
    description = "User to run the service as";
  };

  group = lib.mkOption {
    type = lib.types.str;
    default = "users";
    description = "Group to run the service as";
  };

  redis.port = lib.mkOption {
    type = lib.types.port;
    default = 6380;
    description = "Port for the dedicated Redis instance";
  };

  envFile = lib.mkOption {
    type = lib.types.nullOr lib.types.path;
    default = null;
    description = "Path to env file for non-secret variable expansion in config";
  };

  models = lib.mkOption { ... };    # see below
  agents = lib.mkOption { ... };    # see below
  resources = lib.mkOption { ... }; # see below, optional
};
```

### models

Named map of model backends. Each key is a model ID referenced by agents.

```nix
services.river-engine.models.<name> = {
  provider = lib.mkOption {
    type = lib.types.enum [ "ollama" "anthropic" "gguf" ];
    description = "Model backend type";
  };

  endpoint = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = "API endpoint URL";
  };

  name = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = "Model name";
  };

  api_key_file = lib.mkOption {
    type = lib.types.nullOr lib.types.path;
    default = null;
    description = "Path to file containing API key";
  };

  context_limit = lib.mkOption {
    type = lib.types.nullOr lib.types.int;
    default = null;
    description = "Context window size in tokens";
  };

  dimensions = lib.mkOption {
    type = lib.types.nullOr lib.types.int;
    default = null;
    description = "Embedding dimensions (embedding models only)";
  };

  path = lib.mkOption {
    type = lib.types.nullOr lib.types.path;
    default = null;
    description = "Path to GGUF model file (gguf provider only)";
  };
};
```

### agents

Named map of agents. Each agent becomes one `river-gateway` process managed by the orchestrator.

```nix
services.river-engine.agents.<name> = {
  workspace = lib.mkOption {
    type = lib.types.path;
    description = "Path to agent's workspace directory";
  };

  data_dir = lib.mkOption {
    type = lib.types.path;
    description = "Path to agent's runtime data directory (river.db, logs)";
  };

  port = lib.mkOption {
    type = lib.types.port;
    description = "Gateway HTTP port";
  };

  model = lib.mkOption {
    type = lib.types.str;
    description = "Key into models map for primary model";
  };

  spectator_model = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = "Key into models map for spectator. Defaults to model.";
  };

  embedding_model = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = "Key into models map for embeddings";
  };

  context = {
    limit = lib.mkOption {
      type = lib.types.int;
      default = 128000;
      description = "Total context window size in tokens";
    };

    compaction_threshold = lib.mkOption {
      type = lib.types.float;
      default = 0.80;
      description = "Fraction of context limit that triggers compaction";
    };

    fill_target = lib.mkOption {
      type = lib.types.float;
      default = 0.40;
      description = "Post-compaction fill target as fraction of limit";
    };

    min_messages = lib.mkOption {
      type = lib.types.int;
      default = 20;
      description = "Minimum messages always kept in context";
    };
  };

  auth_token_file = lib.mkOption {
    type = lib.types.nullOr lib.types.path;
    default = null;
    description = "Path to file containing gateway API bearer token";
  };

  log = {
    level = lib.mkOption {
      type = lib.types.enum [ "trace" "debug" "info" "warn" "error" ];
      default = "info";
      description = "Log level";
    };

    dir = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Log directory. Defaults to {data_dir}/logs/";
    };

    file = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Explicit log file path, overrides dir";
    };

    json_stdout = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Emit JSON logs to stdout";
    };
  };

  adapters = lib.mkOption {
    type = lib.types.attrsOf adapterSubmodule;
    default = {};
    description = "Adapter configurations for this agent";
  };
};
```

### agents.\<name\>.adapters

Generic adapter submodule. The `type` field determines which binary runs. Adapter-specific config goes in `settings`.

```nix
services.river-engine.agents.<name>.adapters.<name> = {
  type = lib.mkOption {
    type = lib.types.str;
    description = "Adapter type (determines binary: river-{type})";
  };

  port = lib.mkOption {
    type = lib.types.port;
    description = "Adapter HTTP port";
  };

  token_file = lib.mkOption {
    type = lib.types.nullOr lib.types.path;
    default = null;
    description = "Path to file containing adapter token";
  };

  settings = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = {};
    description = "Adapter-specific settings passed through to JSON config";
  };
};
```

### resources (optional)

```nix
services.river-engine.resources = {
  enable = lib.mkEnableOption "Local model resource management";

  reserve_vram_mb = lib.mkOption {
    type = lib.types.int;
    default = 500;
  };

  reserve_ram_mb = lib.mkOption {
    type = lib.types.int;
    default = 2000;
  };

  llama_server_path = lib.mkOption {
    type = lib.types.str;
    default = "llama-server";
  };

  port_range = lib.mkOption {
    type = lib.types.str;
    default = "8100-8200";
  };
};
```

Only included in the generated JSON when `resources.enable = true`.

## Config Generation

The module generates a JSON config file matching the orchestrator's expected format (as specified in the orchestrator config design doc). The JSON is written via `pkgs.writeText` and passed to `river-orchestrator --config`.

Redis URL is constructed from the module's Redis port and injected into every agent's config as `redis_url = "redis://127.0.0.1:${toString cfg.redis.port}"`.

Model references (`model`, `spectator_model`, `embedding_model`) are validated at eval time with assertions — referencing a model key that doesn't exist in `models` is a build error.

## Validation

Nix-level assertions:

- Every agent's `model` must reference a key in `models`
- Every agent's `spectator_model` (if set) must reference a key in `models`
- Every agent's `embedding_model` (if set) must reference a key in `models`
- No port conflicts across orchestrator, agents, adapters, and Redis
- `gguf` provider models must have `path` set
- `ollama` and `anthropic` provider models must have `endpoint` set

## Systemd

### redis-river-engine.service

Declared via `services.redis.servers.river-engine` with the configured port. Standard NixOS Redis module — no custom config needed beyond the port.

### river-engine.service

```ini
[Unit]
Description=River Engine Orchestrator
After=network.target redis-river-engine.service
Requires=redis-river-engine.service

[Service]
Type=simple
User=<cfg.user>
Group=<cfg.group>
ExecStart=<package>/bin/river-orchestrator --config <generated-json> --port <cfg.port>
Restart=on-failure
RestartSec=5
ReadWritePaths=<all agent data_dirs and workspaces>

[Install]
WantedBy=multi-user.target
```

`EnvironmentFile` is set if `cfg.envFile` is provided.

The orchestrator handles process supervision for gateways and adapters — the module does not create separate systemd units for them.

## Filesystem

`systemd.tmpfiles.rules` ensures each agent's `data_dir` exists with correct ownership.

Agent `workspace` directories are not created by the module — they are expected to already exist (your git repos).

## File Layout

Two files added to the NixOS config:

- `/etc/nixos/modules/river-engine.nix` — the module
- `/etc/nixos/packages/river-engine/package.nix` — the package

The flake input and module import in `/etc/nixos/flake.nix` are not managed by this spec — Cass wires those manually.

## Example Usage

```nix
services.river-engine = {
  enable = true;
  srcPath = /home/cassie/river-engine;
  port = 5000;
  user = "cassie";
  group = "users";
  redis.port = 6380;

  models = {
    gemma4 = {
      provider = "ollama";
      endpoint = "http://localhost:11434/v1";
      name = "gemma4:e2b";
      context_limit = 8192;
    };
    nomic-embed = {
      provider = "ollama";
      endpoint = "http://localhost:11434/v1";
      name = "nomic-embed-text";
      dimensions = 768;
    };
  };

  agents = {
    iris = {
      workspace = "/home/cassie/stream";
      data_dir = "/var/lib/river/iris";
      port = 3000;
      model = "gemma4";
      spectator_model = "gemma4";
      embedding_model = "nomic-embed";
      context = {
        limit = 8192;
        compaction_threshold = 0.80;
        fill_target = 0.40;
        min_messages = 20;
      };
      log.level = "debug";
      adapters = {
        discord = {
          type = "discord";
          port = 8081;
          token_file = "/home/cassie/.secrets/discord-token";
          settings = {
            guild_id = "1234567890";
            channels = [];
          };
        };
      };
    };
  };
};
```

## Out of Scope

- Flake integration (Cass wires the flake input manually)
- Agenix integration (secrets are managed as plain files)
- Agent birth automation (`river-gateway birth` is run manually)
- Hot-reloading config (restart the service to apply changes)
- Ollama module management (Ollama is assumed to be running independently)

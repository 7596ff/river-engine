# Nix Package & NixOS Module Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a NixOS module and package that compiles river-engine from local source and runs it as a systemd service with a dedicated Redis instance.

**Architecture:** Two files — a package derivation (`package.nix`) that builds the Rust workspace, and a NixOS module (`river-engine.nix`) that declares fully typed options, generates a JSON config matching `config_file.rs`, manages a dedicated Redis server, and runs the orchestrator via systemd. The module is imported in the host config, not via a flake input.

**Tech Stack:** Nix (NixOS module system), Rust (buildRustPackage), Redis, systemd

---

## File Structure

| File | Purpose |
|------|---------|
| `/etc/nixos/packages/river-engine/package.nix` | Rust package derivation — builds all binaries from local source |
| `/etc/nixos/modules/river-engine.nix` | NixOS module — options, config generation, systemd, Redis |
| `/etc/nixos/packages/default.nix` | Package overlay — add river-engine entry |
| `/etc/nixos/hosts/athena/default.nix` | Host config — import the module |

## Reference Files

These files define the JSON config shape the generated config must match. Read these before implementing:

- `/home/cassie/river-engine/crates/river-orchestrator/src/config_file.rs` — Rust deserialization types (source of truth for field names, types, and defaults)
- `/home/cassie/river-engine/deploy/river.example.json` — example config
- `/home/cassie/river-engine/docs/superpowers/specs/2026-05-03-orchestrator-config-design.md` — config design spec
- `/home/cassie/river-engine/docs/superpowers/specs/2026-05-07-nix-packaging-design.md` — this feature's design spec

## Critical JSON Shape Notes

From `config_file.rs` — the generated JSON must match these exactly:

- `adapters` is a **JSON array** (`Vec<AdapterConfig>`), not a map
- `channels` is `Vec<u64>` — channel IDs are integers, not strings
- `guild_id` is `Option<String>`
- `provider` is a free `String` in Rust, but the Nix module constrains it to an enum for validation
- Adapter `type` field maps to Rust's `adapter_type` (serde renames it)
- `bin` on adapters is `Option<PathBuf>` — omit from JSON to get the default `river-{type}`
- `resources` has a `Default` impl — omit the entire block to get defaults

---

### Task 1: Package Derivation

**Files:**
- Create: `/etc/nixos/packages/river-engine/package.nix`
- Modify: `/etc/nixos/packages/default.nix`

- [ ] **Step 1: Create package directory**

```bash
sudo mkdir -p /etc/nixos/packages/river-engine
```

- [ ] **Step 2: Write package.nix**

Create `/etc/nixos/packages/river-engine/package.nix`:

```nix
{ lib
, rustPlatform
, sqlite
, src ? /home/cassie/river-engine
}:

rustPlatform.buildRustPackage {
  pname = "river-engine";
  version = "0.1.0";
  inherit src;
  cargoLock.lockFile = "${src}/Cargo.lock";

  buildInputs = [ sqlite ];

  meta = {
    description = "Multi-agent orchestration system";
    license = lib.licenses.mit;
  };
}
```

The `src` parameter defaults to the local checkout but can be overridden by the module via `callPackage`. The `shell.nix` in the repo lists `sqlite` as a build input — `rusqlite` with `bundled` feature needs it.

- [ ] **Step 3: Update packages/default.nix**

Modify `/etc/nixos/packages/default.nix` — add the river-engine entry back:

```nix
{ pkgs }:
{
  letta-code = pkgs.callPackage ./letta-code/package.nix { };
  rarbg-selfhosted = pkgs.callPackage ./rarbg-selfhosted/package.nix { };
  river-engine = pkgs.callPackage ./river-engine/package.nix { };
  openclaw = pkgs.callPackage ./openclaw/package.nix { };
  whisper-timestamped = pkgs.callPackage ./whisper-timestamped/package.nix { };
}
```

- [ ] **Step 4: Test that the package evaluates**

```bash
cd /etc/nixos && sudo nix-build -E 'with import <nixpkgs> {}; callPackage ./packages/river-engine/package.nix {}'
```

Expected: Nix begins building the Rust workspace. This will take a while on first build (compiling all dependencies). It should produce `result/bin/river-orchestrator`, `result/bin/river-gateway`, `result/bin/river-discord`, `result/bin/river-migrate`.

If the build fails on linking, check whether additional `buildInputs` are needed (e.g., `openssl`, `pkg-config`). The `shell.nix` lists only `cargo rustc gcc sqlite nodejs_24` — the `bundled` feature on rusqlite means it compiles sqlite from C source via `cc`, which needs a C compiler (provided by stdenv).

- [ ] **Step 5: Commit**

```bash
cd /etc/nixos && sudo git add packages/river-engine/package.nix packages/default.nix
sudo git commit -m "feat(nix): add river-engine package derivation from local source"
```

---

### Task 2: Module — Option Declarations

**Files:**
- Create: `/etc/nixos/modules/river-engine.nix`

This task writes all the option declarations. No `config` block yet — just `options`.

- [ ] **Step 1: Create the module file with top-level options and model submodule**

Create `/etc/nixos/modules/river-engine.nix`:

```nix
{ config, lib, pkgs, ... }:

let
  cfg = config.services.river-engine;

  modelSubmodule = lib.types.submodule {
    options = {
      provider = lib.mkOption {
        type = lib.types.enum [ "ollama" "anthropic" "openai" "gguf" ];
        description = "Model backend type";
      };

      endpoint = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "API endpoint URL (required for ollama, anthropic, openai)";
      };

      name = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Model name at the endpoint";
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
        description = "Embedding dimensions (marks this as an embedding model)";
      };

      path = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to GGUF model file (gguf provider only)";
      };
    };
  };

  contextSubmodule = lib.types.submodule {
    options = {
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
  };

  logSubmodule = lib.types.submodule {
    options = {
      level = lib.mkOption {
        type = lib.types.enum [ "trace" "debug" "info" "warn" "error" ];
        default = "info";
        description = "Log level";
      };

      dir = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Log directory (defaults to {data_dir}/logs/)";
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
  };

  adapterSubmodule = lib.types.submodule {
    options = {
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
        description = "Adapter-specific settings (guild_id, channels, etc.)";
      };
    };
  };

  agentSubmodule = lib.types.submodule {
    options = {
      workspace = lib.mkOption {
        type = lib.types.path;
        description = "Path to agent's workspace directory";
      };

      data_dir = lib.mkOption {
        type = lib.types.path;
        description = "Path to agent's runtime data directory";
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
        description = "Key into models map for spectator (defaults to model)";
      };

      embedding_model = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Key into models map for embeddings";
      };

      context = lib.mkOption {
        type = contextSubmodule;
        default = {};
        description = "Context window configuration";
      };

      auth_token_file = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to file containing gateway API bearer token";
      };

      log = lib.mkOption {
        type = logSubmodule;
        default = {};
        description = "Logging configuration";
      };

      adapters = lib.mkOption {
        type = lib.types.attrsOf adapterSubmodule;
        default = {};
        description = "Adapter configurations for this agent";
      };
    };
  };

  resourcesSubmodule = lib.types.submodule {
    options = {
      enable = lib.mkEnableOption "local model resource management";

      reserve_vram_mb = lib.mkOption {
        type = lib.types.int;
        default = 500;
        description = "VRAM to keep free (MB)";
      };

      reserve_ram_mb = lib.mkOption {
        type = lib.types.int;
        default = 2000;
        description = "RAM to keep free (MB)";
      };

      llama_server_path = lib.mkOption {
        type = lib.types.str;
        default = "llama-server";
        description = "Path to llama-server binary";
      };

      port_range = lib.mkOption {
        type = lib.types.str;
        default = "8100-8200";
        description = "Port range for managed llama-server instances (format: start-end)";
      };
    };
  };
in
{
  options.services.river-engine = {
    enable = lib.mkEnableOption "River Engine orchestrator";

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
      default = "cassie";
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
      description = "Path to env file for non-secret variable expansion";
    };

    models = lib.mkOption {
      type = lib.types.attrsOf modelSubmodule;
      default = {};
      description = "Named model backend configurations";
    };

    agents = lib.mkOption {
      type = lib.types.attrsOf agentSubmodule;
      default = {};
      description = "Named agent configurations";
    };

    resources = lib.mkOption {
      type = resourcesSubmodule;
      default = {};
      description = "Local model resource management (optional)";
    };
  };

  # config block will be added in Task 3
}
```

- [ ] **Step 2: Verify the module evaluates with no config**

```bash
cd /etc/nixos && sudo nix eval --impure --expr '
  let
    pkgs = import <nixpkgs> {};
    module = import ./modules/river-engine.nix;
    eval = pkgs.lib.evalModules {
      modules = [
        module
        { services.river-engine.enable = false; }
      ];
    };
  in
    eval.config.services.river-engine.enable
'
```

Expected: `false`

This may produce a warning or error if the module system expects more context. If it does, that's fine — the real test is importing it in the host config (Task 4). At minimum, check there are no syntax errors:

```bash
nix-instantiate --parse /etc/nixos/modules/river-engine.nix > /dev/null
```

Expected: no output (clean parse).

- [ ] **Step 3: Commit**

```bash
cd /etc/nixos && sudo git add modules/river-engine.nix
sudo git commit -m "feat(nix): river-engine module option declarations"
```

---

### Task 3: Module — Config Generation, Validation, and Systemd

**Files:**
- Modify: `/etc/nixos/modules/river-engine.nix`

This task adds the `config` block: assertions, JSON generation, Redis, systemd service, and tmpfiles.

- [ ] **Step 1: Add the config block**

Add the following `config` block to `/etc/nixos/modules/river-engine.nix`, replacing the `# config block will be added in Task 3` comment:

```nix
  config = lib.mkIf cfg.enable (
    let
      package = pkgs.callPackage ../packages/river-engine/package.nix {
        src = cfg.srcPath;
      };

      redisUrl = "redis://127.0.0.1:${toString cfg.redis.port}";

      # Build the JSON config matching config_file.rs types exactly
      generatedConfig = {
        port = cfg.port;

        models = lib.mapAttrs (name: model:
          lib.filterAttrs (k: v: v != null) {
            provider = model.provider;
            endpoint = model.endpoint;
            name = model.name;
            api_key_file = if model.api_key_file != null
              then toString model.api_key_file else null;
            context_limit = model.context_limit;
            dimensions = model.dimensions;
            path = if model.path != null
              then toString model.path else null;
          }
        ) cfg.models;

        agents = lib.mapAttrs (name: agent: {
          workspace = toString agent.workspace;
          data_dir = toString agent.data_dir;
          port = agent.port;
          model = agent.model;
          redis_url = redisUrl;
          context = {
            inherit (agent.context) limit min_messages;
            compaction_threshold = agent.context.compaction_threshold;
            fill_target = agent.context.fill_target;
          };
          log = lib.filterAttrs (k: v: v != null) {
            level = agent.log.level;
            dir = if agent.log.dir != null then toString agent.log.dir else null;
            file = if agent.log.file != null then toString agent.log.file else null;
            json_stdout = agent.log.json_stdout;
          };
          adapters = lib.mapAttrsToList (aname: adapter:
            lib.filterAttrs (k: v: v != null) ({
              type = adapter.type;
              bin = "${package}/bin/river-${adapter.type}";
              port = adapter.port;
              token_file = if adapter.token_file != null
                then toString adapter.token_file else null;
            } // lib.optionalAttrs (adapter.settings ? guild_id) {
              guild_id = adapter.settings.guild_id;
            } // lib.optionalAttrs (adapter.settings ? channels) {
              channels = adapter.settings.channels;
            })
          ) agent.adapters;
        } // lib.optionalAttrs (agent.spectator_model != null) {
          spectator_model = agent.spectator_model;
        } // lib.optionalAttrs (agent.embedding_model != null) {
          embedding_model = agent.embedding_model;
        } // lib.optionalAttrs (agent.auth_token_file != null) {
          auth_token_file = toString agent.auth_token_file;
        }) cfg.agents;
      } // lib.optionalAttrs cfg.resources.enable {
        resources = {
          inherit (cfg.resources) reserve_vram_mb reserve_ram_mb port_range;
          llama_server_path = cfg.resources.llama_server_path;
        };
      };

      configFile = pkgs.writeText "river-config.json"
        (builtins.toJSON generatedConfig);

      # Collect all ReadWritePaths
      agentPaths = lib.flatten (lib.mapAttrsToList (name: agent: [
        (toString agent.data_dir)
        (toString agent.workspace)
      ]) cfg.agents);
    in
    {
      # --- Assertions ---
      assertions =
        # Model reference validation
        lib.flatten (lib.mapAttrsToList (agentName: agent:
          [
            {
              assertion = cfg.models ? ${agent.model};
              message = "river-engine agent '${agentName}' references undefined model '${agent.model}'";
            }
          ]
          ++ lib.optional (agent.spectator_model != null) {
            assertion = cfg.models ? ${agent.spectator_model};
            message = "river-engine agent '${agentName}' references undefined spectator_model '${agent.spectator_model}'";
          }
          ++ lib.optional (agent.embedding_model != null) {
            assertion = cfg.models ? ${agent.embedding_model};
            message = "river-engine agent '${agentName}' references undefined embedding_model '${agent.embedding_model}'";
          }
        ) cfg.agents)

        # Provider-specific validation
        ++ lib.flatten (lib.mapAttrsToList (modelName: model: [
          {
            assertion = model.provider != "gguf" || model.path != null;
            message = "river-engine model '${modelName}' has provider 'gguf' but no path set";
          }
          {
            assertion = !(builtins.elem model.provider [ "ollama" "anthropic" "openai" ]) || model.endpoint != null;
            message = "river-engine model '${modelName}' has provider '${model.provider}' but no endpoint set";
          }
        ]) cfg.models)

        # Port conflict detection
        ++ (
          let
            allPorts = [
              { port = cfg.port; label = "orchestrator"; }
              { port = cfg.redis.port; label = "redis"; }
            ]
            ++ lib.mapAttrsToList (name: agent:
              { port = agent.port; label = "agent '${name}'"; }
            ) cfg.agents
            ++ lib.flatten (lib.mapAttrsToList (agentName: agent:
              lib.mapAttrsToList (adapterName: adapter:
                { port = adapter.port; label = "adapter '${agentName}/${adapterName}'"; }
              ) agent.adapters
            ) cfg.agents);

            portNumbers = map (p: p.port) allPorts;
            uniquePorts = lib.unique portNumbers;
          in
          [{
            assertion = builtins.length portNumbers == builtins.length uniquePorts;
            message = "river-engine: port conflict detected among configured ports";
          }]
        );

      # --- Redis ---
      services.redis.servers.river-engine = {
        enable = true;
        port = cfg.redis.port;
      };

      # --- Systemd Service ---
      systemd.services.river-engine = {
        description = "River Engine Orchestrator";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" "redis-river-engine.service" ];
        requires = [ "redis-river-engine.service" ];

        serviceConfig = {
          Type = "simple";
          User = cfg.user;
          Group = cfg.group;

          ExecStart = lib.concatStringsSep " " ([
            "${package}/bin/river-orchestrator"
            "--config ${configFile}"
            "--port ${toString cfg.port}"
          ] ++ lib.optional (cfg.envFile != null)
            "--env-file ${toString cfg.envFile}"
          );

          Restart = "on-failure";
          RestartSec = 5;
          StartLimitBurst = 3;
          StartLimitIntervalSec = 60;

          ReadWritePaths = agentPaths;
        };
      };

      # --- Filesystem ---
      systemd.tmpfiles.rules = lib.mapAttrsToList (name: agent:
        "d ${toString agent.data_dir} 0750 ${cfg.user} ${cfg.group} -"
      ) cfg.agents;
    }
  );
```

- [ ] **Step 2: Verify no syntax errors**

```bash
nix-instantiate --parse /etc/nixos/modules/river-engine.nix > /dev/null
```

Expected: no output (clean parse).

- [ ] **Step 3: Commit**

```bash
cd /etc/nixos && sudo git add modules/river-engine.nix
sudo git commit -m "feat(nix): river-engine module config generation, validation, and systemd"
```

---

### Task 4: Wire Into Host Config

**Files:**
- Modify: `/etc/nixos/hosts/athena/default.nix`

- [ ] **Step 1: Import the module and add river-engine configuration**

Edit `/etc/nixos/hosts/athena/default.nix` to uncomment the module import and add the service config:

```nix
{ lib, pkgs, ... }:
{
  imports = [
    ./desktop.nix
    ./filesystems.nix
    ./network.nix
    ./system.nix
    ./user.nix

    ../../modules/river-engine.nix
  ];

  time.timeZone = "America/New_York";
  i18n.defaultLocale = "en_US.UTF-8";

  hardware.nvidia.open = true;

  networking.hostName = "athena";
  networking.hostId = "09a1d49f";

  # age.secrets.river-env = {
  #   file = ../../secrets/river-env.age;
  #   mode = "700";
  #   owner = "river";
  # };

  age.secrets.http-basic-auth = {
    file = ../../secrets/http-basic-auth.age;
    mode = "700";
    owner = "nginx";
  };

  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];

  nix.settings.extra-sandbox-paths = [ "${pkgs.git}" "${pkgs.openssh}" ];

  nixpkgs.config.allowUnfreePredicate =
    pkg:
    builtins.elem (lib.getName pkg) [
      "nvidia-x11"
      "rarbg-selfhosted"
      "steam"
      "steam-unwrapped"
      "nvidia-settings"
      "gazelle-origin"
      "discord"
      "cuda12.8-cuda_cccl-12.8.90"
      "cuda_cccl"
      "cuda_cudart"
      "cuda_nvcc"
      "libcublas"
      "claude-code"
      "obsidian"
    ];

  services.river-engine = {
    enable = true;
    srcPath = /home/cassie/river-engine;
    port = 5000;
    user = "cassie";
    group = "cassie";
    redis.port = 6380;

    models = {
      gemma4 = {
        provider = "ollama";
        endpoint = "http://localhost:11434/v1";
        name = "gemma4:e2b";
        context_limit = 8192;
      };
    };

    agents = {
      iris = {
        workspace = /home/cassie/stream;
        data_dir = /var/lib/river/iris;
        port = 3000;
        model = "gemma4";
        spectator_model = "gemma4";
        context = {
          limit = 8192;
          compaction_threshold = 0.80;
          fill_target = 0.40;
          min_messages = 20;
        };
        log.level = "debug";
      };
    };
  };

  security.sudo.wheelNeedsPassword = true;

  system.stateVersion = "25.11";
}
```

Note: no adapters or embedding model configured yet — starting with just the gateway and a local model for testing.

- [ ] **Step 2: Dry-run build to check for eval errors**

```bash
sudo nixos-rebuild dry-build
```

Expected: Nix evaluates the config and begins building. If there are type errors, missing options, or assertion failures, they will show here. Fix any issues before proceeding.

Common issues to watch for:
- Path types: Nix paths (`/foo/bar`) vs strings (`"/foo/bar"`) — the module uses `toString` where the JSON needs strings
- Redis module: ensure `services.redis.servers` is available on nixpkgs 25.11 (it should be)
- Package build: first build will compile all Rust dependencies

- [ ] **Step 3: Inspect the generated JSON config**

After a successful dry-build, find and inspect the generated config:

```bash
cat $(find /nix/store -name "river-config.json" -newer /etc/nixos/modules/river-engine.nix 2>/dev/null | head -1)
```

Verify it matches the expected structure from `config_file.rs`:
- `port` is an integer
- `models` is a map with string keys
- `agents` is a map with string keys
- `agents.iris.adapters` is an empty array `[]`
- `agents.iris.redis_url` is `"redis://127.0.0.1:6380"`
- `agents.iris.context` has all four fields
- No `resources` block (since we didn't enable it)
- No null values in the output (filtered by `lib.filterAttrs`)

- [ ] **Step 4: Commit**

```bash
cd /etc/nixos && sudo git add hosts/athena/default.nix
sudo git commit -m "feat(nix): wire river-engine module into athena with ollama config"
```

---

### Task 5: Build, Switch, and Verify

**Files:** none (this is a deploy and verification task)

- [ ] **Step 1: Build and switch**

```bash
sudo nixos-rebuild switch
```

Expected: system rebuilds. The river-engine package compiles (first build takes several minutes). Redis starts. The river-engine service starts (and may fail if there's no `river.db` with a birth memory yet — this is expected).

- [ ] **Step 2: Verify Redis is running**

```bash
redis-cli -p 6380 ping
```

Expected: `PONG`

If `redis-cli` is not in PATH, try:

```bash
nix-shell -p redis --run "redis-cli -p 6380 ping"
```

- [ ] **Step 3: Check the service status**

```bash
systemctl status river-engine.service
```

Expected: The service has started. It may have failed if the orchestrator can't find a birth memory for the iris agent — check the logs:

```bash
journalctl -u river-engine.service -n 50 --no-pager
```

If the log shows a birth memory error, that's correct — we need to run `river-gateway birth` first. If the log shows a config parsing error, the generated JSON doesn't match what the Rust code expects — inspect the JSON (see Task 4 Step 3) and fix.

- [ ] **Step 4: Run the gateway birth if needed**

If the orchestrator reports a missing birth memory:

```bash
# Find the built package
RIVER_PKG=$(find /nix/store -maxdepth 1 -name "*river-engine*" -type d | grep -v ".drv" | sort | tail -1)
$RIVER_PKG/bin/river-gateway birth --data-dir /var/lib/river/iris --name iris
```

Then restart the service:

```bash
sudo systemctl restart river-engine.service
```

- [ ] **Step 5: Verify end-to-end**

```bash
# Service should be active
systemctl is-active river-engine.service

# Orchestrator should be responding
curl -s http://127.0.0.1:5000/health || echo "no health endpoint yet — check logs"

# Gateway should be responding
curl -s http://127.0.0.1:3000/health || echo "no health endpoint yet — check logs"

# Redis should have the river-engine database
nix-shell -p redis --run "redis-cli -p 6380 info keyspace"
```

- [ ] **Step 6: Commit any fixes**

If any changes were needed to the module or package during verification:

```bash
cd /etc/nixos && sudo git add -A
sudo git commit -m "fix(nix): adjustments from first river-engine deploy"
```

---

### Task 6: Assertion Testing

**Files:** none (manual verification)

Verify that the Nix-level validations catch errors correctly. These are destructive tests — make each change, run `sudo nixos-rebuild dry-build`, verify it fails with the expected message, then revert.

- [ ] **Step 1: Test undefined model reference**

Temporarily change the agent's model to a nonexistent key:

In `/etc/nixos/hosts/athena/default.nix`, change `model = "gemma4";` to `model = "nonexistent";`.

```bash
sudo nixos-rebuild dry-build 2>&1 | grep "river-engine"
```

Expected: assertion failure mentioning `references undefined model 'nonexistent'`.

Revert the change.

- [ ] **Step 2: Test port conflict**

Temporarily set the agent port to match the orchestrator port:

Change `port = 3000;` to `port = 5000;` in the iris agent config.

```bash
sudo nixos-rebuild dry-build 2>&1 | grep "river-engine"
```

Expected: assertion failure mentioning port conflict.

Revert the change.

- [ ] **Step 3: Test gguf without path**

Temporarily add a gguf model without a path:

```nix
broken-model = {
  provider = "gguf";
  context_limit = 32000;
};
```

```bash
sudo nixos-rebuild dry-build 2>&1 | grep "river-engine"
```

Expected: assertion failure mentioning `provider 'gguf' but no path set`.

Revert the change.

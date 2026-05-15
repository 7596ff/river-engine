{ config, lib, pkgs, defaultPackage ? null, ... }:

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

      api_key_env = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Environment variable name for API key (e.g. DEEPSEEK_API_KEY)";
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

      token_env = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Environment variable name for adapter token (e.g. DISCORD_TOKEN)";
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

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPackage;
      description = "The river-engine package to use (defaults to the flake's package)";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 9253;
      description = "Orchestrator HTTP port";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "river";
      description = "User to run the service as";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "river";
      description = "Group to run the service as";
    };

    redis.port = lib.mkOption {
      type = lib.types.port;
      default = 9254;
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

  config = lib.mkIf cfg.enable (
    let
      package = cfg.package;

      binPrefix = "${package}/bin";

      redisUrl = "redis://127.0.0.1:${toString cfg.redis.port}";

      # Build the JSON config matching config_file.rs types exactly
      generatedConfig = {
        port = cfg.port;

        models = lib.mapAttrs (name: model:
          lib.filterAttrs (k: v: v != null) {
            provider = model.provider;
            endpoint = model.endpoint;
            name = model.name;
            api_key_env = model.api_key_env;
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
              bin = "${binPrefix}/river-${adapter.type}";
              port = adapter.port;
              token_file = if adapter.token_file != null
                then toString adapter.token_file else null;
              token_env = adapter.token_env;
            } // lib.optionalAttrs (adapter.settings ? guild_id) {
              guild_id = toString adapter.settings.guild_id;
            } // lib.optionalAttrs (adapter.settings ? channels) {
              channels = map toString adapter.settings.channels;
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

        startLimitBurst = 5;
        startLimitIntervalSec = 60;

        path = with pkgs; [ bash coreutils findutils gnused gnugrep gawk ];

        serviceConfig = {
          Type = "simple";
          User = cfg.user;
          Group = cfg.group;

          ExecStart = lib.concatStringsSep " " ([
            "${binPrefix}/river-orchestrator"
            "--config ${configFile}"
            "--port ${toString cfg.port}"
          ] ++ lib.optional (cfg.envFile != null)
            "--env-file ${toString cfg.envFile}"
          );

          Restart = "on-failure";
          RestartSec = 5;

          ReadWritePaths = agentPaths;
        } // lib.optionalAttrs (cfg.envFile != null) {
          EnvironmentFile = toString cfg.envFile;
        };
      };

      # --- User/Group ---
      users.users.${cfg.user} = lib.mkIf (cfg.user == "river") {
        isSystemUser = true;
        group = cfg.group;
        description = "River Engine service user";
      };

      users.groups.${cfg.group} = lib.mkIf (cfg.group == "river") {};

      # --- Filesystem ---
      # Data dirs are world-readable so the TUI can tail the home channel JSONL
      systemd.tmpfiles.rules = lib.mapAttrsToList (name: agent:
        "d ${toString agent.data_dir} 0755 ${cfg.user} ${cfg.group} -"
      ) cfg.agents;
    }
  );
}

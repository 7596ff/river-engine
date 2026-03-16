# River home-manager module — Multi-agent support
#
# Usage:
#   services.river = {
#     enable = true;
#     model.path = "/models/Qwen3.5-9B-Q4_K_M.gguf";
#     agents.thomas = {
#       workspace = "/home/thomas/workspace";
#       modelUrl = "http://127.0.0.1:8080";  # local model
#       gateway.port = 3000;
#       discord = { enable = true; tokenFile = "/path/to/token"; };
#     };
#     agents.thomas-claude = {
#       workspace = "/home/thomas/workspace-claude";
#       modelUrl = "http://127.0.0.1:4000";  # LiteLLM proxy
#       gateway.port = 3003;
#       discord = { enable = true; tokenFile = "/path/to/token2"; };
#     };
#   };
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.river;

  binDir = "/var/lib/thomas/bin";
  libDir = "/var/lib/thomas/lib";

  # Agent submodule type
  agentModule = lib.types.submodule ({ name, ... }: {
    options = {
      workspace = lib.mkOption {
        type = lib.types.str;
        description = "Workspace directory for this agent's context files";
      };

      dataDir = lib.mkOption {
        type = lib.types.str;
        default = "${config.home.homeDirectory}/.river-${name}";
        description = "State/data directory for this agent";
      };

      modelUrl = lib.mkOption {
        type = lib.types.str;
        default = "http://127.0.0.1:${toString cfg.model.port}";
        description = "URL of the model server (local llama.cpp or LiteLLM proxy)";
      };

      modelName = lib.mkOption {
        type = lib.types.str;
        default = "default";
        description = "Model name to pass to the gateway";
      };

      gateway.port = lib.mkOption {
        type = lib.types.port;
        default = 3000;
        description = "Port for this agent's gateway";
      };

      api.port = lib.mkOption {
        type = lib.types.port;
        default = 3001;
        description = "Port for this agent's API layer";
      };

      discord = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Enable Discord bot for this agent";
        };

        tokenFile = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Path to file containing RIVER_DISCORD_TOKEN=<token>";
        };

        apiPort = lib.mkOption {
          type = lib.types.port;
          default = 3002;
          description = "Port for Discord bot's control API (presence, etc.)";
        };

        extraEnv = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Extra environment variables for the Discord bot";
        };
      };

      heartbeat = {
        intervalSecs = lib.mkOption {
          type = lib.types.int;
          default = 540;
          description = "Heartbeat interval in seconds";
        };

        session = lib.mkOption {
          type = lib.types.str;
          default = "main";
          description = "Session ID for heartbeats";
        };
      };
    };
  });

in
{
  options.services.river = {
    enable = lib.mkEnableOption "River AI engine";

    # ── Shared infrastructure ──

    model = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Enable the local llama.cpp model server";
      };

      path = lib.mkOption {
        type = lib.types.str;
        default = "/models/Qwen3.5-9B-Q4_K_M.gguf";
        description = "Path to the GGUF model file";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 8080;
        description = "Port for llama-server";
      };

      gpuLayers = lib.mkOption {
        type = lib.types.int;
        default = 99;
        description = "Number of layers to offload to GPU";
      };

      contextSize = lib.mkOption {
        type = lib.types.int;
        default = 65536;
        description = "Context window size in tokens";
      };

      extraArgs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Extra arguments passed to llama-server";
      };
    };

    embedding = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Enable the embedding server for semantic memory";
      };

      path = lib.mkOption {
        type = lib.types.str;
        default = "/models/nomic-embed-text-v1.5.Q8_0.gguf";
        description = "Path to the embedding model GGUF file";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 8081;
        description = "Port for the embedding server";
      };
    };

    litellm = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Enable LiteLLM proxy for API-backed models";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 4000;
        description = "Port for LiteLLM proxy";
      };

      configFile = lib.mkOption {
        type = lib.types.str;
        default = "${config.home.homeDirectory}/litellm-config.yaml";
        description = "Path to LiteLLM config YAML";
      };

      envFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Path to env file with API keys";
      };

      venvPath = lib.mkOption {
        type = lib.types.str;
        default = "${config.home.homeDirectory}/litellm-env";
        description = "Path to LiteLLM Python virtualenv";
      };
    };

    # ── Per-agent configuration ──

    agents = lib.mkOption {
      type = lib.types.attrsOf agentModule;
      default = { };
      description = "Named agent configurations. Each gets its own gateway, API, and optional Discord bot.";
    };
  };

  config = lib.mkIf cfg.enable {

    systemd.user.services = lib.mkMerge ([
      # ── Shared services ──
      {
        river-model = lib.mkIf cfg.model.enable {
          Unit.Description = "River Model Runner (llama.cpp with CUDA)";
          Service = {
            Type = "simple";
            ExecStart = lib.concatStringsSep " " ([
              "${binDir}/llama-server"
              "-m ${cfg.model.path}"
              "--port ${toString cfg.model.port}"
              "-ngl ${toString cfg.model.gpuLayers}"
              "-c ${toString cfg.model.contextSize}"
              "--host 127.0.0.1"
            ] ++ cfg.model.extraArgs);
            Environment = [ "LD_LIBRARY_PATH=/run/opengl-driver/lib:${libDir}" ];
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        river-embedding = lib.mkIf cfg.embedding.enable {
          Unit.Description = "River Embedding Server (nomic-embed-text)";
          Service = {
            Type = "simple";
            ExecStart = lib.concatStringsSep " " [
              "${binDir}/llama-server"
              "-m ${cfg.embedding.path}"
              "--port ${toString cfg.embedding.port}"
              "--host 127.0.0.1"
              "--embedding"
              "-c 2048"
              "-ngl 0"
            ];
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        river-litellm = lib.mkIf cfg.litellm.enable {
          Unit.Description = "LiteLLM API Proxy";
          Service = {
            Type = "simple";
            ExecStart = "${cfg.litellm.venvPath}/bin/litellm --config ${cfg.litellm.configFile} --port ${toString cfg.litellm.port} --host 127.0.0.1";
            EnvironmentFile = lib.mkIf (cfg.litellm.envFile != null) cfg.litellm.envFile;
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };
      }
    ] ++ (lib.mapAttrsToList (name: agentCfg: {

      # Gateway for this agent
      "river-${name}-gateway" = {
        Unit = {
          Description = "River Gateway (${name})";
          After = [ "river-model.service" ];
          Wants = [ "river-model.service" ];
        };
        Service = {
          Type = "simple";
          ExecStart = "${binDir}/river-gateway";
          Environment = [
            "RIVER_GATEWAY_PORT=${toString agentCfg.gateway.port}"
            "RIVER_MODEL_URL=${agentCfg.modelUrl}"
            "RIVER_API_URL=http://127.0.0.1:${toString agentCfg.api.port}"
            "RIVER_MODEL_NAME=${agentCfg.modelName}"
            "RIVER_DB_PATH=${agentCfg.dataDir}/river.db"
            "RIVER_WORKSPACE=${agentCfg.workspace}"
          ] ++ lib.optionals cfg.embedding.enable [
            "RIVER_EMBEDDING_URL=http://127.0.0.1:${toString cfg.embedding.port}"
          ];
          WorkingDirectory = agentCfg.dataDir;
          Restart = "on-failure";
          RestartSec = 3;
        };
        Install.WantedBy = [ "default.target" ];
      };

      # API layer for this agent
      "river-${name}-api" = {
        Unit.Description = "River API Layer (${name})";
        Service = {
          Type = "simple";
          ExecStart = "${binDir}/river-api";
          Environment = [
            "RIVER_API_PORT=${toString agentCfg.api.port}"
          ];
          WorkingDirectory = agentCfg.dataDir;
          Restart = "on-failure";
          RestartSec = 3;
        };
        Install.WantedBy = [ "default.target" ];
      };

    } // lib.optionalAttrs agentCfg.discord.enable {

      # Discord bot for this agent
      "river-${name}-discord" = {
        Unit = {
          Description = "River Discord Bot (${name})";
          After = [ "river-${name}-gateway.service" ];
          Wants = [ "river-${name}-gateway.service" ];
        };
        Service = {
          Type = "simple";
          ExecStart = "${binDir}/river-discord";
          Environment = [
            "RIVER_GATEWAY_URL=http://127.0.0.1:${toString agentCfg.gateway.port}"
            "RIVER_HEARTBEAT_SECS=${toString agentCfg.heartbeat.intervalSecs}"
            "RIVER_HEARTBEAT_SESSION=${agentCfg.heartbeat.session}"
            "RIVER_DISCORD_API_PORT=${toString agentCfg.discord.apiPort}"
          ] ++ agentCfg.discord.extraEnv;
          EnvironmentFile = lib.mkIf (agentCfg.discord.tokenFile != null) agentCfg.discord.tokenFile;
          WorkingDirectory = agentCfg.dataDir;
          Restart = "on-failure";
          RestartSec = 5;
        };
        Install.WantedBy = [ "default.target" ];
      };

    }) cfg.agents));
  };
}

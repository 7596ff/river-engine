# nix/home-module.nix
# Home-manager module for River Engine user services
{ config, lib, pkgs, ... }:

let
  cfg = config.services.river;
  riverLib = import ./lib.nix { inherit lib; };

  # packages must be computed lazily (inside config) since it depends on cfg
  mkPackages = cudaSupport: import ./packages.nix {
    inherit pkgs;
    inherit cudaSupport;
  };

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
    (lib.mkIf cfg.orchestrator.enable (let
      packages = mkPackages cfg.orchestrator.cudaSupport;
    in {
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
    }))

    # Embedding service
    (lib.mkIf cfg.embedding.enable (let
      packages = mkPackages cfg.embedding.cudaSupport;
    in {
      systemd.user.services.river-embedding = {
        Unit = {
          Description = "River Engine Embedding Server";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = riverLib.mkEmbeddingCommand {
            cfg = cfg.embedding;
            inherit packages;
          };
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    }))

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
    (lib.mkMerge (lib.mapAttrsToList (name: agentCfg: lib.mkIf agentCfg.enable (let
      packages = mkPackages false;  # Agents don't need CUDA
    in {
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
    })) cfg.agents))
  ];
}

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

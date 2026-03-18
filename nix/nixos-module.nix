# nix/nixos-module.nix
# NixOS module for River Engine system services
{ config, lib, pkgs, ... }:

let
  riverLib = import ./lib.nix { inherit lib; };

  # packages must be computed lazily (inside config) since it depends on config
  mkPackages = cudaSupport: import ./packages.nix {
    inherit pkgs cudaSupport;
    src = config.services.river.package.src;
  };

  commonServiceConfig = {
    Restart = "on-failure";
    RestartSec = 5;
  };

  # Agent service generators - called lazily per agent
  mkAgentServices = name: agentCfg: let
    packages = mkPackages false;
  in lib.optionalAttrs agentCfg.enable {
    "river-${name}-gateway" = {
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
  } // lib.optionalAttrs (agentCfg.enable && agentCfg.discord.enable) {
    "river-${name}-discord" = {
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
  };

  mkAgentUsers = name: agentCfg: lib.optionalAttrs agentCfg.enable {
    "river-${name}" = {
      isSystemUser = true;
      group = "river-${name}";
      home = "/var/lib/river-${name}";
    };
  };

  mkAgentGroups = name: agentCfg: lib.optionalAttrs agentCfg.enable {
    "river-${name}" = {};
  };

in {
  options.services.river = {
    package = riverLib.packageOptions;
    orchestrator = riverLib.orchestratorOptions;
    embedding = riverLib.embeddingOptions;
    redis = riverLib.redisOptions;
    litellm = riverLib.litellmOptions;

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
    (lib.mkIf config.services.river.orchestrator.enable (let
      cfg = config.services.river.orchestrator;
      packages = mkPackages cfg.cudaSupport;
    in {
      systemd.services.river-orchestrator = {
        description = "River Engine Orchestrator";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = commonServiceConfig // {
          DynamicUser = true;
          StateDirectory = "river-orchestrator";
          ExecStart = riverLib.mkOrchestratorCommand {
            inherit cfg packages;
          };
        };

        environment = cfg.environment;
      };
    }))

    # Embedding service
    (lib.mkIf config.services.river.embedding.enable (let
      cfg = config.services.river.embedding;
      packages = mkPackages cfg.cudaSupport;
    in {
      systemd.services.river-embedding = {
        description = "River Engine Embedding Server";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = commonServiceConfig // {
          DynamicUser = true;
          ExecStart = riverLib.mkEmbeddingCommand {
            inherit cfg packages;
          };
        };
      };
    }))

    # Redis via NixOS module
    (lib.mkIf config.services.river.redis.enable (let
      cfg = config.services.river.redis;
    in {
      services.redis.servers.river = {
        enable = true;
        port = cfg.port;
      };
    }))

    # LiteLLM proxy service
    (lib.mkIf config.services.river.litellm.enable (let
      cfg = config.services.river.litellm;
      configFile = pkgs.writeText "litellm-config.yaml" (riverLib.mkLitellmConfig { inherit cfg; });
      python = pkgs.python3.withPackages (ps: with ps; [
        litellm
        # Web framework
        fastapi
        starlette
        uvicorn
        uvloop
        gunicorn
        # HTTP clients
        httpx
        aiohttp
        anyio
        websockets
        # AI providers
        openai
        anthropic
        tiktoken
        tokenizers
        # Data & serialization
        orjson
        pyyaml
        pydantic
        jinja2
        # Auth & crypto
        pyjwt
        cryptography
        # Utilities
        backoff
        tenacity
        apscheduler
        python-dotenv
        python-multipart
        click
        rich
        # Monitoring
        prometheus-client
      ]);
    in {
      systemd.services.river-litellm = {
        description = "River LiteLLM Proxy";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = commonServiceConfig // {
          DynamicUser = true;
          ExecStart = lib.concatStringsSep " " ([
            "${python}/bin/litellm"
            "--config" (toString configFile)
            "--port" (toString cfg.port)
          ] ++ cfg.extraArgs);
          EnvironmentFile = toString cfg.apiKeyFile;
        };
      };
    }))

    # Agent services - use mapAttrs for lazy evaluation
    {
      systemd.services = lib.mkMerge (lib.attrValues (lib.mapAttrs mkAgentServices config.services.river.agents));
      users.users = lib.mkMerge (lib.attrValues (lib.mapAttrs mkAgentUsers config.services.river.agents));
      users.groups = lib.mkMerge (lib.attrValues (lib.mapAttrs mkAgentGroups config.services.river.agents));
    }
  ];
}

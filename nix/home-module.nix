# nix/home-module.nix
# Home-manager module for River Engine user services
{ config, lib, pkgs, ... }:

let
  riverLib = import ./lib.nix { inherit lib; };
  mkPackages = cudaSupport: import ./packages.nix {
    inherit pkgs cudaSupport;
    src = config.services.river.package.src;
  };
  commonServiceConfig = {
    Restart = "on-failure";
    RestartSec = 5;
  };

  # Default PATH for shell commands
  defaultPath = lib.makeBinPath (with pkgs; [ bash coreutils curl pandoc ddgr git ]);

  # Agent service generator - called lazily per agent
  mkAgentServices = openrouterCfg: anthropicCfg: name: agentCfg: let
    packages = mkPackages false;
    gatewayCmd = riverLib.mkGatewayCommand {
      cfg = agentCfg;
      discordCfg = agentCfg.discord;
      inherit openrouterCfg anthropicCfg packages;
    };
  in lib.optionalAttrs agentCfg.enable {
    "river-${name}-gateway" = {
      Unit = {
        Description = "River Gateway - ${name}";
        After = [ "network.target" ];
      };
      Service = commonServiceConfig // {
        ExecStart = gatewayCmd;
        Environment = [ "HOME=%h" "PATH=${defaultPath}" ] ++ lib.mapAttrsToList (k: v: "${k}=${v}") agentCfg.environment;
      } // lib.optionalAttrs (agentCfg.environmentFile != null) {
        EnvironmentFile = toString agentCfg.environmentFile;
      };
      Install = {
        WantedBy = [ "default.target" ];
      };
    };
  } // lib.optionalAttrs (agentCfg.enable && agentCfg.discord.enable) {
    "river-${name}-discord" = {
      Unit = {
        Description = "River Discord Adapter - ${name}";
        After = [ "river-${name}-gateway.service" ];
        BindsTo = [ "river-${name}-gateway.service" ];
        PartOf = [ "river-${name}-gateway.service" ];
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
  };

in {
  options.services.river = {
    package = riverLib.packageOptions;
    orchestrator = riverLib.orchestratorOptions;
    embedding = riverLib.embeddingOptions;
    redis = riverLib.redisOptions;
    litellm = riverLib.litellmOptions;
    openrouter = riverLib.openrouterOptions;
    anthropic = riverLib.anthropicOptions;

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
    (lib.mkIf config.services.river.orchestrator.enable (let
      cfg = config.services.river.orchestrator;
      packages = mkPackages cfg.cudaSupport;
    in {
      systemd.user.services.river-orchestrator = {
        Unit = {
          Description = "River Engine Orchestrator";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = riverLib.mkOrchestratorCommand {
            inherit cfg packages;
          };
          Environment = lib.mapAttrsToList (k: v: "${k}=${v}") cfg.environment;
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    }))

    # Embedding service
    (lib.mkIf config.services.river.embedding.enable (let
      cfg = config.services.river.embedding;
      packages = mkPackages cfg.cudaSupport;
    in {
      systemd.user.services.river-embedding = {
        Unit = {
          Description = "River Engine Embedding Server";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = riverLib.mkEmbeddingCommand {
            inherit cfg packages;
          };
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    }))

    # Redis as user service
    (lib.mkIf config.services.river.redis.enable (let
      cfg = config.services.river.redis;
    in {
      systemd.user.services.river-redis = {
        Unit = {
          Description = "River Redis Server";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = "${pkgs.redis}/bin/redis-server --port ${toString cfg.port} --dir %h/.local/share/river/redis";
          ExecStartPre = "${pkgs.coreutils}/bin/mkdir -p %h/.local/share/river/redis";
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
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
        # SSO
        fastapi-sso
      ]);
    in {
      systemd.user.services.river-litellm = {
        Unit = {
          Description = "River LiteLLM Proxy";
          After = [ "network.target" ];
        };

        Service = commonServiceConfig // {
          ExecStart = lib.concatStringsSep " " ([
            "${python}/bin/litellm"
            "--config" (toString configFile)
            "--port" (toString cfg.port)
          ] ++ cfg.extraArgs);
          EnvironmentFile = toString cfg.apiKeyFile;
        };

        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    }))

    # Agent services - use mapAttrs for lazy evaluation
    {
      systemd.user.services = lib.mkMerge (lib.attrValues (lib.mapAttrs (mkAgentServices config.services.river.openrouter config.services.river.anthropic) config.services.river.agents));
    }
  ];
}

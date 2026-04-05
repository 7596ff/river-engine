{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.river-engine;
  settingsFormat = pkgs.formats.json { };

  # Generate orchestrator config file
  configFile = settingsFormat.generate "river.json" cfg.settings;
in
{
  options.services.river-engine = {
    enable = lib.mkEnableOption "River Engine multi-agent orchestration system";

    package = lib.mkPackageOption pkgs "river-engine" { };

    settings = lib.mkOption {
      type = settingsFormat.type;
      default = { };
      description = ''
        Configuration for the River Engine orchestrator.
        See example configuration for structure.
      '';
      example = lib.literalExpression ''
        {
          port = 4337;
          models = {
            "claude-sonnet" = {
              endpoint = "https://api.anthropic.com/v1/messages";
              name = "claude-sonnet-4-20250514";
              api_key = "$ANTHROPIC_API_KEY";
              context_limit = 200000;
            };
          };
          dyads = {
            river = {
              workspace = "/var/lib/river/workspaces/river";
              uid = 1000;
              gid = 1000;
              left = {
                name = "Iris";
                model = "claude-sonnet";
              };
              right = {
                name = "Viola";
                model = "claude-sonnet";
              };
              initialActor = "left";
              ground = {
                name = "Cassie";
                id = "123456789";
                adapter = "discord";
                channel = "$DISCORD_DM_CHANNEL_ID";
              };
              adapters = [
                {
                  path = "''${pkgs.river-engine}/bin/river-discord";
                  side = "left";
                  token = "$DISCORD_TOKEN";
                  guild_id = "987654321";
                }
              ];
            };
          };
        }
      '';
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Environment file containing secrets like API keys.
        Should contain lines like:
          ANTHROPIC_API_KEY=sk-...
          DISCORD_TOKEN=...
      '';
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "river";
      description = "User account under which river-engine runs.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "river";
      description = "Group under which river-engine runs.";
    };

    createUser = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Whether to create the river user and group automatically.";
    };

    uid = lib.mkOption {
      type = lib.types.nullOr lib.types.int;
      default = 400;
      description = "UID for the river service user. Set to null for auto-assignment.";
    };

    gid = lib.mkOption {
      type = lib.types.nullOr lib.types.int;
      default = 400;
      description = "GID for the river service group. Set to null for auto-assignment.";
    };

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/river";
      description = "Directory for river-engine data and workspaces.";
    };

    orchestrator = {
      enable = lib.mkEnableOption "River Engine orchestrator service" // {
        default = true;
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 4337;
        description = "Port for the orchestrator HTTP API.";
      };

      openFirewall = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Whether to open the orchestrator port in the firewall.";
      };
    };

    workers = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          enable = lib.mkEnableOption "this worker";

          dyad = lib.mkOption {
            type = lib.types.str;
            description = "Name of the dyad this worker belongs to.";
          };

          side = lib.mkOption {
            type = lib.types.enum [ "left" "right" ];
            description = "Which side of the dyad (left or right).";
          };

          port = lib.mkOption {
            type = lib.types.port;
            default = 0;
            description = "Port for worker HTTP server (0 = auto-assign).";
          };
        };
      });
      default = { };
      description = "Worker instances to run.";
      example = lib.literalExpression ''
        {
          "river-left" = {
            enable = true;
            dyad = "river";
            side = "left";
          };
          "river-right" = {
            enable = true;
            dyad = "river";
            side = "right";
          };
        }
      '';
    };

    adapters = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          enable = lib.mkEnableOption "this adapter";

          type = lib.mkOption {
            type = lib.types.enum [ "discord" "slack" "tui" ];
            description = "Adapter type.";
          };

          dyad = lib.mkOption {
            type = lib.types.str;
            description = "Name of the dyad this adapter serves.";
          };

          port = lib.mkOption {
            type = lib.types.port;
            default = 0;
            description = "Port for adapter HTTP server (0 = auto-assign).";
          };
        };
      });
      default = { };
      description = "Adapter instances to run.";
      example = lib.literalExpression ''
        {
          "river-discord" = {
            enable = true;
            type = "discord";
            dyad = "river";
          };
        }
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    # Create user and group (if createUser is true)
    users.users.${cfg.user} = lib.mkIf cfg.createUser {
      isSystemUser = true;
      group = cfg.group;
      home = cfg.dataDir;
      createHome = true;
      description = "River Engine service user";
      uid = lib.mkIf (cfg.uid != null) cfg.uid;
    };

    users.groups.${cfg.group} = lib.mkIf cfg.createUser (
      lib.optionalAttrs (cfg.gid != null) { gid = cfg.gid; }
    );

    # Systemd services (orchestrator, workers, adapters)
    systemd.services = {
      # Orchestrator service
      river-orchestrator = lib.mkIf cfg.orchestrator.enable {
        description = "River Engine Orchestrator";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = {
          Type = "simple";
          User = cfg.user;
          Group = cfg.group;
          WorkingDirectory = cfg.dataDir;
          ExecStart = "${cfg.package}/bin/river-orchestrator --config ${configFile} --port ${toString cfg.orchestrator.port}";
          Restart = "on-failure";
          RestartSec = "5s";

          # Hardening
          NoNewPrivileges = true;
          ProtectSystem = "strict";
          ProtectHome = true;
          PrivateTmp = true;
          ReadWritePaths = [ cfg.dataDir ];
        } // lib.optionalAttrs (cfg.environmentFile != null) {
          EnvironmentFile = cfg.environmentFile;
        };
      };
    } // (lib.mapAttrs' (name: workerCfg:
      # Worker services
      lib.nameValuePair "river-worker-${name}" (lib.mkIf workerCfg.enable {
        description = "River Engine Worker - ${name}";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" "river-orchestrator.service" ];
        requires = [ "river-orchestrator.service" ];

        serviceConfig = {
          Type = "simple";
          User = cfg.user;
          Group = cfg.group;
          WorkingDirectory = cfg.dataDir;
          ExecStart = lib.concatStringsSep " " [
            "${cfg.package}/bin/river-worker"
            "--orchestrator http://127.0.0.1:${toString cfg.orchestrator.port}"
            "--dyad ${workerCfg.dyad}"
            "--side ${workerCfg.side}"
            "--port ${toString workerCfg.port}"
          ];
          Restart = "on-failure";
          RestartSec = "5s";

          # Hardening
          NoNewPrivileges = true;
          ProtectSystem = "strict";
          ProtectHome = true;
          PrivateTmp = true;
          ReadWritePaths = [ cfg.dataDir ];
        } // lib.optionalAttrs (cfg.environmentFile != null) {
          EnvironmentFile = cfg.environmentFile;
        };
      })
    ) cfg.workers) // (lib.mapAttrs' (name: adapterCfg:
      # Adapter services
      lib.nameValuePair "river-adapter-${name}" (lib.mkIf adapterCfg.enable {
        description = "River Engine Adapter - ${name}";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" "river-orchestrator.service" ];
        requires = [ "river-orchestrator.service" ];

        serviceConfig = {
          Type = "simple";
          User = cfg.user;
          Group = cfg.group;
          WorkingDirectory = cfg.dataDir;
          ExecStart = lib.concatStringsSep " " [
            "${cfg.package}/bin/river-${adapterCfg.type}"
            "--orchestrator http://127.0.0.1:${toString cfg.orchestrator.port}"
            "--dyad ${adapterCfg.dyad}"
            "--type ${adapterCfg.type}"
            "--port ${toString adapterCfg.port}"
          ];
          Restart = "on-failure";
          RestartSec = "5s";

          # Hardening
          NoNewPrivileges = true;
          ProtectSystem = "strict";
          ProtectHome = true;
          PrivateTmp = true;
          ReadWritePaths = [ cfg.dataDir ];
        } // lib.optionalAttrs (cfg.environmentFile != null) {
          EnvironmentFile = cfg.environmentFile;
        };
      })
    ) cfg.adapters);

    # Firewall
    networking.firewall.allowedTCPPorts = lib.mkIf cfg.orchestrator.openFirewall [
      cfg.orchestrator.port
    ];

    # Create data directory and workspace directories with proper permissions
    systemd.tmpfiles.rules = [
      "d ${cfg.dataDir} 0750 ${cfg.user} ${cfg.group} -"
      "d ${cfg.dataDir}/workspaces 0750 ${cfg.user} ${cfg.group} -"
    ] ++ (lib.concatLists (lib.mapAttrsToList (dyadName: dyadCfg: [
      "d ${dyadCfg.workspace} 0750 ${cfg.user} ${cfg.group} -"
      "d ${dyadCfg.workspace}/left 0750 ${cfg.user} ${cfg.group} -"
      "d ${dyadCfg.workspace}/right 0750 ${cfg.user} ${cfg.group} -"
      "d ${dyadCfg.workspace}/roles 0750 ${cfg.user} ${cfg.group} -"
      "d ${dyadCfg.workspace}/conversations 0750 ${cfg.user} ${cfg.group} -"
    ]) (cfg.settings.dyads or {})));
  };
}

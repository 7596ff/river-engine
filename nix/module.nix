# The NixOS module (wall ch. 09): production runs without the CLI —
# one systemd service per agent, rendered from the same river.json
# the CLI accepts. Per-agent services mean per-agent restarts,
# journals, and resource limits, with systemd as the supervisor.
#
#   services.river = {
#     enable = true;
#     package = river-engine.packages.${system}.default;
#     configFile = ./river.json;        # the one config
#     environmentFile = "/run/secrets/river.env";  # the secrets
#     agents = [ "ada" ];               # which agents this host runs
#     user = "river";
#   };

{ config, lib, pkgs, ... }:

let
  cfg = config.services.river;
in
{
  options.services.river = {
    enable = lib.mkEnableOption "river gateways";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The river-engine package providing river-gateway.";
    };

    configFile = lib.mkOption {
      type = lib.types.path;
      description = "The river.json config file (one config, two runners).";
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "EnvironmentFile= with secrets (KEY=value lines), never in the store.";
    };

    agents = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      description = "Agent names from the config to run on this host.";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "river";
      description = "User the gateways run as.";
    };

    graceSeconds = lib.mkOption {
      type = lib.types.int;
      default = 30;
      description = "TimeoutStopSec — long enough to finish a turn (wall ch. 01).";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.user;
      createHome = true;
      home = "/var/lib/river";
    };
    users.groups.${cfg.user} = { };

    systemd.services = lib.listToAttrs (map (name: {
      name = "river-${name}";
      value = {
        description = "river gateway: ${name}";
        wantedBy = [ "multi-user.target" ];
        after = [ "network-online.target" ];
        wants = [ "network-online.target" ];

        serviceConfig = {
          ExecStart =
            "${cfg.package}/bin/river-gateway run --config ${cfg.configFile} --agent ${name}";
          User = cfg.user;
          Group = cfg.user;

          # Crash restart; backoff via systemd's own knobs.
          Restart = "on-failure";
          RestartSec = 1;
          RestartMaxDelaySec = 60;
          RestartSteps = 6;

          # SIGTERM finishes the turn (wall ch. 01); then SIGKILL.
          TimeoutStopSec = cfg.graceSeconds;
          KillSignal = "SIGTERM";
        } // lib.optionalAttrs (cfg.environmentFile != null) {
          EnvironmentFile = cfg.environmentFile;
        };
      };
    }) cfg.agents);
  };
}

# Example NixOS configuration for river-engine
#
# Add this to your NixOS configuration:
#   imports = [ ./path/to/river-engine/nix/module.nix ];
#
# Then configure as shown below.

{ config, pkgs, ... }:

{
  # Import the river-engine module
  imports = [
    ./module.nix
  ];

  # Override the package if building from local source
  nixpkgs.overlays = [
    (final: prev: {
      river-engine = final.callPackage ./package.nix { };
    })
  ];

  services.river-engine = {
    enable = true;

    # Environment file with secrets (recommended)
    # Create this file with:
    #   ANTHROPIC_API_KEY=sk-ant-...
    #   DISCORD_TOKEN=...
    #   DISCORD_DM_CHANNEL_ID=...
    environmentFile = "/run/secrets/river-engine";

    # Orchestrator settings
    orchestrator = {
      enable = true;
      port = 4337;
      openFirewall = false;  # Set true if workers run on different machines
    };

    # Main configuration (becomes river.json)
    settings = {
      port = 4337;

      # Model definitions
      models = {
        "claude-sonnet" = {
          endpoint = "https://api.anthropic.com/v1/messages";
          name = "claude-sonnet-4-20250514";
          api_key = "$ANTHROPIC_API_KEY";
          context_limit = 200000;
        };

        "claude-haiku" = {
          endpoint = "https://api.anthropic.com/v1/messages";
          name = "claude-haiku-4-20250514";
          api_key = "$ANTHROPIC_API_KEY";
          context_limit = 200000;
        };

        "nomic-embed" = {
          endpoint = "http://localhost:11434/v1/embeddings";
          name = "nomic-embed-text";
          api_key = "ollama";
          dimensions = 768;
        };
      };

      # Embedding model
      embed = {
        model = "nomic-embed";
      };

      # Dyad configurations
      dyads = {
        river = {
          workspace = "/var/lib/river/workspaces/river";

          uid = 1000;
          gid = 1000;

          left.name = "Iris";
          left.model = "claude-sonnet";

          right.name = "Viola";
          right.model = "claude-haiku";

          initialActor = "left";

          # Human operator (ground truth)
          ground = {
            name = "Cassie";  # optional, defaults to dyad name
            id = "123456789012345678";
            adapter = "discord";
            channel = "$DISCORD_DM_CHANNEL_ID";
          };

          # Platform adapters (binary resolved from PATH as river-{type})
          adapters = [
            {
              path = "${pkgs.river-engine}/bin/river-discord";
              side = "left";
              token = "$DISCORD_TOKEN";
              guild_id = "987654321098765432";
            }
          ];
        };
      };
    };

    # Worker instances
    # implied from config above

    # Adapter instances
    # this too
  };
}

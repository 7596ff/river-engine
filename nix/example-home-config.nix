# nix/example-home-config.nix
# Example home-manager configuration for River Engine
#
# This runs River as user services instead of system services.
# Useful for single-user setups or when you don't have root access.
#
# Usage in your home-manager configuration:
#   imports = [ /path/to/river-engine/nix/home-module.nix ];
#   # Then add the config below

{ config, pkgs, ... }:

{
  # Import the River Engine home-manager module
  # imports = [ ./home-module.nix ];

  services.river = {
    # Required: path to river-engine source
    package.src = /path/to/river-engine;

    # ─────────────────────────────────────────────────────────────
    # Orchestrator - manages local GGUF models
    # ─────────────────────────────────────────────────────────────
    orchestrator = {
      enable = true;
      port = 5000;
      modelDirs = [
        "${config.home.homeDirectory}/models"
      ];
      idleTimeout = 900;
      cudaSupport = false;
    };

    # ─────────────────────────────────────────────────────────────
    # Embedding server
    # ─────────────────────────────────────────────────────────────
    embedding = {
      enable = true;
      port = 8200;
      modelPath = "${config.home.homeDirectory}/models/nomic-embed-text-v1.5.Q8_0.gguf";
      gpuLayers = 99;
      cudaSupport = false;
    };

    # ─────────────────────────────────────────────────────────────
    # Redis - runs as user service with data in ~/.local/share/river/redis
    # ─────────────────────────────────────────────────────────────
    redis = {
      enable = true;
      port = 6379;
    };

    # ─────────────────────────────────────────────────────────────
    # Agent
    # ─────────────────────────────────────────────────────────────
    agents.river = {
      enable = true;

      # Use XDG directories for user setup
      workspace = "${config.home.homeDirectory}/.local/share/river/workspace";
      dataDir = "${config.home.homeDirectory}/.local/share/river/data";

      agentName = "river";
      port = 3000;
      contextLimit = 131072;

      # Local models via orchestrator
      orchestratorUrl = "http://localhost:5000";
      modelUrl = "http://localhost:5000";
      modelName = "llama-3.2-3b";
      spectatorModelUrl = "http://localhost:5000";
      spectatorModelName = "llama-3.2-1b";

      # Memory services
      embeddingUrl = "http://localhost:8200";
      redisUrl = "redis://localhost:6379";

      # Discord
      discord = {
        enable = true;
        tokenFile = "${config.home.homeDirectory}/.config/river/discord-token";
        guildId = 123456789012345678;
        port = 3002;
        channels = [ 123456789012345678 ];
        stateFile = "${config.home.homeDirectory}/.local/share/river/discord-state.json";
      };

      environment = {
        RUST_LOG = "info";
      };
    };
  };

  # Ensure workspace directories exist with proper structure
  # You'll need to copy the actor/spectator files manually:
  #   cp -r /path/to/river-engine/workspace/* ~/.local/share/river/workspace/
  home.activation.riverWorkspace = config.lib.dag.entryAfter [ "writeBoundary" ] ''
    mkdir -p ~/.local/share/river/{workspace,data}
    mkdir -p ~/.local/share/river/workspace/{actor,spectator,conversations,embeddings,thinking}
    mkdir -p ~/.config/river
  '';
}

# nix/example-config.nix
# Example NixOS configuration for River Engine
#
# This shows a complete setup with:
# - Orchestrator for local model management
# - Embedding server for semantic memory
# - Redis for ephemeral memory
# - An agent named "river" with Discord adapter
#
# Usage in your NixOS configuration:
#   imports = [ /path/to/river-engine/nix/nixos-module.nix ];
#   # Then add the config below

{ config, pkgs, ... }:

{
  # Import the River Engine module
  # imports = [ ./nixos-module.nix ];

  services.river = {
    # Required: path to river-engine source
    package.src = /path/to/river-engine;

    # ─────────────────────────────────────────────────────────────
    # Orchestrator - manages local GGUF models via llama.cpp
    # ─────────────────────────────────────────────────────────────
    orchestrator = {
      enable = true;
      port = 5000;

      # Directories containing .gguf model files
      modelDirs = [
        /home/user/models
        /var/lib/models
      ];

      # Resource management
      idleTimeout = 900;        # Unload idle models after 15 min
      reserveVramMb = 500;      # Keep 500MB VRAM free
      reserveRamMb = 2000;      # Keep 2GB RAM free
      portRange = "8080-8180";  # Ports for llama-server instances

      # Enable for NVIDIA GPUs
      cudaSupport = false;
    };

    # ─────────────────────────────────────────────────────────────
    # Embedding server - for semantic memory (memory_search tool)
    # ─────────────────────────────────────────────────────────────
    embedding = {
      enable = true;
      port = 8200;
      modelPath = /home/user/models/nomic-embed-text-v1.5.Q8_0.gguf;
      gpuLayers = 99;  # Offload all layers to GPU
      cudaSupport = false;
    };

    # ─────────────────────────────────────────────────────────────
    # Redis - for working memory and medium-term memory
    # ─────────────────────────────────────────────────────────────
    redis = {
      enable = true;
      port = 6379;
    };

    # ─────────────────────────────────────────────────────────────
    # LiteLLM (optional) - proxy for external API providers
    # Use this OR openrouter/anthropic options below, not both
    # ─────────────────────────────────────────────────────────────
    # litellm = {
    #   enable = true;
    #   port = 4000;
    #   apiKeyFile = /run/secrets/anthropic-api-key;
    #   models = [
    #     { name = "claude-sonnet"; litellmModel = "claude-sonnet-4-20250514"; }
    #     { name = "claude-haiku"; litellmModel = "claude-3-5-haiku-20241022"; }
    #   ];
    # };

    # ─────────────────────────────────────────────────────────────
    # OpenRouter (optional) - external API with prompt caching
    # ─────────────────────────────────────────────────────────────
    # openrouter = {
    #   enable = true;
    #   apiKeyFile = /run/secrets/openrouter-api-key;
    #   model = "anthropic/claude-sonnet-4";
    # };

    # ─────────────────────────────────────────────────────────────
    # Anthropic (optional) - direct Claude API access
    # ─────────────────────────────────────────────────────────────
    # anthropic = {
    #   enable = true;
    #   apiKeyFile = /run/secrets/anthropic-api-key;
    #   model = "claude-sonnet-4-20250514";
    # };

    # ─────────────────────────────────────────────────────────────
    # Agents - define one or more agents
    # ─────────────────────────────────────────────────────────────
    agents.river = {
      enable = true;

      # Workspace must contain actor/ and spectator/ subdirectories
      # with AGENTS.md, IDENTITY.md, RULES.md files.
      # Copy from river-engine/workspace/ as a starting point.
      workspace = /var/lib/river/workspace;

      # Database and logs directory
      dataDir = /var/lib/river/data;

      # Agent identity (used for Redis namespacing)
      agentName = "river";

      # Gateway HTTP API port
      port = 3000;

      # Context window size
      contextLimit = 131072;

      # Model configuration - use orchestrator for local models
      orchestratorUrl = "http://localhost:5000";
      modelUrl = "http://localhost:5000";
      modelName = "llama-3.2-3b";

      # Spectator can use a different (smaller/faster) model
      spectatorModelUrl = "http://localhost:5000";
      spectatorModelName = "llama-3.2-1b";

      # Memory services
      embeddingUrl = "http://localhost:8200";
      redisUrl = "redis://localhost:6379";

      # Discord adapter
      discord = {
        enable = true;
        tokenFile = /run/secrets/discord-token;
        guildId = 123456789012345678;  # Your Discord server ID
        port = 3002;
        channels = [
          123456789012345678  # Channel IDs to listen on
          234567890123456789
        ];
      };

      # Or define custom adapters
      # adapters = [
      #   {
      #     name = "slack";
      #     outboundUrl = "http://localhost:3003/send";
      #     readUrl = "http://localhost:3003/read";
      #   }
      # ];

      # Extra environment variables
      environment = {
        RUST_LOG = "info,river_gateway=debug";
      };
    };

    # You can define multiple agents
    # agents.assistant = {
    #   enable = true;
    #   workspace = /var/lib/river-assistant/workspace;
    #   dataDir = /var/lib/river-assistant/data;
    #   port = 3010;
    #   # ... other options
    # };
  };
}

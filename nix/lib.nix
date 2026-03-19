# nix/lib.nix
# Shared option types and service builders for River Engine modules
{ lib }:

let
  inherit (lib) mkOption mkEnableOption types;

in {
  # Package source options
  packageOptions = {
    src = mkOption {
      type = types.path;
      description = "Path to river-engine source directory.";
    };
  };

  # Orchestrator options
  orchestratorOptions = {
    enable = mkEnableOption "River orchestrator";
    port = mkOption { type = types.port; default = 5000; description = "Port for the orchestrator API."; };
    healthThreshold = mkOption { type = types.int; default = 120; description = "Health threshold in seconds."; };
    modelDirs = mkOption { type = types.listOf types.path; default = []; description = "Directories to scan for GGUF models."; };
    externalModelsFile = mkOption { type = types.nullOr types.path; default = null; description = "Path to external models config JSON."; };
    modelsConfigFile = mkOption { type = types.nullOr types.path; default = null; description = "Path to legacy models config JSON."; };
    llamaServerPath = mkOption { type = types.nullOr types.path; default = null; description = "Path to llama-server binary."; };
    idleTimeout = mkOption { type = types.int; default = 900; description = "Idle timeout in seconds."; };
    portRange = mkOption { type = types.str; default = "8080-8180"; description = "Port range for llama-server instances."; };
    reserveVramMb = mkOption { type = types.int; default = 500; description = "Reserved VRAM in MB."; };
    reserveRamMb = mkOption { type = types.int; default = 2000; description = "Reserved RAM in MB."; };
    cudaSupport = mkOption { type = types.bool; default = false; description = "Enable CUDA support."; };
    environment = mkOption { type = types.attrsOf types.str; default = {}; description = "Extra environment variables."; };
  };

  # Embedding server options
  embeddingOptions = {
    enable = mkEnableOption "River embedding server";
    port = mkOption { type = types.port; default = 8200; description = "Port for the embedding server."; };
    modelPath = mkOption { type = types.path; description = "Path to embedding model GGUF file."; };
    gpuLayers = mkOption { type = types.int; default = 99; description = "Number of GPU layers."; };
    cudaSupport = mkOption { type = types.bool; default = false; description = "Enable CUDA support."; };
  };

  # Redis options
  redisOptions = {
    enable = mkEnableOption "Redis for River";
    port = mkOption { type = types.port; default = 6379; description = "Port for Redis server."; };
  };

  # LiteLLM proxy options
  litellmOptions = {
    enable = mkEnableOption "LiteLLM proxy for external models";
    port = mkOption { type = types.port; default = 4000; description = "Port for LiteLLM API."; };
    apiKeyFile = mkOption { type = types.path; description = "Path to file containing ANTHROPIC_API_KEY."; };
    models = mkOption {
      type = types.listOf (types.submodule {
        options = {
          name = mkOption { type = types.str; description = "Model name alias."; };
          litellmModel = mkOption { type = types.str; description = "LiteLLM model identifier (e.g., claude-sonnet-4-20250514)."; };
        };
      });
      default = [{ name = "claude-sonnet"; litellmModel = "claude-sonnet-4-20250514"; }];
      description = "Models to expose via LiteLLM.";
    };
    extraArgs = mkOption { type = types.listOf types.str; default = []; description = "Extra arguments for litellm."; };
  };

  # OpenRouter options (alternative to LiteLLM with better prompt caching support)
  openrouterOptions = {
    enable = mkEnableOption "OpenRouter API (supports automatic prompt caching)";
    apiKeyFile = mkOption { type = types.path; description = "Path to file containing OPENROUTER_API_KEY."; };
    baseUrl = mkOption { type = types.str; default = "https://openrouter.ai/api"; description = "OpenRouter API base URL."; };
    model = mkOption { type = types.str; default = "anthropic/claude-sonnet-4"; description = "Model to use (e.g., anthropic/claude-sonnet-4)."; };
  };

  # Anthropic API options (direct API access with ephemeral caching)
  anthropicOptions = {
    enable = mkEnableOption "Anthropic API (direct Claude access with ephemeral caching)";
    apiKeyFile = mkOption { type = types.path; description = "Path to file containing ANTHROPIC_API_KEY."; };
    baseUrl = mkOption { type = types.str; default = "https://api.anthropic.com"; description = "Anthropic API base URL."; };
    model = mkOption { type = types.str; default = "claude-sonnet-4-20250514"; description = "Model to use (e.g., claude-sonnet-4-20250514)."; };
  };

  # Discord adapter options
  discordOptions = {
    enable = mkEnableOption "Discord adapter";
    tokenFile = mkOption { type = types.path; description = "Path to Discord bot token file."; };
    guildId = mkOption { type = types.int; description = "Discord guild ID."; };
    gatewayUrl = mkOption { type = types.nullOr types.str; default = null; description = "Gateway URL (auto-derived if null)."; };
    port = mkOption { type = types.port; default = 3002; description = "Adapter HTTP server port."; };
    channels = mkOption { type = types.listOf types.int; default = []; description = "Initial Discord channel IDs."; };
    stateFile = mkOption { type = types.nullOr types.path; default = null; description = "State file for persistence."; };
  };

  # Agent submodule options (function that takes name)
  mkAgentOptions = { name, ... }: {
    enable = mkEnableOption "this River agent";
    workspace = mkOption { type = types.path; description = "Workspace directory."; };
    dataDir = mkOption { type = types.path; description = "Data directory for database."; };
    agentName = mkOption { type = types.str; default = name; description = "Agent name for Redis namespacing."; };
    port = mkOption { type = types.port; default = 3000; description = "Gateway port."; };
    contextLimit = mkOption { type = types.int; default = 131072; description = "Context window size in tokens."; };
    modelUrl = mkOption { type = types.nullOr types.str; default = null; description = "Model server URL."; };
    modelName = mkOption { type = types.nullOr types.str; default = null; description = "Model name."; };
    orchestratorUrl = mkOption { type = types.nullOr types.str; default = null; description = "Orchestrator URL."; };
    embeddingUrl = mkOption { type = types.nullOr types.str; default = null; description = "Embedding server URL."; };
    redisUrl = mkOption { type = types.nullOr types.str; default = null; description = "Redis URL."; };
    authTokenFile = mkOption { type = types.nullOr types.path; default = null; description = "Path to file containing bearer token for authentication."; };
    environmentFile = mkOption { type = types.nullOr types.path; default = null; description = "Path to environment file (KEY=value format) for API keys etc."; };
    adapters = mkOption {
      type = types.listOf (types.submodule {
        options = {
          name = mkOption { type = types.str; description = "Adapter name (e.g., 'discord', 'slack')."; };
          outboundUrl = mkOption { type = types.str; description = "URL for sending messages."; };
          readUrl = mkOption { type = types.nullOr types.str; default = null; description = "URL for reading channel history (optional)."; };
        };
      });
      default = [];
      description = "Communication adapters for send_message tool.";
    };
    environment = mkOption { type = types.attrsOf types.str; default = {}; description = "Extra environment variables."; };
  };

  # Command builder: orchestrator
  mkOrchestratorCommand = { cfg, packages }: let
    llamaServer = if cfg.llamaServerPath != null
      then cfg.llamaServerPath
      else "${packages.llama-cpp}/bin/llama-server";
  in lib.concatStringsSep " " ([
    "${packages.river-orchestrator}/bin/river-orchestrator"
    "--port" (toString cfg.port)
    "--health-threshold" (toString cfg.healthThreshold)
    "--idle-timeout" (toString cfg.idleTimeout)
    "--llama-server-path" llamaServer
    "--port-range" cfg.portRange
    "--reserve-vram-mb" (toString cfg.reserveVramMb)
    "--reserve-ram-mb" (toString cfg.reserveRamMb)
  ] ++ lib.optionals (cfg.modelDirs != []) [
    "--model-dirs" (lib.concatStringsSep "," (map toString cfg.modelDirs))
  ] ++ lib.optionals (cfg.externalModelsFile != null) [
    "--external-models" (toString cfg.externalModelsFile)
  ] ++ lib.optionals (cfg.modelsConfigFile != null) [
    "--models-config" (toString cfg.modelsConfigFile)
  ]);

  # Command builder: embedding
  mkEmbeddingCommand = { cfg, packages }: lib.concatStringsSep " " [
    "${packages.llama-cpp}/bin/llama-server"
    "--embedding"
    "--model" (toString cfg.modelPath)
    "--port" (toString cfg.port)
    "--n-gpu-layers" (toString cfg.gpuLayers)
  ];

  # Command builder: gateway
  # discordCfg is optional - if provided and enabled, auto-adds discord adapter
  # openrouterCfg is optional - if provided and enabled, auto-derives model URL
  # anthropicCfg is optional - if provided and enabled, auto-derives model URL (takes precedence)
  mkGatewayCommand = { cfg, packages, discordCfg ? null, openrouterCfg ? null, anthropicCfg ? null }: let
    # Build adapter flags from explicit config
    explicitAdapters = map (a:
      "--adapter" + " " + a.name + ":" + a.outboundUrl +
        (if a.readUrl != null then ":" + a.readUrl else "")
    ) cfg.adapters;

    # Auto-add discord adapter if discord is enabled
    discordAdapter = lib.optionals (discordCfg != null && discordCfg.enable) [
      "--adapter" "discord:http://localhost:${toString discordCfg.port}/send"
    ];

    adapterFlags = explicitAdapters ++ discordAdapter;

    # Derive model URL: explicit > anthropic > openrouter > null
    # Note: model.rs auto-detects provider based on URL (api.anthropic.com = Anthropic)
    effectiveModelUrl =
      if cfg.modelUrl != null then cfg.modelUrl
      else if anthropicCfg != null && anthropicCfg.enable then anthropicCfg.baseUrl
      else if openrouterCfg != null && openrouterCfg.enable then openrouterCfg.baseUrl
      else null;

    # Derive model name: explicit > anthropic > openrouter > null
    effectiveModelName =
      if cfg.modelName != null then cfg.modelName
      else if anthropicCfg != null && anthropicCfg.enable then anthropicCfg.model
      else if openrouterCfg != null && openrouterCfg.enable then openrouterCfg.model
      else null;
  in lib.concatStringsSep " " ([
    "${packages.river-gateway}/bin/river-gateway"
    "--workspace" (toString cfg.workspace)
    "--data-dir" (toString cfg.dataDir)
    "--agent-name" cfg.agentName
    "--port" (toString cfg.port)
    "--context-limit" (toString cfg.contextLimit)
  ] ++ lib.optionals (effectiveModelUrl != null) [
    "--model-url" effectiveModelUrl
  ] ++ lib.optionals (effectiveModelName != null) [
    "--model-name" effectiveModelName
  ] ++ lib.optionals (cfg.orchestratorUrl != null) [
    "--orchestrator-url" cfg.orchestratorUrl
  ] ++ lib.optionals (cfg.embeddingUrl != null) [
    "--embedding-url" cfg.embeddingUrl
  ] ++ lib.optionals (cfg.redisUrl != null) [
    "--redis-url" cfg.redisUrl
  ] ++ lib.optionals (cfg.authTokenFile != null) [
    "--auth-token-file" (toString cfg.authTokenFile)
  ] ++ adapterFlags);

  # Command builder: discord
  mkDiscordCommand = { cfg, agentPort, packages }: let
    gatewayUrl = if cfg.gatewayUrl != null
      then cfg.gatewayUrl
      else "http://localhost:${toString agentPort}";
  in lib.concatStringsSep " " ([
    "${packages.river-discord}/bin/river-discord"
    "--token-file" (toString cfg.tokenFile)
    "--gateway-url" gatewayUrl
    "--listen-port" (toString cfg.port)
    "--guild-id" (toString cfg.guildId)
  ] ++ lib.optionals (cfg.channels != []) [
    "--channels" (lib.concatMapStringsSep "," toString cfg.channels)
  ] ++ lib.optionals (cfg.stateFile != null) [
    "--state-file" (toString cfg.stateFile)
  ]);

  # Generate LiteLLM config YAML
  mkLitellmConfig = { cfg }: let
    modelList = map (m: {
      model_name = m.name;
      litellm_params = {
        model = m.litellmModel;
        # Anthropic prompt caching: 4 breakpoint max, 20-block lookback from each
        # System prompt + last message covers most conversations efficiently
        cache_control_injection_points = [
          { location = "message"; role = "system"; }  # Cache stable system prompt
          { location = "message"; index = -1; }       # Last msg - 20-block lookback covers recent context
        ];
      };
    }) cfg.models;
  in builtins.toJSON {
    model_list = modelList;
  };
}

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
    modelUrl = mkOption { type = types.nullOr types.str; default = null; description = "Model server URL."; };
    modelName = mkOption { type = types.nullOr types.str; default = null; description = "Model name."; };
    orchestratorUrl = mkOption { type = types.nullOr types.str; default = null; description = "Orchestrator URL."; };
    embeddingUrl = mkOption { type = types.nullOr types.str; default = null; description = "Embedding server URL."; };
    redisUrl = mkOption { type = types.nullOr types.str; default = null; description = "Redis URL."; };
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
  mkGatewayCommand = { cfg, packages }: lib.concatStringsSep " " ([
    "${packages.river-gateway}/bin/river-gateway"
    "--workspace" (toString cfg.workspace)
    "--data-dir" (toString cfg.dataDir)
    "--agent-name" cfg.agentName
    "--port" (toString cfg.port)
  ] ++ lib.optionals (cfg.modelUrl != null) [
    "--model-url" cfg.modelUrl
  ] ++ lib.optionals (cfg.modelName != null) [
    "--model-name" cfg.modelName
  ] ++ lib.optionals (cfg.orchestratorUrl != null) [
    "--orchestrator-url" cfg.orchestratorUrl
  ] ++ lib.optionals (cfg.embeddingUrl != null) [
    "--embedding-url" cfg.embeddingUrl
  ] ++ lib.optionals (cfg.redisUrl != null) [
    "--redis-url" cfg.redisUrl
  ]);

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
}

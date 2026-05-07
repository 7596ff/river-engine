{ lib, pkgs, ... }:
{
  imports = [
    ./desktop.nix
    ./filesystems.nix
    ./network.nix
    ./system.nix
    ./user.nix

    ../../modules/river-engine.nix
  ];

  time.timeZone = "America/New_York";
  i18n.defaultLocale = "en_US.UTF-8";

  hardware.nvidia.open = true;

  networking.hostName = "athena";
  networking.hostId = "09a1d49f";

  # age.secrets.river-env = {
  #   file = ../../secrets/river-env.age;
  #   mode = "700";
  #   owner = "river";
  # };

  age.secrets.http-basic-auth = {
    file = ../../secrets/http-basic-auth.age;
    mode = "700";
    owner = "nginx";
  };

  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];

  nix.settings.extra-sandbox-paths = [ "${pkgs.git}" "${pkgs.openssh}" ];

  nixpkgs.config.allowUnfreePredicate =
    pkg:
    builtins.elem (lib.getName pkg) [
      "nvidia-x11"
      "rarbg-selfhosted"
      "steam"
      "steam-unwrapped"
      "nvidia-settings"
      "gazelle-origin"
      "discord"
      "cuda12.8-cuda_cccl-12.8.90"
      "cuda_cccl"
      "cuda_cudart"
      "cuda_nvcc"
      "libcublas"
      "claude-code"
      "obsidian"
    ];

  services.river-engine = {
    enable = true;
    user = "cassie";
    group = "cassie";

    srcPath = /home/cassie/river-engine;

    models = {
      gemma4 = {
        provider = "ollama";
        endpoint = "http://localhost:11434/v1";
        name = "gemma4:e2b";
        context_limit = 8192;
      };
    };

    agents = {
      iris = {
        workspace = /home/cassie/stream;
        data_dir = /var/lib/river/iris;
        port = 3000;
        model = "gemma4";
        spectator_model = "gemma4";
        context = {
          limit = 8192;
          compaction_threshold = 0.80;
          fill_target = 0.40;
          min_messages = 20;
        };
        log.level = "debug";
      };
    };
  };

  security.sudo.wheelNeedsPassword = true;

  system.stateVersion = "25.11";
}

# nix/packages.nix
# Package definitions for River Engine binaries
{ pkgs, cudaSupport ? false }:

let
  llama-cpp = pkgs.llama-cpp.override {
    inherit cudaSupport;
  };

  src = ./..;

  commonBuildInputs = with pkgs; [ openssl ];
  commonNativeBuildInputs = with pkgs; [ pkg-config ];

in {
  inherit llama-cpp;

  river-gateway = pkgs.rustPlatform.buildRustPackage {
    pname = "river-gateway";
    version = "0.1.0";
    inherit src;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-gateway" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };

  river-orchestrator = pkgs.rustPlatform.buildRustPackage {
    pname = "river-orchestrator";
    version = "0.1.0";
    inherit src;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-orchestrator" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };

  river-discord = pkgs.rustPlatform.buildRustPackage {
    pname = "river-discord";
    version = "0.1.0";
    inherit src;
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [ "-p" "river-discord" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };
}

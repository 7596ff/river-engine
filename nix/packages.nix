# nix/packages.nix
# Package definitions for River Engine binaries
{ pkgs, cudaSupport ? false, src }:

let
  llama-cpp = pkgs.llama-cpp.override {
    inherit cudaSupport;
  };

  # Filter source to only include Rust build files
  filteredSrc = pkgs.lib.sourceByRegex src [
    "Cargo\.toml"
    "Cargo\.lock"
    "crates"
    "crates/.*"
  ];

  commonBuildInputs = with pkgs; [ openssl ];
  commonNativeBuildInputs = with pkgs; [ pkg-config git ];

in {
  inherit llama-cpp;

  river-gateway = pkgs.rustPlatform.buildRustPackage {
    pname = "river-gateway";
    version = "0.1.3";
    src = filteredSrc;
    cargoLock.lockFile = "${filteredSrc}/Cargo.lock";
    cargoBuildFlags = [ "-p" "river-gateway" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };

  river-orchestrator = pkgs.rustPlatform.buildRustPackage {
    pname = "river-orchestrator";
    version = "0.1.3";
    src = filteredSrc;
    cargoLock.lockFile = "${filteredSrc}/Cargo.lock";
    cargoBuildFlags = [ "-p" "river-orchestrator" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };

  river-discord = pkgs.rustPlatform.buildRustPackage {
    pname = "river-discord";
    version = "0.1.3";
    src = filteredSrc;
    cargoLock.lockFile = "${filteredSrc}/Cargo.lock";
    cargoBuildFlags = [ "-p" "river-discord" ];
    nativeBuildInputs = commonNativeBuildInputs;
    buildInputs = commonBuildInputs;
  };
}

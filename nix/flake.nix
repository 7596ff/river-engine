{
  description = "River Engine - Multi-agent orchestration system";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages = {
          river-engine = pkgs.callPackage ./package.nix { };
          default = self.packages.${system}.river-engine;
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            cargo
            rustc
            rust-analyzer
            pkg-config
            openssl
          ];
        };
      }
    ) // {
      nixosModules = {
        river-engine = import ./module.nix;
        default = self.nixosModules.river-engine;
      };

      # Overlay for adding river-engine to pkgs
      overlays.default = final: prev: {
        river-engine = final.callPackage ./package.nix { };
      };
    };
}

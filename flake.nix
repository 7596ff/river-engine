{
  description = "river-engine — multi-agent orchestration system";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        packages = {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "river-engine";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.git ];
            buildInputs = [ pkgs.sqlite ];

            meta = with pkgs.lib; {
              description = "Multi-agent orchestration system";
              license = licenses.mit;
            };
          };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            cargo
            rustc
            rust-analyzer
            clippy
            rustfmt
            sqlite
            git
            redis
          ];
        };
      }) // {
        nixosModules.default = import ./nix/modules/river-engine.nix;

        overlays.default = final: prev: {
          river-engine = self.packages.${final.system}.default;
        };
      };
}

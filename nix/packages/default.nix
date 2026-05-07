{ pkgs }:
{
  letta-code = pkgs.callPackage ./letta-code/package.nix { };
  rarbg-selfhosted = pkgs.callPackage ./rarbg-selfhosted/package.nix { };
  river-engine = pkgs.callPackage ./river-engine/package.nix { };
  openclaw = pkgs.callPackage ./openclaw/package.nix { };
  whisper-timestamped = pkgs.callPackage ./whisper-timestamped/package.nix { };
}

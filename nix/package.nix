{
  lib,
  rustPlatform,
  fetchgit,
  pkg-config,
  openssl,
}:

rustPlatform.buildRustPackage rec {
  pname = "river-engine";
  version = "0.1.0";

  src = fetchgit {
    url = "git@athena.7596ff.com:river-engine.git";
    rev = "main";
    # Replace with actual hash after first build attempt
    hash = lib.fakeHash;
  };

  cargoLock = {
    lockFile = "${src}/Cargo.lock";
  };

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    openssl
  ];

  # Build all workspace binaries
  cargoBuildFlags = [
    "--workspace"
  ];

  # Install all binaries
  postInstall = ''
    # Binaries are automatically installed from workspace
    # river-orchestrator, river-worker, river-discord, river-tui, river-embed
  '';

  meta = with lib; {
    description = "Multi-agent orchestration system with platform adapters";
    homepage = "https://athena.7596ff.com/river-engine";
    license = licenses.mit;
    maintainers = [ ];
    platforms = platforms.linux;
  };
}

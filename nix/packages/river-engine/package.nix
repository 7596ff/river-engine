{ lib
, rustPlatform
, sqlite
, git
, src ? /home/cassie/river-engine
}:

rustPlatform.buildRustPackage {
  pname = "river-engine";
  version = "0.1.0";
  inherit src;
  cargoLock.lockFile = "${src}/Cargo.lock";

  nativeBuildInputs = [ git ];
  buildInputs = [ sqlite ];

  meta = {
    description = "Multi-agent orchestration system";
    license = lib.licenses.mit;
  };
}

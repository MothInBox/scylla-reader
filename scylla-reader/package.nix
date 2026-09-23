{ rustPlatform, pkg-config, openssl, curl, lib
, settings ? {
    rate_limit_secs = 2;
    debug_log = false;
    reader_mode = "Paged";
  }
}:
let
  # Single source of truth: version comes from [workspace.package] in the root Cargo.toml
  workspace = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "scylla-reader";
  version = workspace.workspace.package.version;
  src = ../.;
  cargoRoot = "scylla-reader";
  buildAndTestSubdir = "scylla-reader";
  cargoBuildFlags = ["-p" "scylla-reader" "-p" "scylla-server"];
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ openssl curl ];

  SCYLLA_RATE_LIMIT = toString settings.rate_limit_secs;
  SCYLLA_DEBUG_LOG = if settings.debug_log then "true" else "false";
  SCYLLA_READER_MODE = settings.reader_mode;
}

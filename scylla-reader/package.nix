{ rustPlatform, pkg-config, openssl, curl, lib
, settings ? {
    rate_limit_secs = 2;
    debug_log = false;
    reader_mode = "Paged";
  }
}:
rustPlatform.buildRustPackage {
  pname = "scylla-reader";
  version = "0.2.0";
  src = ../.;
  cargoRoot = "scylla-reader";
  buildAndTestSubdir = "scylla-reader";
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ openssl curl ];

  SCYLLA_RATE_LIMIT = toString settings.rate_limit_secs;
  SCYLLA_DEBUG_LOG = if settings.debug_log then "true" else "false";
  SCYLLA_READER_MODE = settings.reader_mode;
}

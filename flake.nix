{
  description = "Scylla-Reader Development Environment and Package";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
  }: let
    supportedSystems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];

    forEachSystem = f:
      nixpkgs.lib.genAttrs supportedSystems (system:
        f {
          pkgs = import nixpkgs {
            inherit system;
            overlays = [rust-overlay.overlays.default];
          };
        });
  in {
    packages = forEachSystem ({pkgs}: let
      rustToolchain = pkgs.rust-bin.stable.latest.default.override {
        targets = ["wasm32-unknown-unknown"];
      };
      customRustPlatform = pkgs.makeRustPlatform {
        cargo = rustToolchain;
        rustc = rustToolchain;
      };
    in {
      default = customRustPlatform.buildRustPackage {
        pname = "scylla-reader";
        version = "0.1.0";

        src = ./.;

        cargoRoot = "scylla-reader";

        buildAndTestSubdir = "scylla-reader";

        cargoLock = {
          lockFile = ./scylla-reader/Cargo.lock;
        };

        nativeBuildInputs = [pkgs.pkg-config];
        buildInputs = [pkgs.openssl pkgs.curl];
      };
    });

    devShells = forEachSystem ({pkgs}: let
      rust = pkgs.rust-bin.stable.latest.default.override {
        targets = ["wasm32-unknown-unknown"];
      };
    in {
      default = pkgs.mkShell {
        buildInputs = with pkgs; [
          rust
          pkg-config
          openssl
          curl
          extism-cli
        ];
      };
    });
  };
}

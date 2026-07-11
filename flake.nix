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
      default = pkgs.callPackage ./scylla-reader/package.nix {
        rustPlatform = customRustPlatform;
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
          cargo-watch
          just
          pre-commit
        ];
      };
    });
  };
}

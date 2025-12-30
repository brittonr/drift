{
  description = "Drift - A terminal music player for streaming services";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        nativeBuildInputs = with pkgs; [
          rustToolchain
          pkg-config
          cargo-watch
        ];

        buildInputs = with pkgs; [
          openssl
        ];

        runtimeDependencies = with pkgs; [
          cava  # Console audio visualizer
          mpc  # For MPD control
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs;
          buildInputs = buildInputs ++ runtimeDependencies;

          shellHook = ''
            echo "Drift Development Environment"
            echo "============================="
            echo ""
            echo "Commands:"
            echo "  cargo build    - Build the project"
            echo "  cargo run      - Run the TUI"
            echo "  cargo watch    - Auto-rebuild on changes"
            echo ""
          '';

          RUST_BACKTRACE = 1;
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "drift";
          version = "0.1.0";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          inherit nativeBuildInputs buildInputs;
        };
      });
}
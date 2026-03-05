{
  description = "Drift - A terminal music player for streaming services";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    unit2nix = {
      url = "github:brittonr/unit2nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      unit2nix,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };

        updatePlan = pkgs.writeShellScriptBin "update-plan" ''
          exec ${unit2nix.packages.${system}.unit2nix}/bin/unit2nix \
            --manifest-path ./Cargo.toml \
            -o build-plan.json
        '';

        nativeBuildInputs = with pkgs; [
          rustToolchain
          pkg-config
          cargo-watch
          updatePlan
        ];

        buildInputs = with pkgs; [
          openssl
        ];

        runtimeDependencies = with pkgs; [
          cava # Console audio visualizer
          mpc # For MPD control
        ];

        # unit2nix per-crate builds (manual mode — no IFD)
        # Regenerate build-plan.json: run `update-plan` in devshell
        ws = import "${unit2nix}/lib/build-from-unit-graph.nix" {
          inherit pkgs;
          src = ./.;
          resolvedJson = ./build-plan.json;
        };
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
            echo "  update-plan    - Regenerate build-plan.json"
            echo ""
          '';

          RUST_BACKTRACE = 1;
        };

        packages.default = ws.workspaceMembers."drift".build;

        # Regenerate build plan (requires nightly cargo on PATH)
        apps.update-plan = {
          type = "app";
          program = toString (
            pkgs.writeShellScript "update-plan" ''
              exec ${unit2nix.packages.${system}.unit2nix}/bin/unit2nix \
                --manifest-path ./Cargo.toml \
                -o build-plan.json
            ''
          );
        };
      }
      // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
        # NixOS VM integration tests (Linux only, require KVM)
        checks = {
          mpd-integration = import ./tests/nixos/mpd-integration.nix {
            inherit pkgs;
            drift = self.packages.${system}.default;
          };
          remote-mpd = import ./tests/nixos/remote-mpd.nix {
            inherit pkgs;
            drift = self.packages.${system}.default;
          };
        };
      }
    );
}

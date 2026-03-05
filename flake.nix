{
  description = "Drift - A terminal music player for streaming services";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    unit2nix = {
      url = "github:brittonr/unit2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      unit2nix,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      mkPkgs = system:
        import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = mkPkgs system;
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
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              rustToolchain
              pkg-config
              cargo-watch
              updatePlan
            ];
            buildInputs = with pkgs; [
              openssl
              cava
              mpc
            ];

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
        }
      );

      packages = forAllSystems (system:
        let
          pkgs = mkPkgs system;
          # unit2nix per-crate builds (manual mode — no IFD)
          # Regenerate build-plan.json: run `update-plan` in devshell
          ws = import "${unit2nix}/lib/build-from-unit-graph.nix" {
            inherit pkgs;
            src = ./.;
            resolvedJson = ./build-plan.json;
          };
        in
        {
          default = ws.workspaceMembers."drift".build;
        }
      );

      apps = forAllSystems (system:
        let
          pkgs = mkPkgs system;
        in
        {
          # Regenerate build plan (requires nightly cargo on PATH)
          update-plan = {
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
      );

      # NixOS VM integration tests (Linux only, require KVM)
      checks = forAllSystems (system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          mpd-integration = import ./tests/nixos/mpd-integration.nix {
            inherit pkgs;
            drift = self.packages.${system}.default;
          };
          remote-mpd = import ./tests/nixos/remote-mpd.nix {
            inherit pkgs;
            drift = self.packages.${system}.default;
          };
        }
      );
    };
}

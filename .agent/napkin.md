# Napkin

## Corrections
| Date | Source | What Went Wrong | What To Do Instead |
|------|--------|----------------|-------------------|

## User Preferences
- Uses unit2nix for per-crate incremental Nix builds (no crane/IFD)
- `update-plan` regenerates build-plan.json when Cargo.toml/deps change
- Nightly Rust toolchain

## Patterns That Work
- unit2nix incremental builds work: changing src/main.rs only rebuilds `rust_drift` derivation (~81s), all dependency crates stay cached in nix store
- `nix build .#default -L` shows exactly which derivations rebuild

## Patterns That Don't Work
- (none yet)

## Domain Notes
- Drift is a terminal music player (ratatui TUI) for Tidal streaming
- Workspace has one path dep: `drift-plugin` in `crates/drift-plugin`
- Optional `aspen` feature deps point to `../aspen/` (sibling repo, not in workspace)
- 3 binaries: drift (main TUI), drift-sync, tidal-db
- NixOS VM integration tests in `tests/nixos/`

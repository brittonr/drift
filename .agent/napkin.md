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
- Replication task design: Arc-wrap LocalStorage + WalManager, pass to tokio::spawn. mpsc channel for SyncEvents. poll_changes() drains channel via try_recv(). Merge functions are pure (merge.rs), task applies results to local storage.

## Patterns That Don't Work
- redb AccessGuard borrows: must read value into owned type, drop guard, THEN mutate the table (can't hold immutable borrow while inserting)

## Domain Notes
- Drift is a terminal music player (ratatui TUI) for Tidal streaming
- Workspace has one path dep: `drift-plugin` in `crates/drift-plugin`
- Optional `aspen` feature deps point to `../aspen/` (sibling repo, not in workspace)
- 3 binaries: drift (main TUI), drift-sync, tidal-db
- NixOS VM integration tests in `tests/nixos/`
- Playlist sync uses drift-plugin types: SyncedPlaylist (LWW metadata + OR-set tracks), PlaylistIndex
- Peer cluster support: PeerConfig in StorageConfig, Aspen peer cluster API (AddPeerCluster, UpdatePeerClusterFilter with include prefix)
- Key schema: drift:{user}:playlist:{id}, drift:{user}:playlist_index
- TUI peer browse: tab 2 in browse mode, three-panel (peers/playlists/tracks), h/l between panels
- AspenStorage peer playlist reads: scan for drift:*:playlist_index from non-self users, match peer name to user ID
- PeerClusterManager: init on startup, AddPeerCluster + include filter ["drift:"], idempotent (checks existing before adding)
- Worker delegated tasks that report "done" may not actually edit files — verify with git diff

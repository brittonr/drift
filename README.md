# Drift

A terminal music player for **Tidal**, **YouTube**, and **Bandcamp** with an MPD backend.

Drift keeps playback state locally. Optional cross-device sync uses S3-compatible storage, including RustFS. Celld can coordinate metadata writes.

## Features

- **Local-first architecture** — all data in local redb/TOML/JSON, works fully offline, multiplayer sync optional
- **Multi-service search** — unified search across Tidal, YouTube, and Bandcamp with fuzzy filtering (nucleo)
- **Album radio** — auto-generate queues from similar tracks
- **Playlist management** — create, rename, sync, and manage playlists across services
- **Download management** — queue downloads, track progress, content-dedup via BLAKE3
- **Offline playback** — downloaded tracks are preferred automatically, queue restore works without network
- **Cross-device sync** — durable background replication to S3, with optional Celld metadata coordination
- **Metadata cache** — playlists, favorites, albums, artists cached locally for instant offline access
- **Album art** — sixel/kitty protocol image rendering in-terminal
- **CAVA visualizer** — live audio visualizer integration
- **Video mode** — YouTube video playback via mpv
- **Theming** — built-in presets (Catppuccin Mocha, Nord, Dracula, Gruvbox, Solarized, Tokyo Night) plus custom themes
- **drift-sync** — bulk library downloader at MAX (HI_RES_LOSSLESS) quality

## Requirements

| Dependency | Required | Purpose |
|-----------|----------|---------|
| **MPD** | Yes | Audio playback backend |
| **yt-dlp** | No | YouTube and Bandcamp streaming/downloads |
| **CAVA** | No | Audio visualizer |
| **mpv** | No | YouTube video playback |

## Installation

```sh
# Nix (recommended)
nix run .#drift

# Cargo
cargo build --release

# S3 support is enabled by default
cargo build --release --features s3

# Local-only build
cargo build --release --no-default-features
```

## Configuration

Configuration lives at `~/.config/drift/config.toml`. A default config is created on first run.

```toml
[mpd]
host = "localhost"
port = 6600

[playback]
default_volume = 80
audio_quality = "high"       # "low", "high", "lossless", "master"
resume_on_startup = true

[ui]
show_visualizer = true
show_album_art = true
visualizer_bars = 20
status_interval_ms = 200
album_art_cache_size = 50    # LRU eviction for album art images

[downloads]
max_concurrent = 2
# download_dir = "/custom/path"  # default: XDG cache dir
auto_tag = true
sync_interval_minutes = 30   # auto-sync interval for playlists (0 = disabled)

[service]
primary = "tidal"
auto_detect = true           # enable YouTube/Bandcamp if yt-dlp found
# enabled = ["tidal", "youtube"]  # explicit service list

[bandcamp]
# cookie_file = "/path/to/cookies.txt"  # Netscape format
# cookies_from_browser = "firefox"       # or chrome, brave, edge
# username = "myusername"                # for collection URL
cache_duration_hours = 24

[search]
max_results = 30
debounce_ms = 300
fuzzy_filter = true
timeout_seconds = 10
history_size = 50
live_preview = true
min_chars = 2
cache_enabled = true
cache_ttl_seconds = 3600

[video]
mpv_path = "mpv"
socket_path = "/tmp/mpv-drift.sock"
# window_geometry = "1280x720"
fullscreen = false
hwdec = "auto"

[storage]
backend = "local"                # always local-first
sync_enabled = false             # enable S3 replication after configuration
# user_id = "alice"             # same account name on each device
# device_id = "laptop"          # unique device name; defaults to hostname
prefer_local_files = true        # use downloaded files instead of streaming
metadata_cache_ttl_minutes = 60  # how long cached playlists/favorites stay fresh
wal_max_entries = 1000           # max pending sync operations
wal_max_age_days = 7             # legacy field; pending writes never expire automatically

[theme]
# preset = "catppuccin-mocha"  # or: nord, dracula, gruvbox, solarized, tokyo-night
# Or define custom colors:
# primary = "#89b4fa"
# secondary = "#cba6f7"
# background = "#1e1e2e"
```

S3 and Celld configuration, credential scope, and migration limits are in [the storage runbook](docs/storage-s3.md).

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `h/j/k/l` | Move left/down/up/right |
| `Tab` | Cycle tabs/panels |
| `gg` | Jump to top |
| `ge` | Jump to end |
| `Esc` | Back/cancel |

### Playback

| Key | Action |
|-----|--------|
| `Space+p` | Pause/resume |
| `Space+n` | Next track |
| `Space+b` | Previous track |
| `r` | Toggle repeat |
| `s` | Toggle shuffle |
| `1` | Toggle single mode |

### Volume & Seek

| Key | Action |
|-----|--------|
| `+/-` | Volume up/down |
| `[/]` or `</>` or `,/.` | Seek backward/forward |

### Queue

| Key | Action |
|-----|--------|
| `w` | Toggle queue panel |
| `y` | Add to queue (yank) |
| `Y` | Add all to queue |
| `d` | Remove from queue |
| `D` | Clear entire queue |
| `J/K` | Move track down/up in queue |
| `Enter/p` | Play selected |

### Views

| Key | Action |
|-----|--------|
| `b` | Browse playlists |
| `/` | Search |
| `L` | Library/Favorites |
| `W` | Downloads view |
| `v` | View artist/album detail |
| `Space+v` | Toggle visualizer |

### Downloads & Playlists

| Key | Action |
|-----|--------|
| `O` | Download track |
| `S` | Sync playlist |
| `o` | Toggle offline mode |
| `f` | Add/remove favorite |
| `R` | Toggle radio mode / Retry download |
| `C` | Create new playlist |
| `a` | Add track to playlist |

### System

| Key | Action |
|-----|--------|
| `Space+q` | Quit |
| `Space+c` | Clear debug log |
| `Space+e` | Export debug log |
| `?` | Show help |

## Architecture

```
drift/
├── Cargo.toml              # Workspace root
├── src/
│   ├── lib.rs              # Library crate (shared by all binaries)
│   ├── main.rs             # TUI binary
│   ├── bin/
│   │   ├── sync.rs         # drift-sync bulk downloader
│   │   └── tidal_db.rs     # tidal-db CLI tool
│   ├── app/                # Application state and logic
│   ├── ui/                 # Ratatui TUI rendering
│   ├── handlers/           # Keyboard input handling
│   ├── service/            # Music service backends (Tidal, YouTube, Bandcamp)
│   ├── storage/            # Local storage, replication core, S3/Celld adapters
│   ├── sync/               # Bulk library sync engine
│   ├── config.rs           # Configuration types
│   ├── downloads.rs        # Download management
│   ├── download_db.rs      # Download history (redb)
│   ├── history_db.rs       # Play history (redb)
│   ├── search.rs           # Search with fuzzy filtering
│   ├── search_cache.rs     # Search result cache
│   └── queue_persistence.rs # Queue save/restore
└── crates/
    └── drift-plugin/       # Server-side plugin logic (dedup, TTL, pruning)
```

### Storage: local-first, multiplayer second

```
App reads/writes ──► LocalFirstStorage
                     ├─ LocalStorage (redb)     ◄── all reads, all writes (always)
                     ├─ MetadataCache (redb)     ◄── playlists, favorites, albums, artists
                     ├─ WalManager (redb)        ◄── persistent write-ahead log for sync
                     └─ S3Storage (optional)     ◄── S3 blobs + S3 or Celld metadata
```

The `DriftStorage` trait abstracts all persistence. `LocalFirstStorage` is the default backend:

- **Playback metadata reads** use local storage. Explicit remote blob requests use the network.
- **Local writes** queue for replication when sync is enabled.
- **Remote queues** use Lamport order. History uses stable play identities. Playlist writes and tombstones use deterministic last-write order.
- **Pending operations** survive restart — the WAL is a redb database, not in-memory
- **Metadata cache** stores playlists, favorites, albums, artists with TTL-based staleness
- **Playback prefers local files** — downloaded tracks play instantly without API calls

S3 keys use `<prefix>/users/<user>/state.json` and `<prefix>/users/<user>/blobs/<blake3>`.
Celld mode keeps the same blobs and stores the metadata snapshot in a dedicated Durable Object.

## drift-sync

Bulk download your entire Tidal library at maximum quality:

```sh
drift-sync                     # output: ~/Music/Tidal/
drift-sync -o /path/to/music   # custom output directory
```

Downloads all favorite albums, tracks, and playlists at HI_RES_LOSSLESS quality with automatic fallback. Shares the download database (`.tidal-dl.redb`) with the TUI, so previously downloaded tracks are automatically skipped.

Credentials are loaded from `~/.config/drift/credentials.json` or `~/.config/tidal-tui/credentials.json`.

The optional [shared Tidal authorization broker](docs/tidal-authorization.md) renews tokens for the TUI and `drift-sync` without giving them a refresh token.
It also exposes an access-only command for a restricted SearXNG connection. Deployment is not yet enabled.

## Development

```sh
# Enter dev shell
nix develop

# Run all tests
cargo test --workspace

# Run S3 and local-only feature tests
cargo test --all-targets --all-features
cargo test --all-targets --no-default-features
node --test celld/worker.test.mjs

# Regenerate the Nix dependency graph after dependency changes
nix run .#update-plan

# Build release
cargo build --release --features s3
```

## References

- [Celld v0.3.0](https://github.com/denoland/celld/tree/v0.3.0): Durable Object host, SQLite API, and durable-write acknowledgement contract.
- [RustFS](https://github.com/rustfs/rustfs): S3-compatible object storage. The isolated integration test used `1.0.0-rc.2`.
- [Onix Celld module](../onix-core/modules/celld/): separate fleet credentials, private listeners, and RustFS deployment conventions.
- [Apache object_store](https://docs.rs/object_store/0.12.5/object_store/): conditional S3 writes and AWS SigV4 signing.
- [Subwayrat](https://github.com/brittonr/subwayrat/tree/2e52b3150819a2365aaefd3dcf8bbd2a2fa2e901): pinned terminal widgets.

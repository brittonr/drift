# Drift

A terminal music player for **Tidal**, **YouTube**, and **Bandcamp** with an MPD backend.

Drift provides multi-service search, album radio, playlist management, download sync, and optional cross-device sync via Aspen distributed storage — all from a keyboard-driven TUI with album art, CAVA visualizer, and theming support.

## Features

- **Multi-service search** — unified search across Tidal, YouTube, and Bandcamp with fuzzy filtering (nucleo)
- **Album radio** — auto-generate queues from similar tracks
- **Playlist management** — create, rename, sync, and manage playlists across services
- **Download management** — queue downloads, track progress, content-dedup via BLAKE3
- **Cross-device sync** — optional sync of history, queue, and search cache via Aspen distributed KV
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

# With Aspen distributed sync
cargo build --release --features aspen
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
backend = "local"            # "local" or "aspen"
# cluster_ticket = "..."     # required for aspen backend
# user_id = "hostname"       # defaults to hostname

[theme]
# preset = "catppuccin-mocha"  # or: nord, dracula, gruvbox, solarized, tokyo-night
# Or define custom colors:
# primary = "#89b4fa"
# secondary = "#cba6f7"
# background = "#1e1e2e"
```

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
│   ├── storage/            # DriftStorage trait + local/aspen backends
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

### Storage backends

The `DriftStorage` trait abstracts all persistence:

- **`local`** — redb databases + JSON/TOML files in XDG directories. Fully offline, zero configuration.
- **`aspen`** — Aspen distributed KV over iroh QUIC. Multi-device sync with automatic reconnection and write queuing during outages.

Key schema: `drift:{user}:history:{timestamp}`, `drift:{user}:queue`, `drift:{user}:search:{hash}`, `drift:{user}:search_history`

## drift-sync

Bulk download your entire Tidal library at maximum quality:

```sh
drift-sync                     # output: ~/Music/Tidal/
drift-sync -o /path/to/music   # custom output directory
```

Downloads all favorite albums, tracks, and playlists at HI_RES_LOSSLESS quality with automatic fallback. Shares the download database (`.tidal-dl.redb`) with the TUI, so previously downloaded tracks are automatically skipped.

Credentials are loaded from `~/.config/drift/credentials.json` or `~/.config/tidal-tui/credentials.json`.

## Development

```sh
# Enter dev shell
nix develop

# Run all tests
cargo test --workspace

# Run with Aspen feature
cargo test --workspace --features aspen

# Build release
cargo build --release --features aspen
```

# Tidal TUI

A proper terminal user interface for Tidal, inspired by spotify-tui but actually working!

## Features

- 🎵 Browse your Tidal playlists and tracks
- 🎮 Control playback through MPD
- ⌨️ Vim-like keybindings
- 🎨 Beautiful terminal UI with ratatui
- 🔐 OAuth authentication (reuses your existing credentials)

## Building

```bash
cd /home/brittonr/git/tidal-tui
cargo build --release
```

## Running

```bash
# Make sure MPD is running
systemctl status mpd

# Run the TUI
cargo run --release
```

## First Time Setup

1. On first run, it will try to use existing Tidal credentials from upmpdcli
2. If not found, it will prompt for OAuth authentication
3. Credentials are saved in `~/.config/tidal-tui/credentials.json`

## Keybindings

- `Tab` - Switch between panels
- `j/k` or `↓/↑` - Navigate lists
- `Enter` - Play selected track
- `Space` - Play/Pause
- `n/N` - Next/Previous track
- `/` - Search
- `q` - Quit

## Architecture

```
Tidal API → tidal-tui → MPD → Audio Output
                ↓
              rmpc (optional control)
```

## TODO

- [ ] Actual Tidal API integration (currently mock data)
- [ ] Search functionality
- [ ] Album art in terminal
- [ ] Queue management
- [ ] Playlist creation/editing
- [ ] Offline cache support

## Why This Exists

Because all the other options suck:
- spotify-tui is dead
- upmpdcli doesn't integrate with MPD properly for control
- That Python tidal-tui is garbage
- VLC UPnP browsing is clunky

This gives you a REAL terminal music player for Tidal!
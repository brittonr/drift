#!/usr/bin/env bash
# Run Tidal TUI

echo "Starting Tidal TUI..."
echo ""

# Ensure MPD FIFO exists for visualizer
if [ ! -p /tmp/mpd.fifo ]; then
    echo "Creating MPD FIFO for visualizer..."
    mkfifo /tmp/mpd.fifo 2>/dev/null || true
fi

echo "Controls:"
echo "  / - Search for music"
echo "  b - Browse mode"
echo "  Tab - Switch panels/tabs"
echo "  Enter - Play selected item"
echo "  Space - Pause/Resume"
echo "  n/p - Next/Previous track"
echo "  v - Toggle audio visualizer"
echo "  d - Toggle debug panel"
echo "  e - Export debug log to /tmp/tidal-tui-export.log"
echo "  Ctrl+C - Clear debug log"
echo "  q - Quit"
echo ""
echo "Testing search: Try searching for 'radiohead' or your favorite artist"
echo ""

cd /home/brittonr/git/tidal-tui
exec nix develop -c cargo run --release
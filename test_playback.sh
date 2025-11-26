#!/usr/bin/env bash
# Test Tidal TUI playback

echo "Testing Tidal TUI playback..."
echo ""

# Clear MPD first
mpc clear >/dev/null 2>&1
mpc stop >/dev/null 2>&1

echo "Current MPD status:"
mpc status

echo ""
echo "Starting Tidal TUI..."
echo "Instructions:"
echo "  1. Press Tab to switch to tracks panel"
echo "  2. Press Enter on a track to play it"
echo "  3. Check the console output for debug messages"
echo "  4. Press q to quit"
echo ""
echo "Starting in 3 seconds..."
sleep 3

cd /home/brittonr/git/tidal-tui
nix develop -c cargo run --release
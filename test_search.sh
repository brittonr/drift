#!/usr/bin/env bash
# Test the search functionality of Tidal TUI

echo "Starting Tidal TUI to test search functionality..."
echo "Press '/' to open search, type query, Enter to search"
echo "Use Tab to switch between Tracks/Albums/Artists results"
echo "Press 'b' to go back to browse mode"
echo "Press 'q' to quit"
echo ""

cd /home/brittonr/git/tidal-tui
nix develop -c cargo run --release
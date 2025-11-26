#!/usr/bin/env bash

echo "Testing Tidal TUI streaming after search..."
echo ""
echo "This will capture streaming errors to help debug the 401 issue"
echo ""
echo "Instructions:"
echo "1. Press '/' to search"
echo "2. Search for 'test' or any artist"
echo "3. Press Enter to search"
echo "4. Select a track and press Enter to play"
echo "5. Watch for error messages below"
echo ""

cd /home/brittonr/git/tidal-tui

# Run and capture stderr
nix develop -c cargo run --release 2>&1 | tee /tmp/tidal-stream-test.log
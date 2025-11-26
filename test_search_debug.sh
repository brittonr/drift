#!/usr/bin/env bash
# Test search and capture debug output

echo "Testing Tidal TUI search with debug output..."
echo "This will search for 'test' and show the API response structure"
echo ""

cd /home/brittonr/git/tidal-tui

# Run briefly and capture stderr
timeout 10 nix develop -c cargo run --release 2>/tmp/tidal-search-debug.log &
PID=$!

# Wait a bit for app to start
sleep 2

# Send keystrokes to search (if possible with the app running)
echo "App is running. Manual test needed:"
echo "1. Press '/' to open search"
echo "2. Type 'test' and press Enter"
echo "3. Wait for results"
echo "4. Press 'q' to quit"

wait $PID 2>/dev/null

echo ""
echo "=== Debug output from search: ==="
grep -A50 "DEBUG: Track item structure" /tmp/tidal-search-debug.log 2>/dev/null || echo "No debug output found"

echo ""
echo "=== Search-related errors: ==="
grep -i "unknown artist\|artist\|error" /tmp/tidal-search-debug.log | head -20
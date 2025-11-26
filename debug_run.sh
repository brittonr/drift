#!/usr/bin/env bash
# Run Tidal TUI and monitor debug output

echo "Starting Tidal TUI with debug monitoring..."
echo "Debug log will be saved to: /tmp/tidal-tui-debug.log"
echo ""

# Clear old log
rm -f /tmp/tidal-tui-debug.log
touch /tmp/tidal-tui-debug.log

# Run the app in background
cd /home/brittonr/git/tidal-tui
nix develop -c cargo run --release &
APP_PID=$!

echo "Tidal TUI started (PID: $APP_PID)"
echo "Monitoring debug log..."
echo "========================================="
echo ""

# Monitor the log file
tail -f /tmp/tidal-tui-debug.log &
TAIL_PID=$!

# Wait for app to exit
wait $APP_PID

# Kill tail
kill $TAIL_PID 2>/dev/null

echo ""
echo "========================================="
echo "App closed. Full debug log saved to: /tmp/tidal-tui-debug.log"
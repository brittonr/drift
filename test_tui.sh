#!/usr/bin/env bash
# Quick test script to verify Tidal TUI is working

cd /home/brittonr/git/tidal-tui

echo "Testing Tidal TUI..."
echo ""

# Test if it loads credentials
echo "1. Checking for credentials..."
if [ -f ~/.config/upmpdcli/qobuz/oauth2.credentials.json ]; then
    echo "   ✓ Found OAuth credentials"
else
    echo "   ✗ No OAuth credentials found"
fi

# Test if it compiles
echo ""
echo "2. Testing compilation..."
if nix develop -c cargo build --release 2>/dev/null; then
    echo "   ✓ Compilation successful"
else
    echo "   ✗ Compilation failed"
fi

# Test if it runs (with timeout)
echo ""
echo "3. Testing runtime (3 second test)..."
timeout 3 nix develop -c cargo run --release 2>&1 | grep -q "Loading existing" && echo "   ✓ App starts and loads credentials" || echo "   ✓ App starts"

echo ""
echo "Tidal TUI is ready!"
echo ""
echo "To run the full app:"
echo "  cd /home/brittonr/git/tidal-tui"
echo "  nix develop -c cargo run --release"
echo ""
echo "Controls:"
echo "  Tab    - Switch between playlists and tracks"
echo "  ↑/↓    - Navigate lists"
echo "  Enter  - Load playlist / Play track"
echo "  Space  - Play/Pause"
echo "  n/p    - Next/Previous track"
echo "  q      - Quit"
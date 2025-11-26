#!/usr/bin/env bash
# Test script to verify queue functionality

echo "Testing Tidal TUI Queue Functionality"
echo "====================================="
echo ""

# Clear any existing MPD queue
echo "1. Clearing MPD queue..."
mpc clear
echo ""

# Check current status
echo "2. Current MPD status:"
mpc status
echo ""

echo "3. To test the queue:"
echo "   - Run: ./run.sh"
echo "   - Press 'a' to add a single track to queue"
echo "   - Press 'A' to add all tracks to queue"
echo "   - Press 'q' to toggle queue display"
echo "   - Press 'Q' to clear queue"
echo "   - Press 'Delete' to remove selected track from queue"
echo ""
echo "4. The queue should now properly show added tracks!"
echo ""
echo "5. Check debug log for details:"
echo "   tail -f /tmp/tidal-tui-debug.log"
#!/usr/bin/env bash
# Test script to verify Tidal API integration

cd /home/brittonr/git/tidal-tui

echo "Testing Tidal API Integration"
echo "=============================="
echo ""

# Check if credentials exist
echo "1. Checking credentials..."
if [ -f ~/.config/tidal-tui/credentials.json ]; then
    echo "   ✓ Found credentials file"
    echo "   User ID: $(jq -r '.user_id' ~/.config/tidal-tui/credentials.json)"
    echo ""
else
    echo "   ✗ No credentials found"
    exit 1
fi

# Test API directly with curl
echo "2. Testing direct API access..."
TOKEN=$(jq -r '.access_token' ~/.config/tidal-tui/credentials.json)
USER_ID=$(jq -r '.user_id' ~/.config/tidal-tui/credentials.json)

# Test fetching playlists
echo "   Testing playlist fetch..."
RESPONSE=$(curl -s -w "\n%{http_code}" \
    -H "Authorization: Bearer $TOKEN" \
    "https://api.tidal.com/v1/users/$USER_ID/playlists?countryCode=US&limit=3")

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | head -n-1)

if [ "$HTTP_CODE" = "200" ]; then
    echo "   ✓ API call successful (HTTP $HTTP_CODE)"
    PLAYLIST_COUNT=$(echo "$BODY" | jq '.items | length')
    echo "   Found $PLAYLIST_COUNT playlists"

    # Show first playlist
    if [ "$PLAYLIST_COUNT" -gt 0 ]; then
        echo ""
        echo "   First playlist:"
        echo "$BODY" | jq -r '.items[0] | "     - Title: \(.title)\n     - ID: \(.uuid)\n     - Tracks: \(.numberOfTracks)"'
    fi
else
    echo "   ✗ API call failed (HTTP $HTTP_CODE)"
    echo "   Response: $BODY" | head -3
fi

echo ""
echo "3. Running TUI app (5 second test)..."
echo "   The app should display real playlists if API is working"
echo ""
echo "Press any key to start the app test..."
read -n 1

timeout 5 nix develop -c cargo run --release 2>&1 | grep -E "(Loading|Successfully|playlists|API)" || true

echo ""
echo "Test complete!"
echo ""
echo "To run the full app:"
echo "  cd /home/brittonr/git/tidal-tui"
echo "  ./tidal"
#!/usr/bin/env bash
# Test Tidal streaming URL

echo "Testing Tidal streaming URL..."

TOKEN=$(jq -r '.access_token' ~/.config/tidal-tui/credentials.json)
TRACK_ID="109308530"  # Test track

echo "Track ID: $TRACK_ID"
echo ""

# Try different endpoints
echo "1. Testing /streamUrl endpoint..."
RESPONSE=$(curl -s -H "Authorization: Bearer $TOKEN" \
    "https://api.tidal.com/v1/tracks/$TRACK_ID/streamUrl?countryCode=US&soundQuality=LOSSLESS")

echo "Response:"
echo "$RESPONSE" | jq '.' 2>/dev/null || echo "$RESPONSE"

# Check if we got a URL
URL=$(echo "$RESPONSE" | jq -r '.url // empty' 2>/dev/null)
if [ -n "$URL" ]; then
    echo ""
    echo "Got streaming URL:"
    echo "$URL" | head -c 100
    echo "..."
else
    echo ""
    echo "2. Testing /playbackinfopostpaywall endpoint..."
    RESPONSE=$(curl -s -H "Authorization: Bearer $TOKEN" \
        "https://api.tidal.com/v1/tracks/$TRACK_ID/playbackinfopostpaywall?countryCode=US&soundQuality=LOSSLESS&assetPresentation=FULL")

    echo "Response:"
    echo "$RESPONSE" | jq '.' | head -30
fi
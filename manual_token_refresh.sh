#!/usr/bin/env bash
# Manually refresh Tidal OAuth token using refresh token

echo "Manual Tidal Token Refresh"
echo "=========================="
echo ""

# Load current refresh token
REFRESH_TOKEN=$(jq -r '.refresh_token' ~/.config/tidal-tui/credentials.json)
USER_ID=$(jq -r '.user_id' ~/.config/tidal-tui/credentials.json)

if [ -z "$REFRESH_TOKEN" ]; then
    echo "✗ No refresh token found!"
    exit 1
fi

echo "Using refresh token to get new access token..."
echo "User ID: $USER_ID"
echo ""

# Try different client IDs
CLIENT_IDS=("zU4XHVVkc2tDPo4t" "_DSTon1kC8pABnTw" "13319")

for CLIENT_ID in "${CLIENT_IDS[@]}"; do
    echo "Trying client_id: $CLIENT_ID"

    RESPONSE=$(curl -s -X POST \
        -d "grant_type=refresh_token" \
        -d "refresh_token=$REFRESH_TOKEN" \
        -d "client_id=$CLIENT_ID" \
        "https://auth.tidal.com/v1/oauth2/token")

    # Check if we got an access token
    if echo "$RESPONSE" | jq -e '.access_token' >/dev/null 2>&1; then
        echo "✓ Token refreshed successfully!"

        # Extract new tokens
        NEW_ACCESS_TOKEN=$(echo "$RESPONSE" | jq -r '.access_token')
        NEW_REFRESH_TOKEN=$(echo "$RESPONSE" | jq -r '.refresh_token // empty')

        # Update credentials file
        if [ -n "$NEW_REFRESH_TOKEN" ]; then
            jq --arg access "$NEW_ACCESS_TOKEN" --arg refresh "$NEW_REFRESH_TOKEN" \
                '.access_token = $access | .refresh_token = $refresh' \
                ~/.config/tidal-tui/credentials.json > /tmp/new_creds.json
        else
            jq --arg access "$NEW_ACCESS_TOKEN" \
                '.access_token = $access' \
                ~/.config/tidal-tui/credentials.json > /tmp/new_creds.json
        fi

        mv /tmp/new_creds.json ~/.config/tidal-tui/credentials.json
        echo "✓ Saved new tokens to credentials.json"

        # Test if it works
        echo ""
        echo "Testing new token..."
        RESPONSE=$(curl -s -w "\n%{http_code}" \
            -H "Authorization: Bearer $NEW_ACCESS_TOKEN" \
            "https://api.tidal.com/v1/users/$USER_ID/playlists?countryCode=US&limit=1")

        HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
        if [ "$HTTP_CODE" = "200" ]; then
            echo "✓ New token works! (HTTP 200)"
            echo ""
            echo "Success! You can now run:"
            echo "  /home/brittonr/git/tidal-tui/tidal"
        else
            echo "✗ New token test failed (HTTP $HTTP_CODE)"
        fi

        exit 0
    else
        echo "✗ Failed: $(echo "$RESPONSE" | jq -r '.error // "Unknown error"' 2>/dev/null)"
    fi
done

echo ""
echo "✗ All client IDs failed to refresh token."
echo ""
echo "Please re-authenticate through VLC:"
echo "1. Open VLC"
echo "2. Browse to UPnP → upmpdcli → Tidal"
echo "3. This should trigger authentication"
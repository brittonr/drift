#!/usr/bin/env bash
# Copy existing Tidal credentials from upmpdcli

UPMPDCLI_CREDS="/var/cache/upmpdcli/tidal/oauth2.credentials.json"
TIDAL_TUI_DIR="$HOME/.config/tidal-tui"
TIDAL_TUI_CREDS="$TIDAL_TUI_DIR/credentials.json"

if [ -f "$UPMPDCLI_CREDS" ]; then
    echo "Found existing Tidal credentials from upmpdcli!"

    mkdir -p "$TIDAL_TUI_DIR"

    # Convert the format
    python3 -c "
import json
with open('$UPMPDCLI_CREDS', 'r') as f:
    data = json.load(f)

# Extract the actual values from the nested format
output = {
    'access_token': data['access_token']['data'],
    'refresh_token': data['refresh_token']['data'],
    'token_type': data['token_type']['data']
}

with open('$TIDAL_TUI_CREDS', 'w') as f:
    json.dump(output, f, indent=2)
"

    echo "Credentials copied to $TIDAL_TUI_CREDS"
else
    echo "No existing credentials found. You'll need to authenticate on first run."
fi
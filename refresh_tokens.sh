#!/usr/bin/env bash
# Copy fresh tokens from upmpdcli

echo "Copying fresh tokens from upmpdcli..."

# Convert upmpdcli format to our format
python3 << 'PYTHON'
import json

# Read upmpdcli credentials
with open('/var/cache/upmpdcli/tidal/oauth2.credentials.json', 'r') as f:
    upmpd_creds = json.load(f)

# Extract the actual token values
tidal_creds = {
    "access_token": upmpd_creds["access_token"]["data"],
    "refresh_token": upmpd_creds["refresh_token"]["data"],
    "token_type": upmpd_creds["token_type"]["data"],
    "user_id": 206488729
}

# Save to Tidal TUI format
import os
config_dir = os.path.expanduser("~/.config/tidal-tui")
os.makedirs(config_dir, exist_ok=True)

with open(f"{config_dir}/credentials.json", 'w') as f:
    json.dump(tidal_creds, f, indent=2)

print("✓ Tokens copied to ~/.config/tidal-tui/credentials.json")
PYTHON

echo ""
echo "Testing if tokens work..."
cd /home/brittonr/git/tidal-tui
./test_api.sh

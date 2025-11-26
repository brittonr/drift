#!/usr/bin/env python3
"""
Manually refresh Tidal OAuth token using refresh token
"""

import json
import requests
import sys
from pathlib import Path

def refresh_token():
    # Load current credentials
    creds_path = Path.home() / ".config/tidal-tui/credentials.json"

    if not creds_path.exists():
        print("No credentials file found!")
        return False

    with open(creds_path) as f:
        creds = json.load(f)

    print("Current credentials loaded")
    print(f"User ID: {creds.get('user_id')}")

    # Tidal OAuth token refresh endpoint
    token_url = "https://auth.tidal.com/v1/oauth2/token"

    # Try different client IDs
    client_ids = [
        "zU4XHVVkc2tDPo4t",  # Known working Android client ID
        "_DSTon1kC8pABnTw",  # Another known working ID
        "13319",  # From your JWT
    ]

    for client_id in client_ids:
        print(f"\nTrying client_id: {client_id}")

        # Request new access token using refresh token
        data = {
            "grant_type": "refresh_token",
            "refresh_token": creds["refresh_token"],
            "client_id": client_id,
        }

        response = requests.post(token_url, data=data)

        if response.status_code == 200:
            print("✓ Token refreshed successfully!")
            new_tokens = response.json()

            # Update credentials
            creds["access_token"] = new_tokens["access_token"]
            if "refresh_token" in new_tokens:
                creds["refresh_token"] = new_tokens["refresh_token"]

            # Save updated credentials
            with open(creds_path, 'w') as f:
                json.dump(creds, f, indent=2)

            print("✓ New tokens saved to credentials.json")

            # Also update upmpdcli cache if writable
            upmpd_path = Path("/var/cache/upmpdcli/tidal/oauth2.credentials.json")
            if upmpd_path.exists():
                try:
                    upmpd_format = {
                        "token_type": {"data": creds["token_type"]},
                        "session_id": {"data": "refreshed-session"},
                        "access_token": {"data": creds["access_token"]},
                        "refresh_token": {"data": creds["refresh_token"]},
                        "is_pkce": {"data": False}
                    }
                    with open(upmpd_path, 'w') as f:
                        json.dump(upmpd_format, f)
                    print("✓ Updated upmpdcli credentials too")
                except PermissionError:
                    print("✗ Could not update upmpdcli credentials (permission denied)")

            return True
        else:
            print(f"✗ Failed with status {response.status_code}")
            if response.text:
                error_data = response.json() if response.headers.get('content-type', '').startswith('application/json') else response.text
                print(f"  Error: {error_data}")

    print("\n✗ All client IDs failed. Manual re-authentication required.")
    return False

if __name__ == "__main__":
    if refresh_token():
        print("\n✓ Success! Test with: /home/brittonr/git/tidal-tui/test_api.sh")
    else:
        print("\n✗ Token refresh failed. Please re-authenticate through VLC/upmpdcli.")
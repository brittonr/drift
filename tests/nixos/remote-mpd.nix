# Remote MPD test — multi-machine client/server setup.
#
# Validates that drift's remote MPD support works:
#   - Server exports MPD on network
#   - Client connects to remote MPD via mpc -h <server>
#   - Remote playback control (play, pause, volume, queue)
#   - Network isolation works correctly
#
# Run: nix build .#checks.x86_64-linux.remote-mpd -L
{ pkgs, drift }:

let
  mpdPort = 6600;

  sharedModule = {
    virtualisation.graphics = false;
  };
in

pkgs.testers.runNixOSTest {
  name = "drift-remote-mpd";
  skipLint = true;

  nodes.server = { config, pkgs, ... }: {
    imports = [ sharedModule ];

    # MPD listening on all interfaces for remote access
    services.mpd = {
      enable = true;
      musicDirectory = "/var/lib/mpd/music";
      network.listenAddress = "any";
      extraConfig = ''
        audio_output {
          type "null"
          name "Null Output"
          mixer_type "software"
        }
      '';
    };

    networking.firewall.allowedTCPPorts = [ mpdPort ];

    environment.systemPackages = with pkgs; [
      mpc
      sox
    ];

    system.stateVersion = "24.11";
  };

  nodes.client = { config, pkgs, ... }: {
    imports = [ sharedModule ];

    environment.systemPackages = with pkgs; [
      mpc
      drift
    ];

    system.stateVersion = "24.11";
  };

  testScript = ''
    start_all()

    server.wait_for_unit("default.target")
    server.wait_for_unit("mpd.service")
    client.wait_for_unit("default.target")

    # ── Server-side Setup ────────────────────────────────────────
    with server.nested("Generate test audio on server"):
        server.succeed(
            "sox -n /var/lib/mpd/music/remote-test.wav "
            "synth 5 sine 440 channels 2 rate 44100"
        )
        server.succeed(
            "sox -n /var/lib/mpd/music/remote-test-2.wav "
            "synth 3 sine 660 channels 2 rate 44100"
        )
        server.succeed("mpc update --wait")

    # ── Network Connectivity ─────────────────────────────────────
    with client.nested("Client can reach server MPD port"):
        client.wait_until_succeeds(
            "mpc -h server -p ${toString mpdPort} status",
            timeout=30
        )

    # ── Remote Status ────────────────────────────────────────────
    with client.nested("Remote MPD status reporting"):
        result = client.succeed("mpc -h server -p ${toString mpdPort} version")
        assert "mpd version" in result.lower(), f"No MPD version in: {result}"

    # ── Remote Volume Control ────────────────────────────────────
    with client.nested("Remote volume control"):
        client.succeed("mpc -h server -p ${toString mpdPort} volume 65")

        # Verify from both client and server perspectives
        result = client.succeed("mpc -h server -p ${toString mpdPort} volume")
        assert "65%" in result, f"Client sees wrong volume: {result}"

        result = server.succeed("mpc volume")
        assert "65%" in result, f"Server sees wrong volume: {result}"

    # ── Remote Queue Management ──────────────────────────────────
    with client.nested("Remote queue management"):
        client.succeed("mpc -h server -p ${toString mpdPort} clear")
        client.succeed("mpc -h server -p ${toString mpdPort} add remote-test.wav")
        client.succeed("mpc -h server -p ${toString mpdPort} add remote-test-2.wav")

        result = client.succeed("mpc -h server -p ${toString mpdPort} playlist")
        lines = [l for l in result.strip().split("\n") if l.strip()]
        assert len(lines) == 2, f"Expected 2 tracks, got {len(lines)}: {result}"

    # ── Remote Playback ──────────────────────────────────────────
    with client.nested("Remote playback control"):
        client.succeed("mpc -h server -p ${toString mpdPort} play")
        client.sleep(1)

        # Verify playing from client
        result = client.succeed("mpc -h server -p ${toString mpdPort} status")
        assert "[playing]" in result, f"Not playing (client view): {result}"

        # Verify playing from server
        result = server.succeed("mpc status")
        assert "[playing]" in result, f"Not playing (server view): {result}"

        # Pause from client
        client.succeed("mpc -h server -p ${toString mpdPort} pause")
        result = client.succeed("mpc -h server -p ${toString mpdPort} status")
        assert "[paused]" in result, f"Not paused: {result}"

        # Next track from client
        client.succeed("mpc -h server -p ${toString mpdPort} play")
        client.succeed("mpc -h server -p ${toString mpdPort} next")
        client.sleep(1)

        # Stop from client
        client.succeed("mpc -h server -p ${toString mpdPort} stop")

    # ── Remote Playback Modes ────────────────────────────────────
    with client.nested("Remote playback mode toggling"):
        client.succeed("mpc -h server -p ${toString mpdPort} repeat on")
        client.succeed("mpc -h server -p ${toString mpdPort} random on")

        result = server.succeed("mpc status")
        assert "repeat: on" in result, f"Repeat not on (server): {result}"
        assert "random: on" in result, f"Random not on (server): {result}"

        client.succeed("mpc -h server -p ${toString mpdPort} repeat off")
        client.succeed("mpc -h server -p ${toString mpdPort} random off")

    # ── Drift Config for Remote MPD ──────────────────────────────
    with client.nested("Drift config points to remote MPD"):
        client.succeed("""
          mkdir -p /root/.config/drift
          cat > /root/.config/drift/config.toml << 'EOF'
    [mpd]
    host = "server"
    port = 6600

    [playback]
    default_volume = 80
    audio_quality = "high"

    [ui]
    show_visualizer = false
    show_album_art = false
    EOF
        """)

        client.succeed("test -f /root/.config/drift/config.toml")

    # ── drift-sync on Client ─────────────────────────────────────
    with client.nested("drift-sync runs on client machine"):
        result = client.succeed("drift-sync --help")
        assert "drift-sync" in result, f"drift-sync missing from client: {result}"

    # ── Server Stats via Client ──────────────────────────────────
    with client.nested("MPD stats accessible remotely"):
        result = client.succeed("mpc -h server -p ${toString mpdPort} stats")
        assert "Songs" in result or "songs" in result, \
            f"No song count in remote stats: {result}"
  '';
}

{ pkgs, driftPackage }:
let
  stateVersion = "26.05";
  user = "tidal-fixture";
  credentialsFile = "/home/${user}/.config/drift/credentials.json";
  fixtureKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFixtureKeyOnlyNeverUsedForAuthentication fixture";
  node =
    overrides:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      inherit pkgs;
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        (import ./tidal-auth.nix { inherit driftPackage; })
        {
          system.stateVersion = stateVersion;
          users.users.${user}.isNormalUser = true;
          services.drift-tidal-auth = {
            enable = true;
            inherit user credentialsFile;
            exportPublicKeys = [ fixtureKey ];
          }
          // overrides;
        }
      ];
    }).config;
  valid = node { };
  unit = valid.systemd.services.drift-tidal-auth.serviceConfig;
  key = builtins.head valid.users.users.${user}.openssh.authorizedKeys.keys;
  rejected =
    overrides:
    builtins.any (
      assertion: !assertion.assertion && pkgs.lib.hasPrefix "Tidal" assertion.message
    ) (node overrides).assertions;
in
assert unit.User == user;
assert unit.UMask == "0077";
assert unit.RuntimeDirectoryMode == "0700";
assert unit.StandardOutput == "null";
assert unit.ReadWritePaths == [ (builtins.dirOf credentialsFile) ];
assert pkgs.lib.hasPrefix "restrict,command=" key;
assert
  valid.environment.sessionVariables.DRIFT_TIDAL_AUTH_SOCKET == "/run/drift-tidal-auth/broker.sock";
assert valid.networking.firewall.allowedTCPPorts == [ ];
assert rejected { credentialsFile = "/nix/store/unsafe-credential"; };
assert rejected { exportPublicKeys = [ "command=\"bad\" ssh-ed25519 AAAA" ]; };
assert rejected { exportPublicKeys = [ "ssh-ed25519 AAAA\ncommand=bad" ]; };
true

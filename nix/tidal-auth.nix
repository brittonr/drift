# Import through the pinned Drift flake. No credential value enters the store.
{ driftPackage }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.drift-tidal-auth;
  runtimeDirectory = "drift-tidal-auth";
  socket = "/run/${runtimeDirectory}/broker.sock";
  restartSeconds = 30;
  getCommand = pkgs.writeShellScript "drift-tidal-auth-export" ''
    exec ${cfg.package}/bin/drift-tidal-auth get ${lib.escapeShellArg socket}
  '';
in
{
  options.services.drift-tidal-auth = {
    enable = lib.mkEnableOption "single-writer Tidal authorization for Drift clients";
    package = lib.mkOption {
      type = lib.types.package;
      default = driftPackage;
    };
    user = lib.mkOption {
      type = lib.types.str;
      description = "Existing OS account that owns Drift's private credential directory.";
    };
    credentialsFile = lib.mkOption {
      type = lib.types.str;
      description = "Absolute path to canonical Drift credentials.json; never a store path.";
    };
    exportPublicKeys = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "SSH public keys restricted to access-only token export; no shell or forwarding.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion =
          lib.hasPrefix "/" cfg.credentialsFile && !(lib.hasPrefix "/nix/store/" cfg.credentialsFile);
        message = "Tidal credentials must be an absolute runtime path outside the store.";
      }
      {
        assertion = builtins.all (
          key: builtins.match "ssh-ed25519 [A-Za-z0-9+/=]+( [^\n\r]*)?" key != null
        ) cfg.exportPublicKeys;
        message = "Tidal exporter accepts plain Ed25519 public keys only.";
      }
    ];
    environment.sessionVariables.DRIFT_TIDAL_AUTH_SOCKET = socket;
    users.users.${cfg.user}.openssh.authorizedKeys.keys = map (
      key: ''restrict,command="${getCommand}" ${key}''
    ) cfg.exportPublicKeys;
    systemd.services.drift-tidal-auth = {
      description = "Single-writer Tidal authorization broker";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      serviceConfig = {
        User = cfg.user;
        ExecStart = "${cfg.package}/bin/drift-tidal-auth serve ${socket} ${lib.escapeShellArg cfg.credentialsFile}";
        RuntimeDirectory = runtimeDirectory;
        RuntimeDirectoryMode = "0700";
        UMask = "0077";
        Restart = "on-failure";
        RestartSec = restartSeconds;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = "read-only";
        ReadWritePaths = [ (builtins.dirOf cfg.credentialsFile) ];
        PrivateTmp = true;
        ProtectKernelTunables = true;
        ProtectControlGroups = true;
        RestrictSUIDSGID = true;
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
        ];
        StandardOutput = "null";
        StandardError = "journal";
      };
    };
  };
}

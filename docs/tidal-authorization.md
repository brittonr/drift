# Shared Tidal authorization

Status: local implementation. No live account migration or deployment occurred.

## Ownership

`drift-tidal-auth` is the only refresh owner in broker mode. It keeps the refresh token in Drift's private credential file.
The TUI and `drift-sync` receive an access token, expiry, and account ID through a private Unix socket.
SearXNG can use the access-only `get` command through a restricted SSH key. It receives the access token and expiry, but no refresh token or account ID.

`DRIFT_TIDAL_AUTH_SOCKET` selects broker mode. An unavailable broker causes an error, not a fallback to credential-file access.
Without that variable, the updated Rust clients hold the same credential lock for their lifetime. A broker cannot start beside them.
The old Python `tidal-dl` refuses broker mode. Use `drift-sync` for bulk downloads in this mode.
Old installed binaries and external applications do not honor the new lock. They must stop before migration.

## Renewal

The pure core determines whether to renew. It checks expiry before each access request, with a five-minute margin.
Missing expiry also requires renewal. A client can report a rejected access token after HTTP 401.
The broker serializes requests. If another request already replaced that token, the broker returns the replacement instead of rotating again.
The SearXNG access-only SSH route requests proactive renewal. It cannot force token rotation after an unexpected rejection of a still-valid token.

A complete OAuth response must contain a valid access token, Bearer token type, and bounded positive lifetime.
An omitted refresh token preserves the previous refresh token. An empty or malformed refresh token rejects the response.
The shell bounds the HTTP deadline and response size, checks TLS, and rejects redirects.
The client applies one socket deadline across connection, request, and response. Slow partial replies cannot extend that deadline.
It replaces the credential file atomically and syncs the file and directory before it returns the new access token.

## Uncertain renewal

Before the HTTP request, the broker writes and syncs a private renewal-intent record.
A failed or interrupted request can leave the remote token state uncertain. The broker refuses another renewal in that state, including after restart.
If a durable credential replacement differs from the recorded fingerprint, startup completes the interrupted local cleanup.
Otherwise, recovery requires the operator to establish current account authorization. The broker does not guess whether the previous refresh succeeded.
Do not remove an intent merely to force another request. Preserve the old files and establish a fresh login before an operator clears an unresolved intent.

This favors account safety over retry availability. Ordinary expiry is automatic. Revocation and uncertain remote outcomes still require operator recovery.

## Interface

Local protocol: one bounded JSON line per Unix connection.

```json
{"operation":"get"}
```

The refresh operation also supplies `rejected_access_token`. The token goes in the private message, never in command arguments.
Native socket replies contain `access_token`, `expires_at`, and `user_id`. The remote `get` command omits `user_id`.
Error replies contain only a fixed error category.
Socket and credential directories require mode `0700`; files and sockets require `0600`.
The broker holds separate OS locks for the credential directory and socket directory. The two directories must be distinct.

```console
drift-tidal-auth serve /run/drift-tidal-auth/broker.sock /home/USER/.config/drift/credentials.json
drift-tidal-auth get /run/drift-tidal-auth/broker.sock
```

The second command refuses terminal output. Its output is secret and must go directly to the authorized consumer.

## NixOS module

The flake exports `nixosModules.tidal-auth` and `packages.<system>.drift-tidal-auth`.
The module requires an explicit user and canonical credential path. Optional Ed25519 export keys receive a fixed command with no shell or forwarding.
The module sets `DRIFT_TIDAL_AUTH_SOCKET` for new sessions. Existing sessions need the variable and all old Drift processes must stop before broker activation.
The module adds no public listening port. The SSH consumer must pin the server host key and use a dedicated private key.

The canonical file must exist before startup. Perform any legacy credential migration with the broker stopped.
A pinned published Drift revision is required for downstream production consumption. Do not depend on a sibling worktree.

## Validation scope

Tests use synthetic credentials. They cover renewal decisions, malformed replies, private access projection, single-writer locks, queued rejection coalescing, restart recovery, and the native socket.
Full production acceptance also requires the strict lint gate, Nix package checks, both Drift consumers, and an authorized SearXNG search after actual renewal.
No test result alone proves that the live account was migrated.

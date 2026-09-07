# S3 and Celld storage

Drift uses S3-compatible storage instead of the removed Aspen client. RustFS supplies the S3 API. Celld optionally coordinates metadata updates.

This change does not deploy a production service or copy data from an old Aspen cluster.

## Architecture

The application owns deterministic replication documents in `src/storage/replication.rs`. Application-owned ports separate metadata and blob capabilities from their adapters.

`src/storage/s3.rs` performs file reads, content checks, and replication. `src/storage/s3_adapter.rs` implements S3 and Celld transport.

Audio bytes always use S3. Metadata uses one of these explicit modes:

- **S3 mode:** one account snapshot with conditional creation and ETag-based updates.
- **Celld mode:** the supplied worker stores that snapshot in SQLite. Each update uses one atomic conditional SQL statement.

There is no automatic fallback between metadata modes. A fallback can split account state between two authorities.

Celld v0.3.0 holds write responses behind its durability and owner-fencing gate. The worker does not replace that gate.

## Client configuration

The existing application configuration remains TOML. No secret belongs in this file.

```toml
[storage]
backend = "local"
sync_enabled = true
user_id = "alice"
device_id = "laptop"
wal_max_entries = 1000

[storage.s3]
endpoint = "https://s3.example.net"
bucket = "drift-audio"
region = "us-east-1"
prefix = "drift/v1"
access_key_env = "DRIFT_S3_ACCESS_KEY_ID"
secret_key_env = "DRIFT_S3_SECRET_ACCESS_KEY"
allow_http = false

# Optional. Omit both fields to keep metadata directly in S3.
celld_endpoint = "https://drift-metadata.example.net"
celld_token_env = "DRIFT_CELLD_TOKEN"
```

Each device uses the same `user_id` and a different `device_id`. A missing device name uses the hostname.

The endpoint must contain only its origin. Embedded credentials, query parameters, and endpoint paths fail validation.

A private HTTP endpoint requires explicit `allow_http = true`. TLS or an encrypted private network must protect credentials in transit.

The adapter reads only the named credential variables. It does not discover AWS credentials through instance metadata or a shared profile.

Temporary S3 credentials can name an additional variable through `session_token_env`.

## Credential scope

Client S3 credentials need `s3:GetObject` and `s3:PutObject` for:

```text
<bucket>/<prefix>/users/<user>/blobs/*
```

Direct S3 metadata mode also needs those actions for:

```text
<bucket>/<prefix>/users/<user>/state.json
```

Drift does not create buckets. Bucket policy remains the authority for S3 access. The account prefix is not a substitute for that policy.

Celld needs its own fleet bucket and administrator credential. The client must not receive the fleet credential.

Celld's fleet bucket contains deployment, ownership, and recovery records. Direct writes to its reserved prefixes can break the fleet.

## Celld deployment

`celld/wrangler.jsonc` uses Celld's native configuration format. The checked-in credential fields are empty and deny every request.

The deployed worker needs these private runtime variables:

- `CELLD_VAR_DRIFT_USER`: the account name from the client configuration.
- `CELLD_VAR_DRIFT_TOKEN`: a high-entropy bearer token. The client supplies the same value through `DRIFT_CELLD_TOKEN`.

A protected environment file can provide these variables to the Celld service. The token is not an S3 administrator credential.

The worker supports one account per deployment. It authenticates requests before selecting the account's Durable Object.

Before deployment:

1. Create a dedicated fleet bucket and scoped fleet credentials.
2. Verify the bucket's conditional-write behavior with `celld diagnose`.
3. Keep `CELLD_STORAGE_PROBE=1` and `CELLD_OUTPUT_GATE=1`.
4. Configure private peer and operator listeners.
5. Deploy the project in `celld/` to the dedicated fleet.

Do not deploy this worker over the existing Site or counter fleet. One Celld fleet serves one application deployment.

The Onix reference is `../onix-core/modules/celld/`. Its module supplies private listener, credential, restart, and storage-provisioning conventions.

## Failure and retry behavior

The WAL stores each operation before remote replication. Blob intents bind their BLAKE3 identity durably before the first upload.

A changed or missing queued file stops replication. It does not publish an index or remove the pending operation.

The adapter uploads the blob before it publishes its metadata index. Downloads verify both size and BLAKE3 identity.

A failed index update can leave an unreferenced blob. This adapter does not run automatic blob garbage collection.

Metadata updates preserve unrelated concurrent changes through compare-and-swap. A definite conflict triggers a bounded read-and-retry sequence.

A timeout or lost response retains the WAL entry. The next attempt reads current metadata before it decides whether another write is necessary.

History keys and operation stamps remain stable across retries. A durable result that already contains or supersedes an operation permits acknowledgement without another metadata write.

The background task retries on its timer, including after startup. It stops at the first error and retains later entries in order.

Startup no longer expires or removes unconfirmed entries. The legacy `wal_max_age_days` field has no automatic pruning effect.

The WAL binds its account, device, and storage target. A different target fails explicitly instead of redirecting old operations.

Local updates do not wait for network I/O. A full WAL reports that local data exists but the new operation did not enter the replication queue.

Local data and the WAL remain separate stores. This change does not claim atomicity between their commits.

## Merge and size limits

- Queues use Lamport clock, timestamp, and device order.
- History uses stable play identities. The snapshot retains the newest 500 records.
- Playlist updates and deletion tombstones use timestamp, device, and operation order.
- Search documents use the same deterministic last-write order.
- Metadata snapshots cannot exceed 8 MiB.
- Individual blob uploads and downloads cannot exceed 512 MiB.

These are bounded snapshots, not an unbounded event log. An oversized snapshot fails and retains its WAL entry.

Clock skew can affect last-write selection for concurrent playlist edits. The design does not merge concurrent playlist track edits automatically.

## Migration limits

The old `aspen` feature and client dependencies are removed. S3 support is enabled by default. `--no-default-features` builds the local-only client.

Old Aspen tickets and peer subscriptions fail sync validation. They do not grant S3 or Celld access.

Existing local files remain unchanged. Pending legacy blob intents acquire a durable content identity before replay.

The migration does not scan an old Aspen cluster or upload the entire existing local library. New downloads enter the replication queue after sync is enabled.

Cross-account peer playlist discovery is not part of this adapter. S3 account policy and the Celld account token remain private by default.

A change between direct S3 metadata and Celld metadata needs an explicit state transfer. Merely changing the endpoint does not transfer state.

`drift-sync` queues completed downloads. Pending uploads survive its exit and resume when a configured player or downloader starts again.

## Verification

```sh
cargo test --all-targets --all-features
cargo test --all-targets --no-default-features
node --test celld/worker.test.mjs
nix flake check path:. -L
```

The live test uses disposable buckets, loopback listeners, and synthetic credentials. It never connects to an existing storage service.

The test requires explicit binary paths:

```sh
export DRIFT_TEST_RUSTFS_BIN=/absolute/path/to/rustfs
export DRIFT_TEST_CELLD_BIN=/absolute/path/to/celld
export DRIFT_TEST_MC_BIN=/absolute/path/to/mc
export DRIFT_TEST_ESBUILD_BIN=/absolute/path/to/esbuild
cargo test --test s3_rustfs_celld -- --ignored --nocapture
```

The checked live combination is RustFS `1.0.0-rc.2` and Celld `0.3.0`. It passed conditional-write rejection, concurrent metadata updates, blob retrieval, wrong-token denial, and cold recovery from the fleet bucket.

Unit tests also cover lost acknowledgements, denied uploads, changed files, corrupt metadata, WAL corruption, identity rebinding, and sequence overflow.

This evidence covers an isolated single-host fixture. It does not prove production credentials, multi-host availability, or a completed deployment.

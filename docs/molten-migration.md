# Molten migration status

Status: historical investigation, superseded by the S3/RustFS/Celld implementation.

The user replaced the Molten direction with S3 storage and optional Celld metadata. See [the current storage runbook](storage-s3.md).

The source checks and blockers below describe the earlier Molten investigation, not the current Drift backend.

## Source cohort

- Molten source: `../OnixResearch/aspen/`, commit `d327d045e98227945f78ce5a61d801986c717c54`.
- Published Molten parent: `ssh://git@github.com/brittonr/aspen.git`, commit `bb6f3830ee7327da9875ea85a8c8e25697eddc35`.
- The local commit adds a Wasm admission plan. It does not restore the Aspen client crates.
- Widget source: `../OnixResearch/rats/subwayrat/`, published commit `2e52b3150819a2365aaefd3dcf8bbd2a2fa2e901`.

Release dependencies must use published immutable revisions, not these sibling paths.

## Checked integration routes

### Direct client replacement

The Molten source does not contain `aspen-client` or `aspen-client-api`. A Cargo dependency update to the published Molten parent fails:

```text
error: no matching package named `aspen-client` found
```

A crate rename cannot migrate the existing RPC calls in `src/storage/aspen.rs` and `src/storage/peers.rs`.

### Native service client

Molten provides `NativeServiceClient` and `NativeServiceIngressPort` in `src/system_extension/native_host/service.rs`.

The implementation submits to a local `NativeSystemExtensionService`. The documented claim is `local-live-materialized-values-pilot`.

Molten's `docs/native-system-extension-host.md` explicitly describes the Iroh ingress adapter as future work. Its materialization section requires a deployment adapter for durable exact value publication and explicit acceptance uncertainty.

The checked source implements `NativeCallbackValuePort` with `InMemoryNativeCallbackValuePort`. That implementation is a conformance adapter, not a durable remote store.

### Generic fabric transport

Molten provides `RegisteredCrossProcessTransportEffectPort` for admitted frame delivery. The initial endpoint handoff profile supports an explicit loopback bind.

Molten's `docs/fabric-transport-session-runtime.md` excludes durable delivery, automatic retry safety, application authority, and application-level success from the transport contract.

A frame acknowledgement therefore cannot authorize removal of a Drift WAL entry. This route also needs a Drift service implementation and a durable application response.

## Required application boundary

Drift retains ownership of queue, history, playlist, search-cache, and blob semantics. Molten owns the admitted host and transport mechanisms.

The missing integration boundary must provide:

- A published remote ingress adapter with an admitted endpoint and service generation.
- A Drift service that implements the storage operations currently used by the replication loop.
- A durable value adapter and restart recovery for service state.
- Application authority that scopes each request to the permitted user and operation.
- Request identities and outcomes that distinguish rejection, durable completion, and uncertain completion.
- A reconciliation path for uncertain writes before retry or WAL removal.
- Remote reads and change observation that preserve Drift's merge behavior.

A cluster ticket, transport identity, or successful process exit does not provide these guarantees.

## Acceptance evidence

The migration needs two real service processes, not only a mock client. Evidence must cover accepted writes, remote reads, restart recovery, and local playback during remote failure.

Negative cases must include denied authority, wrong service generation, malformed responses, changed value identities, disconnect after submission, and lost acknowledgements.

A lost acknowledgement must not erase a WAL entry or cause an unqualified retry. A transport acknowledgement must not count as durable application completion.

The existing replication loop needs changes at this boundary. `drain_wal` removes entries on `Ok(())`, while `replicate_op` maps any successful blob result to `()`.

Those existing success conditions cannot substitute for Molten application outcomes.

## Validation and retained state

`cargo test --all-targets --locked` stops before compilation because `../aspen/crates/aspen-client/Cargo.toml` is absent.

The trial dependency pins also stopped before compilation. They were reverted. No test-pass or runtime-migration claim follows from these attempts.

The existing staged changes remain intact. This investigation changed no runtime code, Cargo dependency, lockfile, or generated build plan.

The next required deliverable is the published remote service boundary and its durable outcome contract. A fixture-only adapter is not an acceptable replacement.

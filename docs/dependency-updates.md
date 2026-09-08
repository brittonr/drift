# Dependency updates

## September 7, 2026

The first update refreshed compatible Cargo dependencies and four Nix inputs. Nix generated the lockfile. unit2nix generated the build plan.

The second update changed these direct dependency ranges:

| Dependency | Previous range | New range |
| --- | --- | --- |
| base64 | 0.21 | 0.23 |
| crossterm | 0.28 | 0.29 |
| dirs | 5 | 7 |
| lru | 0.12 | 0.18 |
| scraper | 0.20 | 0.27 |

Both Cargo feature matrices passed before and after the updates. The second update adds five tests for Base64 decoding, cache eviction, HTML extraction, and invalid inputs. Nix checks passed, including both MPD VMs.

These results do not prove live provider playback with the updated dependencies. The deployed system did not change.

## HTTP and S3 update

The third update uses reqwest 0.13 and object_store 0.14. Native TLS remains explicit for provider requests. Query and form features remain enabled. The S3 adapter imports the new extension trait without changing its application ports.

Three new request tests cover query encoding, token form encoding, and malformed URLs or authorization headers. Both Cargo feature matrices, the isolated RustFS/Celld test, and the x86_64-linux Nix checks passed. The fixture gives Celld a temporary directory inside its isolated root because the host `/tmp` quota blocked deployment.

## Remaining direct dependencies

The final update changes rand to 0.10, ratatui-image to 11, which to 8, TOML to 1.1, and normal storage to redb 4.

The image adapter uses the current protocol state and terminal-query API. It retains a half-block fallback without a new Chafa runtime dependency. Tests cover a colored render, an empty render area, TOML round trips, malformed configuration, executable lookup, and random-index bounds.

The offline `drift-db-upgrade` tool converts old database files into separate, validated copies. It preserves the original for rollback. See [the upgrade procedure](database-upgrade.md) before use with an existing installation.

Two intentional pins remain:

- `redb-legacy = 2.6.3` supplies the v2 migration bridge. Normal database access uses redb 4. The bridge never receives a current-format file.
- `rat-widgets` remains at `2e52b3150819a2365aaefd3dcf8bbd2a2fa2e901`. The remote default branch points to its parent, `0244e5d36feb65a472d3b76765c546c8c318c250`. That change is a downgrade, not an update.

The final Cargo dry run selects no further compatible updates. The only older registry versions are the migration bridge and generic-array 0.14.7. The latter is an exact requirement of crypto-common 0.1.7 through object_store. This update does not override upstream contracts.

Final verification passed both all-target Cargo feature matrices, the plugin tests, the Celld tests, isolated RustFS/Celld recovery, and x86_64-linux Nix checks. Both MPD VMs passed. Migration tests verified old-reader rollback and preserved WAL values, sequence metadata, and account context.

The git filesystem quota interrupted verification. The completed source is in `/home/brittonr/drift-dependencies` on the root filesystem. No unrelated files were removed. No production database or deployed package changed.

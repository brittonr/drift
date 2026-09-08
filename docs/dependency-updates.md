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

## Remaining version-range changes

The redb dependency remains on 2.x. The upstream redb 3.0 changelog removes support for file format v2. It requires `Database::upgrade()` from redb 2.6 to migrate those files. An update needs migration and rollback evidence for existing Drift databases.

Other major updates remain separate work: rand and ratatui-image. The published rat-widgets revision remains pinned. This update does not claim that every dependency uses its newest release.

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

## Remaining version-range changes

The redb dependency remains on 2.x. The upstream redb 3.0 changelog removes support for file format v2. It requires `Database::upgrade()` from redb 2.6 to migrate those files. An update needs migration and rollback evidence for existing Drift databases.

Other major updates remain separate work: reqwest, object_store, rand, and ratatui-image. The published rat-widgets revision remains pinned. This update does not claim that every dependency uses its newest release.

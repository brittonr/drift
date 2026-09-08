# Upgrade existing database files

Drift now uses redb 4. New databases use file format v3. Existing v2 files need an offline conversion before the updated application can open them.

`drift-db-upgrade SOURCE NEW_DESTINATION` converts one database. It never replaces or modifies the source file. It holds the source file lock during the copy. It rejects active database users, missing sources, corrupt files, and existing destinations.

The tool validates a private temporary copy before publication. New files use owner-only permissions on Unix. The tool publishes the destination without replacement and syncs it. It also syncs the parent directory on Unix.

The exact redb 2.6.3 dependency remains solely as the v2 migration bridge. Normal Drift storage uses redb 4. Current-format files never enter the legacy reader.

## Procedure

CAUTION: Stop every application that uses these databases before conversion or file replacement. Concurrent writers can make the retained original and upgraded copy contain different data.

1. Stop Drift, drift-sync, tidal-db, and external download tools that use the same files.
2. Make an offline backup of all Drift database files.
3. Locate the `.redb` files in the configured data, cache, and music directories.
4. For each file, choose a destination that does not exist.
5. Run the upgrade command for each file.

For example:

```sh
drift-db-upgrade history.redb history.upgraded.redb
```

The Nix application also exposes the tool:

```sh
nix run .#drift-db-upgrade -- history.redb history.upgraded.redb
```

6. Keep the applications stopped until every conversion succeeds.
7. Rename each original file to a retained backup name.
8. Rename each upgraded copy to the original configured path.
9. Start the updated application.

The files include history, download records, playlists, metadata cache, and the pending-operation WAL. External tidal-dl databases also need compatible readers. Search-cache and queue TOML files do not need this database conversion.

## Failure and rollback

A failure before publication leaves the destination absent and the source unchanged. A directory-sync failure can leave a valid published destination. The error identifies that state. Existing destinations always block retries rather than permit replacement.

The tool does not automatically replace files or migrate a whole directory. A crash can leave a private temporary file, but it does not change the original.

Before a rollback, stop all database users. Keep the upgraded files separately. Restore the retained original files and the old application together. The original files do not contain writes made after the upgrade.

## Evidence boundary

Synthetic tests cover v2 conversion, current-format copying, old-reader rollback, Unicode records, pending-operation values, permissions, locked sources, corrupt sources, and existing destinations. No production database was opened or converted during dependency verification. Cross-platform durability is not claimed beyond the tested Linux filesystem behavior.

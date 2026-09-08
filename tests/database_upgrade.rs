use drift::database_upgrade::upgrade_copy;
use redb::{ReadableDatabase, ReadableTableMetadata};

const LEGACY: redb_legacy::TableDefinition<&str, &str> =
    redb_legacy::TableDefinition::new("records");
const CURRENT: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("records");

fn old_database(path: &std::path::Path) {
    let db = redb_legacy::Database::create(path).unwrap();
    let transaction = db.begin_write().unwrap();
    {
        let mut table = transaction.open_table(LEGACY).unwrap();
        table.insert("track", "音楽").unwrap();
        table
            .insert("pending-operation", "retain-on-failure")
            .unwrap();
    }
    transaction.commit().unwrap();
}

#[test]
fn old_format_migrates_without_changing_the_rollback_copy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("old.redb");
    let destination = root.path().join("new.redb");
    old_database(&source);
    let original = std::fs::read(&source).unwrap();
    assert!(matches!(
        redb::Database::open(&source),
        Err(redb::DatabaseError::UpgradeRequired(_))
    ));
    upgrade_copy(&source, &destination).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), original);
    let db = redb::Database::open(&destination).unwrap();
    let transaction = db.begin_read().unwrap();
    let table = transaction.open_table(CURRENT).unwrap();
    const EXPECTED_RECORD_COUNT: u64 = 2;
    assert_eq!(table.len().unwrap(), EXPECTED_RECORD_COUNT);
    assert_eq!(table.get("track").unwrap().unwrap().value(), "音楽");
    assert_eq!(
        table.get("pending-operation").unwrap().unwrap().value(),
        "retain-on-failure"
    );
    let old = redb_legacy::Database::open(&source).unwrap();
    assert_eq!(
        old.begin_read()
            .unwrap()
            .open_table(LEGACY)
            .unwrap()
            .get("track")
            .unwrap()
            .unwrap()
            .value(),
        "音楽"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        const PRIVATE_MODE: u32 = 0o600;
        const PERMISSION_BITS: u32 = 0o777;
        assert_eq!(
            std::fs::metadata(destination).unwrap().permissions().mode() & PERMISSION_BITS,
            PRIVATE_MODE
        );
    }
}

#[test]
fn refuses_existing_destination_and_source_alias() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("old.redb");
    let destination = root.path().join("existing");
    old_database(&source);
    std::fs::write(&destination, b"do not replace").unwrap();
    assert!(upgrade_copy(&source, &destination).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"do not replace");
    assert!(upgrade_copy(&source, &source).is_err());
}

#[test]
fn refuses_corrupt_empty_missing_and_locked_sources() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("output");
    assert!(upgrade_copy(&source, &destination).is_err());
    for contents in [b"".as_slice(), b"not a database"] {
        std::fs::write(&source, contents).unwrap();
        assert!(upgrade_copy(&source, &destination).is_err());
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&source).unwrap(), contents);
    }
    let locked = root.path().join("locked.redb");
    old_database(&locked);
    let _db = redb_legacy::Database::open(&locked).unwrap();
    assert!(upgrade_copy(&locked, &destination).is_err());
    assert!(!destination.exists());
}

#[test]
fn current_format_copy_is_valid_and_missing_parent_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.redb");
    drop(redb::Database::create(&source).unwrap());
    assert!(upgrade_copy(&source, &root.path().join("missing/output")).is_err());
    let destination = root.path().join("copy.redb");
    upgrade_copy(&source, &destination).unwrap();
    assert!(redb::Database::open(destination).is_ok());
}

#[test]
fn cli_upgrades_a_copy_and_rejects_missing_arguments() {
    let binary = env!("CARGO_BIN_EXE_drift-db-upgrade");
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.redb");
    let destination = root.path().join("output.redb");
    old_database(&source);
    let original = std::fs::read(&source).unwrap();
    let success = std::process::Command::new(binary)
        .arg(&source)
        .arg(&destination)
        .output()
        .unwrap();
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert!(redb::Database::open(destination).is_ok());
    let failure = std::process::Command::new(binary).output().unwrap();
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("usage:"));
}

//! Verify the key/value encodings used by the persistent WAL across file formats.
use drift::database_upgrade::upgrade_copy;
use drift::storage::wal::{ReplicationOp, WalEntry};
use redb::ReadableDatabase;

const ENTRY_SEQUENCE: u64 = 7;
const NEXT_SEQUENCE: u64 = ENTRY_SEQUENCE + 1;
const CREATED_AT: u64 = 123;
const OLD_ENTRIES: redb_legacy::TableDefinition<u64, &[u8]> =
    redb_legacy::TableDefinition::new("wal_entries");
const OLD_METADATA: redb_legacy::TableDefinition<&str, u64> =
    redb_legacy::TableDefinition::new("wal_metadata");
const OLD_CONTEXT: redb_legacy::TableDefinition<&str, &str> =
    redb_legacy::TableDefinition::new("wal_context");
const NEW_ENTRIES: redb::TableDefinition<u64, &[u8]> = redb::TableDefinition::new("wal_entries");
const NEW_METADATA: redb::TableDefinition<&str, u64> = redb::TableDefinition::new("wal_metadata");
const NEW_CONTEXT: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("wal_context");

#[test]
fn migration_preserves_pending_operations_sequences_and_account_context() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("wal.redb");
    let output = root.path().join("wal.upgraded.redb");
    let entry = WalEntry {
        op: ReplicationOp::DeletePlaylist {
            playlist_id: "retained-pending-delete".into(),
        },
        created_at_ms: CREATED_AT,
        attempts: 1,
    };
    let encoded = serde_json::to_vec(&entry).unwrap();
    {
        let db = redb_legacy::Database::create(&source).unwrap();
        let transaction = db.begin_write().unwrap();
        transaction
            .open_table(OLD_ENTRIES)
            .unwrap()
            .insert(ENTRY_SEQUENCE, encoded.as_slice())
            .unwrap();
        transaction
            .open_table(OLD_METADATA)
            .unwrap()
            .insert("next_sequence", NEXT_SEQUENCE)
            .unwrap();
        transaction
            .open_table(OLD_CONTEXT)
            .unwrap()
            .insert("replication_target", "synthetic-account")
            .unwrap();
        transaction.commit().unwrap();
    }
    upgrade_copy(&source, &output).unwrap();
    let db = redb::Database::open(&output).unwrap();
    let transaction = db.begin_read().unwrap();
    let table = transaction.open_table(NEW_ENTRIES).unwrap();
    let bytes = table.get(ENTRY_SEQUENCE).unwrap().unwrap();
    assert_eq!(bytes.value(), encoded);
    let recovered: WalEntry = serde_json::from_slice(bytes.value()).unwrap();
    assert_eq!(recovered.created_at_ms, CREATED_AT);
    assert_eq!(recovered.attempts, 1);
    assert!(
        matches!(recovered.op, ReplicationOp::DeletePlaylist { playlist_id } if playlist_id == "retained-pending-delete")
    );
    assert!(table.get(NEXT_SEQUENCE).unwrap().is_none());
    assert_eq!(
        transaction
            .open_table(NEW_METADATA)
            .unwrap()
            .get("next_sequence")
            .unwrap()
            .unwrap()
            .value(),
        NEXT_SEQUENCE
    );
    assert_eq!(
        transaction
            .open_table(NEW_CONTEXT)
            .unwrap()
            .get("replication_target")
            .unwrap()
            .unwrap()
            .value(),
        "synthetic-account"
    );
}

#[test]
fn truncated_database_never_publishes_a_destination() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("truncated.redb");
    let destination = root.path().join("output.redb");
    drop(redb_legacy::Database::create(&source).unwrap());
    std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_len(1)
        .unwrap();
    let before = std::fs::read(&source).unwrap();
    assert!(upgrade_copy(&source, &destination).is_err());
    assert!(!destination.exists());
    assert_eq!(std::fs::read(&source).unwrap(), before);
}

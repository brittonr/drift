use super::*;

fn operation() -> ReplicationOp {
    ReplicationOp::DeletePlaylist {
        playlist_id: "mix".into(),
    }
}

#[test]
fn sequence_survives_an_empty_log_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal.redb");
    let db = Database::create(&path).unwrap();
    WalManager::init_table(&db).unwrap();
    let wal = WalManager {
        db,
        next_seq: Mutex::new(1),
    };
    let first = wal.append(&operation()).unwrap();
    wal.remove(first).unwrap();
    drop(wal);
    let wal = WalManager {
        db: Database::create(path).unwrap(),
        next_seq: Mutex::new(1),
    };
    let second = wal.append(&operation()).unwrap();
    assert!(second > first);
}

#[test]
fn rejects_corrupt_entries_without_skipping_them() {
    let wal = WalManager::new_in_memory().unwrap();
    let txn = wal.db.begin_write().unwrap();
    {
        txn.open_table(WAL_TABLE)
            .unwrap()
            .insert(1, b"broken".as_slice())
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(wal.drain_pending().is_err());
    assert_eq!(wal.len().unwrap(), 1);
}

#[test]
fn context_binding_accepts_reopen_but_rejects_account_rebinding() {
    let wal = WalManager::new_in_memory().unwrap();
    wal.bind_context("account-a-device-a").unwrap();
    wal.bind_context("account-a-device-a").unwrap();
    assert!(wal.bind_context("account-b-device-a").is_err());
    assert!(wal.bind_context("account-a-device-b").is_err());
    wal.bind_context("account-a-device-a").unwrap();
}

#[test]
fn overflow_and_absent_content_binding_fail_without_overwrite() {
    let wal = WalManager::new_in_memory().unwrap();
    let txn = wal.db.begin_write().unwrap();
    {
        txn.open_table(WAL_META_TABLE)
            .unwrap()
            .insert(NEXT_SEQUENCE_KEY, u64::MAX)
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(wal.append(&operation()).is_err());
    assert!(wal.is_empty().unwrap());
    let entry = WalEntry {
        op: operation(),
        created_at_ms: 1,
        attempts: 0,
    };
    assert!(wal.update_entry(1, &entry).is_err());
}

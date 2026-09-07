use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use super::*;
use crate::storage::object_port::MetadataRead;
use crate::storage::wal::WalManager;
use async_trait::async_trait;

const ENTRY_TIME: u64 = 100;

#[derive(Default)]
struct Metadata {
    bytes: Mutex<Option<Vec<u8>>>,
    commits: AtomicUsize,
    lose_ack: AtomicBool,
    conflict_once: AtomicBool,
    deny: AtomicBool,
}

#[async_trait]
impl MetadataPort for Metadata {
    async fn load(&self) -> Result<MetadataRead> {
        let bytes = self.bytes.lock().unwrap().clone();
        let revision = bytes
            .as_ref()
            .map(|bytes| blake3::hash(bytes).to_hex().to_string());
        Ok(MetadataRead { bytes, revision })
    }
    async fn compare_and_swap(&self, expected: Option<&str>, bytes: Vec<u8>) -> Result<bool> {
        ensure!(!self.deny.load(Ordering::SeqCst), "denied");
        if self.conflict_once.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        let mut stored = self.bytes.lock().unwrap();
        let revision = stored
            .as_ref()
            .map(|bytes| blake3::hash(bytes).to_hex().to_string());
        if revision.as_deref() != expected {
            return Ok(false);
        }
        *stored = Some(bytes);
        self.commits.fetch_add(1, Ordering::SeqCst);
        ensure!(
            !self.lose_ack.swap(false, Ordering::SeqCst),
            "acknowledgement lost after commit"
        );
        Ok(true)
    }
}

#[derive(Default)]
struct Blobs {
    values: Mutex<BTreeMap<String, Vec<u8>>>,
    fail: AtomicBool,
}

#[async_trait]
impl BlobPort for Blobs {
    async fn put(&self, hash: &str, bytes: Vec<u8>) -> Result<()> {
        ensure!(!self.fail.load(Ordering::SeqCst), "blob upload denied");
        self.values.lock().unwrap().insert(hash.into(), bytes);
        Ok(())
    }
    async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.values.lock().unwrap().get(hash).cloned())
    }
}

fn storage() -> (S3Storage, Arc<Metadata>, Arc<Blobs>) {
    let metadata = Arc::new(Metadata::default());
    let blobs = Arc::new(Blobs::default());
    (
        S3Storage {
            metadata: metadata.clone(),
            blobs: blobs.clone(),
            user: "alice".into(),
        },
        metadata,
        blobs,
    )
}

fn entry() -> WalEntry {
    WalEntry {
        op: ReplicationOp::DeletePlaylist {
            playlist_id: "mix".into(),
        },
        created_at_ms: ENTRY_TIME,
        attempts: 0,
    }
}

#[tokio::test]
async fn commits_once_and_reconciles_lost_ack_after_restart() {
    let (remote, metadata, blobs) = storage();
    let wal = WalManager::new_in_memory().unwrap();
    wal.append(&entry().op).unwrap();
    metadata.lose_ack.store(true, Ordering::SeqCst);
    assert!(
        crate::storage::replication_task::drain(&wal, &remote, "phone")
            .await
            .is_err()
    );
    assert_eq!(wal.len().unwrap(), 1);
    let restarted = S3Storage {
        metadata: metadata.clone(),
        blobs,
        user: "alice".into(),
    };
    crate::storage::replication_task::drain(&wal, &restarted, "phone")
        .await
        .unwrap();
    assert_eq!(wal.len().unwrap(), 0);
    assert_eq!(metadata.commits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn conflict_reloads_but_denial_preserves_wal_order() {
    let (remote, metadata, _) = storage();
    metadata.conflict_once.store(true, Ordering::SeqCst);
    remote.replicate("phone", 1, &entry()).await.unwrap();
    let wal = WalManager::new_in_memory().unwrap();
    wal.append(&ReplicationOp::DeletePlaylist {
        playlist_id: "first".into(),
    })
    .unwrap();
    wal.append(&ReplicationOp::DeletePlaylist {
        playlist_id: "second".into(),
    })
    .unwrap();
    let pending = wal.len().unwrap();
    metadata.deny.store(true, Ordering::SeqCst);
    assert!(
        crate::storage::replication_task::drain(&wal, &remote, "phone")
            .await
            .is_err()
    );
    assert_eq!(wal.len().unwrap(), pending);
    assert!(!remote
        .snapshot()
        .await
        .unwrap()
        .documents
        .contains_key("playlists/second"));
}

#[tokio::test]
async fn failed_blob_upload_cannot_publish_an_index_or_drop_the_wal_entry() {
    let (remote, metadata, blobs) = storage();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("audio.flac");
    tokio::fs::write(&path, b"audio").await.unwrap();
    let wal = WalManager::new_in_memory().unwrap();
    wal.append(&ReplicationOp::UploadBlob {
        track_id: "song".into(),
        file_path: path.to_str().unwrap().into(),
        expected_hash: None,
    })
    .unwrap();
    blobs.fail.store(true, Ordering::SeqCst);
    assert!(
        crate::storage::replication_task::drain(&wal, &remote, "phone")
            .await
            .is_err()
    );
    assert_eq!(metadata.commits.load(Ordering::SeqCst), 0);
    assert_eq!(wal.len().unwrap(), 1);
    let bound = wal.drain_pending().unwrap();
    assert!(matches!(
        &bound[0].1.op,
        ReplicationOp::UploadBlob {
            expected_hash: Some(_),
            ..
        }
    ));
    blobs.fail.store(false, Ordering::SeqCst);
    crate::storage::replication_task::drain(&wal, &remote, "phone")
        .await
        .unwrap();
    assert_eq!(remote.fetch_blob("song").await.unwrap().unwrap(), b"audio");
    assert_eq!(wal.len().unwrap(), 0);
    let index = remote.has_blob("song").await.unwrap().unwrap();
    blobs
        .values
        .lock()
        .unwrap()
        .insert(index.hash, b"wrong".to_vec());
    assert!(remote.fetch_blob("song").await.is_err());
}

#[tokio::test]
async fn changed_or_missing_upload_file_is_not_acknowledged() {
    let (remote, metadata, _) = storage();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("audio.flac");
    tokio::fs::write(&path, b"original").await.unwrap();
    let mut entry = WalEntry {
        op: ReplicationOp::UploadBlob {
            track_id: "song".into(),
            file_path: path.to_str().unwrap().into(),
            expected_hash: None,
        },
        ..entry()
    };
    assert!(bind_blob(&mut entry).await.unwrap());
    tokio::fs::write(&path, b"replacement").await.unwrap();
    assert!(remote.replicate("phone", 1, &entry).await.is_err());
    tokio::fs::remove_file(&path).await.unwrap();
    assert!(remote.replicate("phone", 1, &entry).await.is_err());
    assert_eq!(metadata.commits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn startup_timer_drains_existing_entries_without_a_new_write() {
    use crate::storage::local_first::LocalFirstStorage;
    const DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);
    const POLL: std::time::Duration = std::time::Duration::from_millis(10);
    const CACHE_TTL: u64 = 3600;
    let (remote, _, _) = storage();
    let local = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    local.wal.append(&entry().op).unwrap();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let handle =
        crate::storage::replication_task::spawn(&local, Arc::new(remote), receiver, events);
    let result = tokio::time::timeout(DEADLINE, async {
        while !local.wal.is_empty().unwrap() {
            tokio::time::sleep(POLL).await;
        }
    })
    .await;
    drop(sender);
    handle.abort();
    assert!(result.is_ok(), "startup did not drain the existing intent");
}

#[tokio::test]
async fn superseded_write_refreshes_local_state_even_when_remote_bytes_do_not_change() {
    use crate::storage::local_first::{LocalFirstStorage, ReplicationMsg};
    use crate::storage::{DriftStorage, PlaylistVisibility, SyncEvent, SyncedPlaylist};
    const FUTURE_TIME: u64 = 4_102_444_800_000;
    const DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);
    const CACHE_TTL: u64 = 3600;
    let (remote, _, _) = storage();
    let tombstone = WalEntry {
        created_at_ms: FUTURE_TIME,
        ..entry()
    };
    remote
        .replicate("other-device", 1, &tombstone)
        .await
        .unwrap();
    let local = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let (events, mut observed) = tokio::sync::mpsc::unbounded_channel();
    let handle =
        crate::storage::replication_task::spawn(&local, Arc::new(remote), receiver, events);
    let result = tokio::time::timeout(DEADLINE, async {
        assert!(matches!(
            observed.recv().await,
            Some(SyncEvent::PlaylistDeleted { .. })
        ));
        local
            .save_playlist(&SyncedPlaylist {
                id: "mix".into(),
                title: "losing update".into(),
                description: None,
                tracks: Vec::new(),
                created_at_ms: ENTRY_TIME,
                updated_at_ms: ENTRY_TIME,
                lamport_clock: 0,
                device_id: String::new(),
                visibility: PlaylistVisibility::Private,
            })
            .await
            .unwrap();
        assert!(local.load_playlist("mix").await.unwrap().is_some());
        sender.send(ReplicationMsg::Drain).await.unwrap();
        assert!(matches!(
            observed.recv().await,
            Some(SyncEvent::PlaylistDeleted { .. })
        ));
        assert!(local.load_playlist("mix").await.unwrap().is_none());
    })
    .await;
    handle.abort();
    assert!(result.is_ok(), "superseded local value did not converge");
}

#[tokio::test]
async fn malformed_remote_state_never_becomes_an_empty_account() {
    let (remote, metadata, _) = storage();
    *metadata.bytes.lock().unwrap() = Some(b"malformed".to_vec());
    assert!(remote.snapshot().await.is_err());
    assert!(remote.replicate("phone", 1, &entry()).await.is_err());
    assert_eq!(metadata.commits.load(Ordering::SeqCst), 0);
}

use super::*;
use crate::service::{CoverArt, ServiceType};

const CACHE_TTL: u64 = 3600;
const TRACK_DURATION: u32 = 180;
const WRITE_COUNT: u64 = 3;
const TWO_ENTRIES: usize = 2;

fn track() -> Track {
    Track {
        id: "song".into(),
        title: "Song".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        duration_seconds: TRACK_DURATION,
        cover_art: CoverArt::None,
        service: ServiceType::Tidal,
    }
}

#[tokio::test]
async fn record_play_reads_from_local_and_queues() {
    let storage = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    storage.record_play(&track()).await.unwrap();
    let history = storage.get_history(1).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].track_id, track().id);
    assert_eq!(storage.pending_wal_count(), 1);
}

#[tokio::test]
async fn queue_stamps_device_and_advances_clock() {
    let storage = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    let queue = PersistedQueue::from_tracks(&[track()], None, None);
    for _ in 0..WRITE_COUNT {
        storage.save_queue(&queue).await.unwrap();
    }
    let saved = storage.load_queue().await.unwrap().unwrap();
    assert_eq!(saved.device_id, "test-device");
    assert_eq!(saved.lamport_clock, WRITE_COUNT);
    assert!(saved.updated_at_ms > 0);
}

#[tokio::test]
async fn writes_append_independent_wal_entries() {
    let storage = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    storage.record_play(&track()).await.unwrap();
    storage.save_queue(&PersistedQueue::new()).await.unwrap();
    assert_eq!(storage.pending_wal_count(), TWO_ENTRIES);
}

#[tokio::test]
async fn local_search_roundtrip_without_network() {
    let storage = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    let results = SearchResults {
        tracks: vec![track()],
        ..SearchResults::default()
    };
    storage.cache_search("query", None, &results).await.unwrap();
    assert_eq!(
        storage
            .get_cached_search("query", None)
            .await
            .unwrap()
            .unwrap()
            .tracks
            .len(),
        1
    );
    assert!(storage
        .get_cached_search("missing", None)
        .await
        .unwrap()
        .is_none());
    assert_eq!(storage.backend_name(), "local-first");
    assert!(storage.poll_changes().await.unwrap().is_empty());
}

#[tokio::test]
async fn event_channel_drains_and_handles_disconnection() {
    let mut storage = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    storage.sync_events_rx = std::sync::Mutex::new(rx);
    tx.send(SyncEvent::QueueChanged(PersistedQueue::new()))
        .unwrap();
    tx.send(SyncEvent::PlaylistDeleted {
        playlist_id: "mix".into(),
    })
    .unwrap();
    let events = storage.poll_changes().await.unwrap();
    assert_eq!(events.len(), TWO_ENTRIES);
    assert!(matches!(events[0], SyncEvent::QueueChanged(_)));
    assert!(storage.poll_changes().await.unwrap().is_empty());
    drop(tx);
    assert!(storage.poll_changes().await.unwrap().is_empty());
}

#[tokio::test]
async fn full_wal_rejects_new_intent_without_discarding_pending_data() {
    let mut storage = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    storage.max_wal_entries = 1;
    storage.record_play(&track()).await.unwrap();
    assert!(storage.save_queue(&PersistedQueue::new()).await.is_err());
    assert_eq!(storage.pending_wal_count(), 1);
    assert!(storage.load_queue().await.unwrap().is_some());
}

#[tokio::test]
async fn disabled_sync_does_not_fill_the_wal() {
    let mut storage = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    storage.queue_enabled = false;
    storage.max_wal_entries = 0;
    storage.record_play(&track()).await.unwrap();
    storage.save_queue(&PersistedQueue::new()).await.unwrap();
    assert_eq!(storage.pending_wal_count(), 0);
    assert_eq!(storage.get_history(1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn clock_overflow_rejects_without_mutation() {
    let storage = LocalFirstStorage::new_for_test(CACHE_TTL).unwrap();
    storage.lamport_clock.store(u64::MAX, Ordering::SeqCst);
    assert!(storage.save_queue(&PersistedQueue::new()).await.is_err());
    assert!(storage.load_queue().await.unwrap().is_none());
    assert_eq!(storage.pending_wal_count(), 0);
}

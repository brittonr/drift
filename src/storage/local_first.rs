//! Local-first storage backend with optional remote replication.
//!
//! All reads come from local storage (redb/TOML/JSON) — never blocks on network.
//! All writes go to local first, then queue for async replication to Aspen.
//! Remote changes are polled and merged into local state using CRDT semantics.
//!
//! ```text
//! App reads/writes ──► LocalFirstStorage
//!                      ├─ LocalStorage (redb)  ◄── all reads, all writes
//!                      ├─ WalManager (redb)    ◄── pending remote ops
//!                      └─ AspenStorage (opt)   ◄── background replication
//! ```
//!
//! The replication task runs in the background when the `aspen` feature is
//! enabled and `sync_enabled = true` in config. It does two things:
//!
//! 1. **WAL drain**: reads pending ops from the WAL and replays them on Aspen.
//!    Stops on first failure to preserve ordering; WAL pruning cleans up old
//!    entries that can never succeed.
//!
//! 2. **Remote poll**: checks Aspen for changes from other devices every 5s.
//!    Merges remote state into local using CRDT semantics (Lamport for queue,
//!    set-union for history) and emits `SyncEvent`s through a channel that
//!    `poll_changes()` drains.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing;

use super::local::LocalStorage;
#[cfg(feature = "aspen")]
use super::merge::{self, QueueMergeResult};
use super::wal::{ReplicationOp, WalManager};
use super::{BlobRef, DriftStorage, PlaylistIndexEntry, SyncEvent, SyncedPlaylist};
use crate::config::StorageConfig;
use crate::history_db::HistoryEntry;
use crate::queue_persistence::PersistedQueue;
use crate::search::SearchHistory;
use crate::service::{SearchResults, ServiceType, Track};

/// Channel message for the background replication task.
enum ReplicationMsg {
    /// New operation appended to WAL — try to drain.
    Drain,
    /// Shut down the replication task.
    Shutdown,
}

pub struct LocalFirstStorage {
    local: Arc<LocalStorage>,
    wal: Arc<WalManager>,
    device_id: String,
    lamport_clock: AtomicU64,
    /// Notify the replication task that new ops are available.
    replication_tx: mpsc::UnboundedSender<ReplicationMsg>,
    /// Handle to the background replication task.
    _replication_handle: Option<tokio::task::JoinHandle<()>>,
    /// Receive SyncEvents from the replication task.
    sync_events_rx: std::sync::Mutex<mpsc::UnboundedReceiver<SyncEvent>>,
}

impl LocalFirstStorage {
    /// Create a new local-first storage backend.
    ///
    /// If `sync_config` indicates sync is desired and the aspen feature is enabled,
    /// spawns a background replication task that drains the WAL to Aspen and
    /// polls for remote changes. Falls back gracefully to local-only if
    /// connection fails.
    pub async fn new(config: &StorageConfig, cache_ttl_seconds: u64) -> Result<Self> {
        let local = Arc::new(LocalStorage::new(cache_ttl_seconds)?);
        let wal = Arc::new(WalManager::new()?);

        // Prune expired WAL entries on startup
        let max_age = Duration::from_secs(config.wal_max_age_days as u64 * 86400);
        if let Ok(pruned) = wal.prune_expired(max_age) {
            if pruned > 0 {
                tracing::info!("Pruned {} expired WAL entries", pruned);
            }
        }
        if let Ok(dropped) = wal.enforce_max_entries(config.wal_max_entries) {
            if dropped > 0 {
                tracing::info!("Dropped {} excess WAL entries", dropped);
            }
        }

        let device_id = config.resolved_user_id();

        // Load Lamport clock from last saved queue
        let initial_lamport = match local.load_queue().await {
            Ok(Some(q)) => q.lamport_clock,
            _ => 0,
        };

        let (replication_tx, replication_rx) = mpsc::unbounded_channel();
        let (sync_tx, sync_rx) = mpsc::unbounded_channel();

        // Spawn background replication if sync is enabled
        let replication_handle = if config.wants_sync() {
            #[cfg(feature = "aspen")]
            {
                spawn_replication_task(
                    config,
                    Arc::clone(&wal),
                    Arc::clone(&local),
                    &device_id,
                    replication_rx,
                    sync_tx,
                )
                .await
            }
            #[cfg(not(feature = "aspen"))]
            {
                tracing::info!("Sync requested but 'aspen' feature not enabled — local only");
                drop(replication_rx);
                drop(sync_tx);
                None
            }
        } else {
            drop(replication_rx);
            drop(sync_tx);
            None
        };

        Ok(Self {
            local,
            wal,
            device_id,
            lamport_clock: AtomicU64::new(initial_lamport),
            replication_tx,
            _replication_handle: replication_handle,
            sync_events_rx: std::sync::Mutex::new(sync_rx),
        })
    }

    /// Create a local-first storage for tests (no remote, in-memory).
    pub fn new_for_test(cache_ttl_seconds: u64) -> Result<Self> {
        let local = Arc::new(LocalStorage::new_for_test(cache_ttl_seconds)?);
        let wal = Arc::new(WalManager::new_in_memory()?);
        let (replication_tx, _rx) = mpsc::unbounded_channel();
        let (_sync_tx, sync_rx) = mpsc::unbounded_channel();

        Ok(Self {
            local,
            wal,
            device_id: "test-device".to_string(),
            lamport_clock: AtomicU64::new(0),
            replication_tx,
            _replication_handle: None,
            sync_events_rx: std::sync::Mutex::new(sync_rx),
        })
    }

    /// Queue a replication operation: write to WAL then notify the drain task.
    fn queue_replication(&self, op: ReplicationOp) {
        if let Err(e) = self.wal.append(&op) {
            tracing::warn!("Failed to append to WAL: {}", e);
            return;
        }
        // Best-effort notify — if the channel is closed, we still have the WAL.
        let _ = self.replication_tx.send(ReplicationMsg::Drain);
    }

    /// Increment and return the next Lamport clock value.
    fn next_lamport(&self) -> u64 {
        self.lamport_clock.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Update local Lamport clock to be at least as large as a remote value.
    #[allow(dead_code)]
    fn observe_lamport(&self, remote: u64) {
        self.lamport_clock.fetch_max(remote, Ordering::SeqCst);
    }

    /// Number of pending WAL entries (for diagnostics).
    pub fn pending_wal_count(&self) -> usize {
        self.wal.len().unwrap_or(0)
    }
}

#[async_trait]
impl DriftStorage for LocalFirstStorage {
    fn backend_name(&self) -> &str {
        "local-first"
    }

    // ── History ──────────────────────────────────────────────────────────

    async fn record_play(&self, track: &Track) -> Result<()> {
        // Write to local first — always succeeds
        self.local.record_play(track).await?;
        // Queue for remote replication
        self.queue_replication(ReplicationOp::RecordPlay(track.clone()));
        Ok(())
    }

    async fn get_history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        // Always read from local — never blocks on network
        self.local.get_history(limit).await
    }

    // ── Queue ────────────────────────────────────────────────────────────

    async fn save_queue(&self, queue: &PersistedQueue) -> Result<()> {
        // Stamp with our device ID and Lamport clock
        let mut stamped = queue.clone();
        stamped.device_id = self.device_id.clone();
        stamped.lamport_clock = self.next_lamport();
        stamped.updated_at_ms = chrono::Utc::now().timestamp_millis() as u64;

        // Write to local first
        self.local.save_queue(&stamped).await?;

        // Queue for remote replication
        self.queue_replication(ReplicationOp::SaveQueue(stamped));
        Ok(())
    }

    async fn load_queue(&self) -> Result<Option<PersistedQueue>> {
        // Always read from local
        self.local.load_queue().await
    }

    // ── Search Cache ────────────────────────────────────────────────────

    async fn cache_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
        results: &SearchResults,
    ) -> Result<()> {
        // Write to local first
        self.local
            .cache_search(query, service_filter, results)
            .await?;
        // Queue for remote replication
        self.queue_replication(ReplicationOp::CacheSearch {
            query: query.to_string(),
            service_filter,
            results: results.clone(),
        });
        Ok(())
    }

    async fn get_cached_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
    ) -> Result<Option<SearchResults>> {
        // Always read from local
        self.local.get_cached_search(query, service_filter).await
    }

    // ── Search History ──────────────────────────────────────────────────

    async fn save_search_history(&self, history: &SearchHistory) -> Result<()> {
        self.local.save_search_history(history).await?;
        self.queue_replication(ReplicationOp::SaveSearchHistory(history.clone()));
        Ok(())
    }

    async fn load_search_history(&self, max_size: usize) -> Result<SearchHistory> {
        self.local.load_search_history(max_size).await
    }

    // ── Playlists ─────────────────────────────────────────────────────────

    async fn save_playlist(&self, playlist: &SyncedPlaylist) -> Result<()> {
        // Stamp with our device ID and Lamport clock
        let mut stamped = playlist.clone();
        stamped.device_id = self.device_id.clone();
        stamped.lamport_clock = self.next_lamport();
        stamped.updated_at_ms = chrono::Utc::now().timestamp_millis() as u64;

        // Write to local first
        self.local.save_playlist(&stamped).await?;

        // Queue for remote replication
        self.queue_replication(ReplicationOp::SavePlaylist(stamped));
        Ok(())
    }

    async fn load_playlist(&self, playlist_id: &str) -> Result<Option<SyncedPlaylist>> {
        // Always read from local
        self.local.load_playlist(playlist_id).await
    }

    async fn list_playlists(&self) -> Result<Vec<PlaylistIndexEntry>> {
        // Always read from local
        self.local.list_playlists().await
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        // Delete from local first
        self.local.delete_playlist(playlist_id).await?;

        // Queue for remote replication
        self.queue_replication(ReplicationOp::DeletePlaylist {
            playlist_id: playlist_id.to_string(),
        });
        Ok(())
    }

    // ── Blob Storage ────────────────────────────────────────────────────

    async fn upload_blob(&self, track_id: &str, file_path: &str) -> Result<Option<String>> {
        // Queue for remote upload — blobs are too large for the WAL,
        // so we just record the intent and let the replication task handle it.
        self.queue_replication(ReplicationOp::UploadBlob {
            track_id: track_id.to_string(),
            file_path: file_path.to_string(),
        });
        Ok(None) // Actual upload happens async; return None for now
    }

    async fn has_blob(&self, _track_id: &str) -> Result<Option<BlobRef>> {
        // Local-first doesn't check remote blob store inline —
        // blob data arrives via the download manager's blob fetch path.
        Ok(None)
    }

    async fn fetch_blob(&self, _track_id: &str) -> Result<Option<Vec<u8>>> {
        // Blob fetches are handled by the download manager, not storage.
        Ok(None)
    }

    // ── Sync ────────────────────────────────────────────────────────────

    async fn poll_changes(&self) -> Result<Vec<SyncEvent>> {
        // Drain any SyncEvents the replication task has pushed.
        // Returns empty without replication (test, no aspen feature, or
        // sync disabled).
        let mut events = Vec::new();
        if let Ok(mut rx) = self.sync_events_rx.lock() {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }
        Ok(events)
    }
}

impl Drop for LocalFirstStorage {
    fn drop(&mut self) {
        let _ = self.replication_tx.send(ReplicationMsg::Shutdown);
    }
}

// ── Background replication task (aspen feature only) ────────────────────────
//
// These functions run in a tokio::spawn task. The task connects to Aspen once,
// then loops: draining WAL entries outbound and polling for inbound remote
// changes. Merge results are sent as SyncEvents through the channel.

/// Connect to Aspen and spawn the replication loop.
///
/// Returns None if the cluster ticket is missing or connection fails.
/// The caller continues in local-only mode.
#[cfg(feature = "aspen")]
async fn spawn_replication_task(
    config: &StorageConfig,
    wal: Arc<WalManager>,
    local: Arc<LocalStorage>,
    device_id: &str,
    mut drain_rx: mpsc::UnboundedReceiver<ReplicationMsg>,
    sync_tx: mpsc::UnboundedSender<SyncEvent>,
) -> Option<tokio::task::JoinHandle<()>> {
    use super::aspen::AspenStorage;

    let ticket = match config.cluster_ticket.as_ref() {
        Some(t) => t,
        None => {
            tracing::warn!("sync_enabled but no cluster_ticket — replication disabled");
            return None;
        }
    };

    let aspen = match AspenStorage::connect(ticket, device_id).await {
        Ok(a) => {
            tracing::info!("Replication task connected to Aspen as '{}'", device_id);
            a
        }
        Err(e) => {
            tracing::warn!("Aspen connection failed, replication disabled: {e}");
            return None;
        }
    };

    // TODO: Initialize PeerClusterManager here once AspenStorage exposes
    // the raw AspenClient (needed for AddPeerCluster / UpdatePeerClusterFilter).
    // Peer data still flows via Aspen cluster-level replication; this just
    // skips the subscription filter setup.
    if !config.peers.is_empty() {
        tracing::info!(
            "{} peers configured (cluster-level replication active, filter setup pending)",
            config.peers.len()
        );
    }

    // Drain any WAL entries accumulated while Aspen was disconnected
    let pending = wal.len().unwrap_or(0);
    if pending > 0 {
        tracing::info!("Replication task starting with {} pending WAL entries", pending);
    }

    let device_id = device_id.to_string();

    Some(tokio::spawn(async move {
        let mut poll_timer = tokio::time::interval(Duration::from_secs(5));
        poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                msg = drain_rx.recv() => {
                    match msg {
                        Some(ReplicationMsg::Drain) => {
                            drain_wal(&wal, &aspen).await;
                        }
                        Some(ReplicationMsg::Shutdown) | None => {
                            tracing::info!("Replication task shutting down, final drain");
                            drain_wal(&wal, &aspen).await;
                            break;
                        }
                    }
                }
                _ = poll_timer.tick() => {
                    poll_remote_changes(&aspen, &local, &sync_tx, &device_id).await;
                }
            }
        }
    }))
}

/// Drain pending WAL entries to Aspen.
///
/// Replays each operation on the Aspen backend. Removes entries from the WAL
/// on success. Stops on first failure to preserve ordering — the next Drain
/// signal will retry from the failed entry.
#[cfg(feature = "aspen")]
async fn drain_wal(wal: &WalManager, aspen: &super::aspen::AspenStorage) {
    let entries = match wal.drain_pending() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Failed to read WAL: {e}");
            return;
        }
    };

    if entries.is_empty() {
        return;
    }

    tracing::debug!("Draining {} WAL entries to Aspen", entries.len());

    for (seq, entry) in entries {
        match replicate_op(&entry.op, aspen).await {
            Ok(()) => {
                if let Err(e) = wal.remove(seq) {
                    tracing::warn!("Failed to remove WAL entry {seq}: {e}");
                }
            }
            Err(e) => {
                tracing::warn!(
                    "WAL entry {seq} replication failed (attempt {}): {e}",
                    entry.attempts + 1
                );
                // Stop draining — preserve ordering, retry next time
                break;
            }
        }
    }
}

/// Replay a single WAL operation on the Aspen backend.
#[cfg(feature = "aspen")]
async fn replicate_op(op: &ReplicationOp, aspen: &super::aspen::AspenStorage) -> Result<()> {
    match op {
        ReplicationOp::RecordPlay(track) => aspen.record_play(track).await,
        ReplicationOp::SaveQueue(queue) => aspen.save_queue(queue).await,
        ReplicationOp::CacheSearch {
            query,
            service_filter,
            results,
        } => aspen.cache_search(query, *service_filter, results).await,
        ReplicationOp::SaveSearchHistory(history) => aspen.save_search_history(history).await,
        ReplicationOp::UploadBlob {
            track_id,
            file_path,
        } => aspen.upload_blob(track_id, file_path).await.map(|_| ()),
        ReplicationOp::SavePlaylist(playlist) => aspen.save_playlist(playlist).await,
        ReplicationOp::DeletePlaylist { playlist_id } => aspen.delete_playlist(playlist_id).await,
    }
}

/// Poll Aspen for remote changes, merge with local state, and emit SyncEvents.
///
/// Uses `AspenStorage::poll_changes()` for change detection (hash-based dedup
/// of our own writes). Remote queue changes go through `merge::merge_queue()`
/// which applies Lamport clock ordering. Remote history changes go through
/// `merge::merge_history()` which applies set-union semantics.
#[cfg(feature = "aspen")]
async fn poll_remote_changes(
    aspen: &super::aspen::AspenStorage,
    local: &LocalStorage,
    sync_tx: &mpsc::UnboundedSender<SyncEvent>,
    device_id: &str,
) {
    let events = match aspen.poll_changes().await {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!("Remote poll failed: {e}");
            return;
        }
    };

    for event in events {
        match event {
            SyncEvent::QueueChanged(remote_queue) => {
                // Load local queue and merge
                let local_queue = match local.load_queue().await {
                    Ok(Some(q)) => q,
                    Ok(None) => PersistedQueue::new(),
                    Err(e) => {
                        tracing::warn!("Failed to load local queue for merge: {e}");
                        continue;
                    }
                };

                match merge::merge_queue(&local_queue, &remote_queue, device_id) {
                    QueueMergeResult::AcceptRemote(merged) => {
                        // Persist the merged queue to local storage
                        if let Err(e) = local.save_queue(&merged).await {
                            tracing::warn!("Failed to save merged queue to local: {e}");
                            continue;
                        }
                        let _ = sync_tx.send(SyncEvent::QueueChanged(merged));
                    }
                    QueueMergeResult::KeepLocal => {
                        tracing::debug!("Remote queue rejected by merge (local is newer)");
                    }
                }
            }
            SyncEvent::HistoryChanged(remote_entries) => {
                // Merge with local history using set-union
                let local_history = match local.get_history(500).await {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::warn!("Failed to load local history for merge: {e}");
                        continue;
                    }
                };

                let new_entries = merge::merge_history(&local_history, &remote_entries);
                if !new_entries.is_empty() {
                    tracing::debug!(
                        "Merged {} new history entries from remote",
                        new_entries.len()
                    );
                    let _ = sync_tx.send(SyncEvent::HistoryChanged(new_entries));
                }
            }
            // Playlist and other events: forward directly
            other => {
                let _ = sync_tx.send(other);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{CoverArt, ServiceType};

    fn test_track(id: &str) -> Track {
        Track {
            id: id.to_string(),
            title: format!("Track {}", id),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_seconds: 180,
            cover_art: CoverArt::None,
            service: ServiceType::Tidal,
        }
    }

    #[tokio::test]
    async fn test_record_play_reads_from_local() {
        let storage = LocalFirstStorage::new_for_test(3600).unwrap();
        let track = test_track("1");

        storage.record_play(&track).await.unwrap();

        let history = storage.get_history(10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].track_id, "1");
    }

    #[tokio::test]
    async fn test_save_queue_stamps_device_and_lamport() {
        let storage = LocalFirstStorage::new_for_test(3600).unwrap();
        let queue = PersistedQueue::from_tracks(&[test_track("1")], Some(0), Some(0));

        storage.save_queue(&queue).await.unwrap();

        let loaded = storage.load_queue().await.unwrap().unwrap();
        assert_eq!(loaded.device_id, "test-device");
        assert_eq!(loaded.lamport_clock, 1); // first save = clock 1
        assert!(loaded.updated_at_ms > 0);
    }

    #[tokio::test]
    async fn test_lamport_increments() {
        let storage = LocalFirstStorage::new_for_test(3600).unwrap();
        let queue = PersistedQueue::from_tracks(&[test_track("1")], None, None);

        storage.save_queue(&queue).await.unwrap();
        storage.save_queue(&queue).await.unwrap();
        storage.save_queue(&queue).await.unwrap();

        let loaded = storage.load_queue().await.unwrap().unwrap();
        assert_eq!(loaded.lamport_clock, 3);
    }

    #[tokio::test]
    async fn test_wal_populated_on_write() {
        let storage = LocalFirstStorage::new_for_test(3600).unwrap();
        let track = test_track("1");

        storage.record_play(&track).await.unwrap();
        storage.save_queue(&PersistedQueue::new()).await.unwrap();

        // WAL should have 2 entries (record_play + save_queue)
        assert_eq!(storage.pending_wal_count(), 2);
    }

    #[tokio::test]
    async fn test_search_cache_roundtrip() {
        let storage = LocalFirstStorage::new_for_test(3600).unwrap();
        let results = SearchResults {
            tracks: vec![test_track("1")],
            ..Default::default()
        };

        storage.cache_search("test", None, &results).await.unwrap();
        let cached = storage.get_cached_search("test", None).await.unwrap();

        assert!(cached.is_some());
        assert_eq!(cached.unwrap().tracks.len(), 1);
    }

    #[tokio::test]
    async fn test_backend_name() {
        let storage = LocalFirstStorage::new_for_test(3600).unwrap();
        assert_eq!(storage.backend_name(), "local-first");
    }

    #[tokio::test]
    async fn test_poll_changes_empty_without_replication() {
        let storage = LocalFirstStorage::new_for_test(3600).unwrap();
        let events = storage.poll_changes().await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_poll_changes_receives_events() {
        // Create storage with a sync channel we control
        let local = Arc::new(LocalStorage::new_for_test(3600).unwrap());
        let wal = Arc::new(WalManager::new_in_memory().unwrap());
        let (replication_tx, _rx) = mpsc::unbounded_channel();
        let (sync_tx, sync_rx) = mpsc::unbounded_channel();

        let storage = LocalFirstStorage {
            local,
            wal,
            device_id: "test-device".to_string(),
            lamport_clock: AtomicU64::new(0),
            replication_tx,
            _replication_handle: None,
            sync_events_rx: std::sync::Mutex::new(sync_rx),
        };

        // Inject a SyncEvent through the channel
        let queue = PersistedQueue::new();
        sync_tx
            .send(SyncEvent::QueueChanged(queue))
            .unwrap();

        // poll_changes should return it
        let events = storage.poll_changes().await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SyncEvent::QueueChanged(_)));

        // Second poll should be empty
        let events = storage.poll_changes().await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_poll_changes_drains_multiple_events() {
        let local = Arc::new(LocalStorage::new_for_test(3600).unwrap());
        let wal = Arc::new(WalManager::new_in_memory().unwrap());
        let (replication_tx, _rx) = mpsc::unbounded_channel();
        let (sync_tx, sync_rx) = mpsc::unbounded_channel();

        let storage = LocalFirstStorage {
            local,
            wal,
            device_id: "test-device".to_string(),
            lamport_clock: AtomicU64::new(0),
            replication_tx,
            _replication_handle: None,
            sync_events_rx: std::sync::Mutex::new(sync_rx),
        };

        // Push multiple events
        sync_tx.send(SyncEvent::QueueChanged(PersistedQueue::new())).unwrap();
        sync_tx.send(SyncEvent::PlaylistChanged { playlist_id: "p1".to_string() }).unwrap();
        sync_tx.send(SyncEvent::PlaylistDeleted { playlist_id: "p2".to_string() }).unwrap();

        // All three should arrive in one poll
        let events = storage.poll_changes().await.unwrap();
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn test_poll_changes_empty_after_sender_dropped() {
        let local = Arc::new(LocalStorage::new_for_test(3600).unwrap());
        let wal = Arc::new(WalManager::new_in_memory().unwrap());
        let (replication_tx, _rx) = mpsc::unbounded_channel();
        let (sync_tx, sync_rx) = mpsc::unbounded_channel();

        let storage = LocalFirstStorage {
            local,
            wal,
            device_id: "test-device".to_string(),
            lamport_clock: AtomicU64::new(0),
            replication_tx,
            _replication_handle: None,
            sync_events_rx: std::sync::Mutex::new(sync_rx),
        };

        // Drop the sender (simulates replication task exit)
        drop(sync_tx);

        // Should return empty, not error
        let events = storage.poll_changes().await.unwrap();
        assert!(events.is_empty());
    }
}
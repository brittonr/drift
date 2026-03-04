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

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing;

use super::local::LocalStorage;
use super::merge::{self, QueueMergeResult};
use super::wal::{ReplicationOp, WalManager};
use super::{BlobRef, DriftStorage, SyncEvent};
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
    local: LocalStorage,
    wal: WalManager,
    device_id: String,
    lamport_clock: AtomicU64,
    /// Notify the replication task that new ops are available.
    replication_tx: mpsc::UnboundedSender<ReplicationMsg>,
    /// Handle to the background replication task.
    _replication_handle: Option<tokio::task::JoinHandle<()>>,
    /// Track last-known queue hash to detect remote changes.
    last_queue_hash: std::sync::Mutex<Option<u64>>,
    /// Track last-known history count to detect remote changes.
    last_history_hash: std::sync::Mutex<Option<u64>>,
}

impl LocalFirstStorage {
    /// Create a new local-first storage backend.
    ///
    /// If `sync_config` indicates sync is desired and the aspen feature is enabled,
    /// connects to the Aspen cluster for background replication. Falls back
    /// gracefully to local-only if connection fails.
    pub async fn new(config: &StorageConfig, cache_ttl_seconds: u64) -> Result<Self> {
        let local = LocalStorage::new(cache_ttl_seconds)?;
        let wal = WalManager::new()?;

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

        // Spawn background replication if sync is enabled
        let replication_handle = if config.wants_sync() {
            #[cfg(feature = "aspen")]
            {
                Self::spawn_replication_task(config, wal_clone_for_replication, replication_rx).await
            }
            #[cfg(not(feature = "aspen"))]
            {
                tracing::info!("Sync requested but 'aspen' feature not enabled — local only");
                drop(replication_rx);
                None
            }
        } else {
            drop(replication_rx);
            None
        };

        Ok(Self {
            local,
            wal,
            device_id,
            lamport_clock: AtomicU64::new(initial_lamport),
            replication_tx,
            _replication_handle: replication_handle,
            last_queue_hash: std::sync::Mutex::new(None),
            last_history_hash: std::sync::Mutex::new(None),
        })
    }

    /// Create a local-first storage for tests (no remote, in-memory).
    pub fn new_for_test(cache_ttl_seconds: u64) -> Result<Self> {
        let local = LocalStorage::new_for_test(cache_ttl_seconds)?;
        let wal = WalManager::new_in_memory()?;
        let (replication_tx, _rx) = mpsc::unbounded_channel();

        Ok(Self {
            local,
            wal,
            device_id: "test-device".to_string(),
            lamport_clock: AtomicU64::new(0),
            replication_tx,
            _replication_handle: None,
            last_queue_hash: std::sync::Mutex::new(None),
            last_history_hash: std::sync::Mutex::new(None),
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

        // Update our hash tracker so we don't echo our own write back
        let hash = simple_hash(&stamped);
        if let Ok(mut h) = self.last_queue_hash.lock() {
            *h = Some(hash);
        }

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
        self.local.cache_search(query, service_filter, results).await?;
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
        // In the local-first model, remote polling is done by the replication
        // task. When remote changes arrive, they're merged into local storage
        // and sync events are emitted. For now, we check if local state has
        // been updated by the replication task by comparing hashes.
        //
        // TODO: When Aspen replication task is wired, it will write merged
        // changes to local and push SyncEvents through a channel.
        Ok(Vec::new())
    }
}

impl Drop for LocalFirstStorage {
    fn drop(&mut self) {
        let _ = self.replication_tx.send(ReplicationMsg::Shutdown);
    }
}

/// Quick non-cryptographic hash for change detection.
fn simple_hash(queue: &PersistedQueue) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    queue.tracks.len().hash(&mut hasher);
    queue.lamport_clock.hash(&mut hasher);
    queue.device_id.hash(&mut hasher);
    if let Some(pos) = queue.current_position {
        pos.hash(&mut hasher);
    }
    hasher.finish()
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
}

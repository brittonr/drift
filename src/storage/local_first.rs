//! Local storage remains authoritative for playback. A durable WAL feeds S3 sync.
//! Failed remote operations remain queued. Startup never expires pending writes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(feature = "s3")]
use anyhow::Context;
use anyhow::{ensure, Result};
use async_trait::async_trait;
use tokio::sync::mpsc;

use super::local::LocalStorage;
use super::wal::{ReplicationOp, WalManager};
use super::{BlobRef, DriftStorage, PlaylistIndexEntry, SyncEvent, SyncedPlaylist};
use crate::config::StorageConfig;
use crate::history_db::HistoryEntry;
use crate::queue_persistence::PersistedQueue;
use crate::search::SearchHistory;
use crate::service::{SearchResults, ServiceType, Track};

const NOTIFICATION_CAPACITY: usize = 1;

pub(super) enum ReplicationMsg {
    Drain,
    Shutdown,
}

pub struct LocalFirstStorage {
    pub(super) local: Arc<LocalStorage>,
    pub(super) wal: Arc<WalManager>,
    pub(super) device_id: String,
    pub(super) lamport_clock: Arc<AtomicU64>,
    pub(super) local_gate: Arc<tokio::sync::Mutex<()>>,
    max_wal_entries: usize,
    queue_enabled: bool,
    replication_tx: mpsc::Sender<ReplicationMsg>,
    replication_handle: Option<tokio::task::JoinHandle<()>>,
    sync_events_rx: std::sync::Mutex<mpsc::UnboundedReceiver<SyncEvent>>,
    #[cfg(feature = "s3")]
    pub(super) remote: Option<Arc<super::s3::S3Storage>>,
}

impl LocalFirstStorage {
    pub async fn new(config: &StorageConfig, cache_ttl_seconds: u64) -> Result<Self> {
        config.validate_sync()?;
        #[cfg(not(feature = "s3"))]
        ensure!(!config.wants_sync(), "sync requires the s3 build feature");
        let local = Arc::new(LocalStorage::new(cache_ttl_seconds)?);
        let wal = Arc::new(WalManager::new()?);
        let mut initial = local
            .load_queue()
            .await?
            .map_or(0, |queue| queue.lamport_clock);
        for index in local.list_playlists().await? {
            if let Some(playlist) = local.load_playlist(&index.id).await? {
                initial = initial.max(playlist.lamport_clock);
            }
        }
        for (_, entry) in wal.drain_pending()? {
            let clock = match entry.op {
                ReplicationOp::SaveQueue(queue) => queue.lamport_clock,
                ReplicationOp::SavePlaylist(playlist) => playlist.lamport_clock,
                _ => 0,
            };
            initial = initial.max(clock);
        }
        if config.wants_sync() {
            let identity = serde_json::to_vec(&(
                &config.user_id,
                config.resolved_device_id(),
                config
                    .s3
                    .as_ref()
                    .map(|s3| (&s3.endpoint, &s3.bucket, &s3.prefix, &s3.celld_endpoint)),
            ))?;
            wal.bind_context(blake3::hash(&identity).to_hex().as_str())?;
        }
        let (replication_tx, replication_rx) = mpsc::channel(NOTIFICATION_CAPACITY);
        let (sync_tx, sync_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "s3")]
        let remote = if config.wants_sync() {
            Some(Arc::new(super::s3::S3Storage::new(
                config.s3.as_ref().context("missing S3 configuration")?,
                config.user_id.as_deref().context("missing sync account")?,
            )?))
        } else {
            None
        };
        let storage = Self {
            local,
            wal,
            device_id: config.resolved_device_id(),
            lamport_clock: Arc::new(AtomicU64::new(initial)),
            local_gate: Arc::new(tokio::sync::Mutex::new(())),
            max_wal_entries: config.wal_max_entries,
            queue_enabled: config.wants_sync(),
            replication_tx,
            replication_handle: None,
            sync_events_rx: std::sync::Mutex::new(sync_rx),
            #[cfg(feature = "s3")]
            remote,
        };
        #[cfg(feature = "s3")]
        let mut storage = storage;
        #[cfg(feature = "s3")]
        if let Some(remote) = &storage.remote {
            storage.replication_handle = Some(super::replication_task::spawn(
                &storage,
                Arc::clone(remote),
                replication_rx,
                sync_tx,
            ));
        }
        #[cfg(not(feature = "s3"))]
        {
            drop(replication_rx);
            drop(sync_tx);
        }
        Ok(storage)
    }

    #[doc(hidden)]
    pub fn new_for_test(cache_ttl_seconds: u64) -> Result<Self> {
        let (replication_tx, _rx) = mpsc::channel(NOTIFICATION_CAPACITY);
        let (_sync_tx, sync_rx) = mpsc::unbounded_channel();
        Ok(Self {
            local: Arc::new(LocalStorage::new_for_test(cache_ttl_seconds)?),
            wal: Arc::new(WalManager::new_in_memory()?),
            device_id: "test-device".into(),
            lamport_clock: Arc::new(AtomicU64::new(0)),
            local_gate: Arc::new(tokio::sync::Mutex::new(())),
            max_wal_entries: StorageConfig::default().wal_max_entries,
            queue_enabled: true,
            replication_tx,
            replication_handle: None,
            sync_events_rx: std::sync::Mutex::new(sync_rx),
            #[cfg(feature = "s3")]
            remote: None,
        })
    }

    fn queue_replication(&self, op: ReplicationOp) -> Result<()> {
        if !self.queue_enabled {
            return Ok(());
        }
        ensure!(
            self.wal.len()? < self.max_wal_entries,
            "WAL is full; local data is saved but this operation is not queued"
        );
        self.wal.append(&op)?;
        // A single pending notification covers all durable entries.
        let _ = self.replication_tx.try_send(ReplicationMsg::Drain);
        Ok(())
    }

    fn next_lamport(&self) -> Result<u64> {
        self.lamport_clock
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |clock| {
                clock.checked_add(1)
            })
            .map(|previous| previous + 1)
            .map_err(|_| anyhow::anyhow!("Lamport clock exhausted"))
    }

    pub fn pending_wal_count(&self) -> usize {
        self.wal.len().unwrap_or(0)
    }
}

#[async_trait]
impl DriftStorage for LocalFirstStorage {
    fn backend_name(&self) -> &str {
        "local-first"
    }

    async fn record_play(&self, track: &Track) -> Result<()> {
        let _guard = self.local_gate.lock().await;
        self.local.record_play(track).await?;
        self.queue_replication(ReplicationOp::RecordPlay(track.clone()))
    }

    async fn get_history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        self.local.get_history(limit).await
    }

    async fn save_queue(&self, queue: &PersistedQueue) -> Result<()> {
        let _guard = self.local_gate.lock().await;
        let mut stamped = queue.clone();
        stamped.device_id = self.device_id.clone();
        stamped.lamport_clock = self.next_lamport()?;
        stamped.updated_at_ms = chrono::Utc::now().timestamp_millis().try_into()?;
        self.local.save_queue(&stamped).await?;
        self.queue_replication(ReplicationOp::SaveQueue(stamped))
    }

    async fn load_queue(&self) -> Result<Option<PersistedQueue>> {
        self.local.load_queue().await
    }

    async fn cache_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
        results: &SearchResults,
    ) -> Result<()> {
        let _guard = self.local_gate.lock().await;
        self.local
            .cache_search(query, service_filter, results)
            .await?;
        self.queue_replication(ReplicationOp::CacheSearch {
            query: query.into(),
            service_filter,
            results: results.clone(),
        })
    }

    async fn get_cached_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
    ) -> Result<Option<SearchResults>> {
        self.local.get_cached_search(query, service_filter).await
    }

    async fn save_search_history(&self, history: &SearchHistory) -> Result<()> {
        let _guard = self.local_gate.lock().await;
        self.local.save_search_history(history).await?;
        self.queue_replication(ReplicationOp::SaveSearchHistory(history.clone()))
    }

    async fn load_search_history(&self, max_size: usize) -> Result<SearchHistory> {
        self.local.load_search_history(max_size).await
    }

    async fn save_playlist(&self, playlist: &SyncedPlaylist) -> Result<()> {
        let _guard = self.local_gate.lock().await;
        let mut stamped = playlist.clone();
        stamped.device_id = self.device_id.clone();
        stamped.lamport_clock = self.next_lamport()?;
        stamped.updated_at_ms = chrono::Utc::now().timestamp_millis().try_into()?;
        self.local.save_playlist(&stamped).await?;
        self.queue_replication(ReplicationOp::SavePlaylist(stamped))
    }

    async fn load_playlist(&self, id: &str) -> Result<Option<SyncedPlaylist>> {
        self.local.load_playlist(id).await
    }
    async fn list_playlists(&self) -> Result<Vec<PlaylistIndexEntry>> {
        self.local.list_playlists().await
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let _guard = self.local_gate.lock().await;
        self.local.delete_playlist(playlist_id).await?;
        self.queue_replication(ReplicationOp::DeletePlaylist {
            playlist_id: playlist_id.into(),
        })
    }

    async fn upload_blob(&self, track_id: &str, file_path: &str) -> Result<Option<String>> {
        let _guard = self.local_gate.lock().await;
        self.queue_replication(ReplicationOp::UploadBlob {
            track_id: track_id.into(),
            file_path: file_path.into(),
            expected_hash: None,
        })?;
        Ok(None) // Queued is not a remote publication acknowledgement.
    }

    async fn has_blob(&self, track_id: &str) -> Result<Option<BlobRef>> {
        #[cfg(feature = "s3")]
        if let Some(remote) = &self.remote {
            return remote.has_blob(track_id).await;
        }
        let _ = track_id;
        Ok(None)
    }

    async fn fetch_blob(&self, track_id: &str) -> Result<Option<Vec<u8>>> {
        #[cfg(feature = "s3")]
        if let Some(remote) = &self.remote {
            return remote.fetch_blob(track_id).await;
        }
        let _ = track_id;
        Ok(None)
    }

    async fn poll_changes(&self) -> Result<Vec<SyncEvent>> {
        let mut events = Vec::new();
        let mut rx = self
            .sync_events_rx
            .lock()
            .map_err(|_| anyhow::anyhow!("sync event lock poisoned"))?;
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        Ok(events)
    }
}

impl Drop for LocalFirstStorage {
    fn drop(&mut self) {
        let _ = self.replication_tx.try_send(ReplicationMsg::Shutdown);
        // Cancellation can leave an unknown remote result, but its WAL entry survives.
        if let Some(handle) = &self.replication_handle {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests;

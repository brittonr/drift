//! Background I/O. Network waits never hold the local mutation lock.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{mpsc, Mutex};

use super::local::LocalStorage;
use super::local_first::{LocalFirstStorage, ReplicationMsg};
use super::replication::{queue_order, Snapshot, HISTORY_PREFIX, PLAYLIST_PREFIX, QUEUE_KEY};
use super::s3::{bind_blob, S3Storage};
use super::settings::POLL_INTERVAL_SECONDS;
use super::wal::WalManager;
use super::{DriftStorage, SyncEvent, SyncedPlaylist};
use crate::queue_persistence::PersistedQueue;

struct Task {
    remote: Arc<S3Storage>,
    wal: Arc<WalManager>,
    local: Arc<LocalStorage>,
    device: String,
    clock: Arc<AtomicU64>,
    local_gate: Arc<Mutex<()>>,
    events: mpsc::UnboundedSender<SyncEvent>,
}

pub(super) fn spawn(
    storage: &LocalFirstStorage,
    remote: Arc<S3Storage>,
    mut rx: mpsc::Receiver<ReplicationMsg>,
    events: mpsc::UnboundedSender<SyncEvent>,
) -> tokio::task::JoinHandle<()> {
    let task = Task {
        remote,
        wal: storage.wal.clone(),
        local: storage.local.clone(),
        device: storage.device_id.clone(),
        clock: storage.lamport_clock.clone(),
        local_gate: storage.local_gate.clone(),
        events,
    };
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECONDS));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_snapshot = None;
        loop {
            tokio::select! {
                message = rx.recv() => {
                    if !matches!(message, Some(ReplicationMsg::Drain)) { break; }
                }
                _ = timer.tick() => {}
            }
            match drain(&task.wal, &task.remote, &task.device).await {
                Ok(true) => last_snapshot = None,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!("Replication stopped; WAL entries retained: {error}");
                    continue;
                }
            }
            let result = async {
                let snapshot = task.remote.snapshot().await?;
                let bytes = snapshot.encode()?;
                let identity = blake3::hash(&bytes);
                if last_snapshot == Some(identity) {
                    return Ok::<_, anyhow::Error>(());
                }
                // A local mutation during the remote read must reach the server first.
                let _guard = task.local_gate.lock().await;
                if task.wal.len()? != 0 {
                    return Ok(());
                }
                task.apply_snapshot(&snapshot).await?;
                last_snapshot = Some(identity);
                Ok(())
            }
            .await;
            if let Err(error) = result {
                tracing::warn!("Remote state was not applied: {error}");
            }
        }
    })
}

pub(super) async fn drain(wal: &WalManager, remote: &S3Storage, device: &str) -> Result<bool> {
    let pending = wal.drain_pending()?;
    let had_pending = !pending.is_empty();
    for (sequence, mut entry) in pending {
        if bind_blob(&mut entry).await? {
            wal.update_entry(sequence, &entry)?;
        }
        remote.replicate(device, sequence, &entry).await?;
        // Stop on local removal failure too. Never acknowledge later entries first.
        wal.remove(sequence)?;
    }
    Ok(had_pending)
}

impl Task {
    async fn apply_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        if let Some(queue) = snapshot.value::<PersistedQueue>(QUEUE_KEY)? {
            self.clock.fetch_max(queue.lamport_clock, Ordering::SeqCst);
            let local = self.local.load_queue().await?;
            if local
                .as_ref()
                .is_none_or(|local| queue_order(&queue) > queue_order(local))
            {
                self.local.save_queue(&queue).await?;
                let _ = self.events.send(SyncEvent::QueueChanged(queue));
            }
        }
        let mut history = Vec::new();
        for (key, document) in &snapshot.documents {
            if key.starts_with(HISTORY_PREFIX) {
                let value = document
                    .value
                    .clone()
                    .context("history record cannot be a tombstone")?;
                history.push(serde_json::from_value::<drift_plugin::HistoryRecord>(
                    value,
                )?);
            }
            if let Some(id) = key.strip_prefix(PLAYLIST_PREFIX) {
                match &document.value {
                    Some(value) => {
                        let playlist: SyncedPlaylist = serde_json::from_value(value.clone())?;
                        anyhow::ensure!(playlist.id == id, "playlist identity mismatch");
                        self.clock
                            .fetch_max(playlist.lamport_clock, Ordering::SeqCst);
                        self.local.save_playlist(&playlist).await?;
                        let _ = self.events.send(SyncEvent::PlaylistChanged {
                            playlist_id: id.into(),
                        });
                    }
                    None => {
                        self.local.delete_playlist(id).await?;
                        let _ = self.events.send(SyncEvent::PlaylistDeleted {
                            playlist_id: id.into(),
                        });
                    }
                }
            }
        }
        if !history.is_empty() {
            self.local.import_history(&history)?;
            let _ = self.events.send(SyncEvent::HistoryChanged(
                self.local
                    .get_history(drift_plugin::DEFAULT_MAX_HISTORY_ENTRIES)
                    .await?,
            ));
        }
        Ok(())
    }
}

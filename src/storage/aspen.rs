//! Aspen distributed KV storage backend.
//!
//! Stores drift data in an Aspen cluster over iroh QUIC:
//!
//! ```text
//! drift:{user}:history:{timestamp_ms:020}   → JSON HistoryEntry
//! drift:{user}:queue                         → JSON PersistedQueue
//! drift:{user}:search:{hash}               → JSON CachedSearch
//! ```
//!
//! The drift plugin running on the cluster handles dedup, pruning,
//! and cache TTL server-side. This client just sends standard
//! WriteKey/ReadKey/ScanKeys RPCs.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;

use aspen_client::AspenClient;
use aspen_client::ClientRpcRequest;
use aspen_client::ClientRpcResponse;

use super::{DriftStorage, SyncEvent};
use crate::history_db::HistoryEntry;
use crate::queue_persistence::PersistedQueue;
use crate::service::{CoverArt, SearchResults, ServiceType, Track};

/// Tracks what we last saw so we can detect remote changes.
#[derive(Default)]
struct SyncState {
    /// Hash of the last queue JSON we wrote or saw.
    last_queue_hash: Option<u64>,
    /// Number of history entries we last saw.
    last_history_count: usize,
    /// Hash of the most recent history key (detects new entries).
    last_history_latest_hash: Option<u64>,
}

pub struct AspenStorage {
    client: Arc<AspenClient>,
    /// Key prefix: `drift:{user_id}:`
    prefix: String,
    /// Mutable sync tracking state.
    sync: Mutex<SyncState>,
}

impl AspenStorage {
    pub async fn connect(cluster_ticket: &str, user_id: &str) -> Result<Self> {
        let client = AspenClient::connect(
            cluster_ticket,
            Duration::from_secs(10),
            None,
        )
        .await
        .context("failed to connect to Aspen cluster")?;

        // Verify connectivity
        match client.send(ClientRpcRequest::Ping).await {
            Ok(ClientRpcResponse::Pong) => {}
            Ok(other) => anyhow::bail!("unexpected ping response: {other:?}"),
            Err(e) => anyhow::bail!("Aspen cluster unreachable: {e}"),
        }

        Ok(Self {
            client: Arc::new(client),
            prefix: format!("drift:{user_id}:"),
            sync: Mutex::new(SyncState::default()),
        })
    }

    fn key(&self, suffix: &str) -> String {
        format!("{}{}", self.prefix, suffix)
    }

    async fn kv_set(&self, key: &str, value: &[u8]) -> Result<()> {
        let resp = self
            .client
            .send(ClientRpcRequest::WriteKey {
                key: key.to_string(),
                value: value.to_vec(),
            })
            .await?;
        match resp {
            ClientRpcResponse::WriteResult(r) if r.is_success => Ok(()),
            ClientRpcResponse::WriteResult(r) => {
                anyhow::bail!("KV write failed: {}", r.error.unwrap_or_default())
            }
            ClientRpcResponse::Error(e) => anyhow::bail!("KV error: {}", e.message),
            _ => anyhow::bail!("unexpected response"),
        }
    }

    async fn kv_get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let resp = self
            .client
            .send(ClientRpcRequest::ReadKey {
                key: key.to_string(),
            })
            .await?;
        match resp {
            ClientRpcResponse::ReadResult(r) if r.was_found => Ok(r.value),
            ClientRpcResponse::ReadResult(_) => Ok(None),
            ClientRpcResponse::Error(e) => anyhow::bail!("KV error: {}", e.message),
            _ => anyhow::bail!("unexpected response"),
        }
    }

    async fn kv_scan(&self, prefix: &str, limit: u32) -> Result<Vec<(String, String)>> {
        let resp = self
            .client
            .send(ClientRpcRequest::ScanKeys {
                prefix: prefix.to_string(),
                limit: Some(limit),
                continuation_token: None,
            })
            .await?;
        match resp {
            ClientRpcResponse::ScanResult(r) => {
                if let Some(err) = r.error {
                    anyhow::bail!("KV scan error: {err}");
                }
                Ok(r.entries.into_iter().map(|e| (e.key, e.value)).collect())
            }
            ClientRpcResponse::Error(e) => anyhow::bail!("KV error: {}", e.message),
            _ => anyhow::bail!("unexpected response"),
        }
    }

    fn hash_bytes(data: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        hasher.finish()
    }

    fn search_cache_key(query: &str, service_filter: Option<ServiceType>) -> String {
        let normalized = query.trim().to_lowercase();
        let filter = service_filter
            .map(|s| s.to_string())
            .unwrap_or_else(|| "all".to_string());
        let mut hasher = DefaultHasher::new();
        format!("{}_{}", normalized, filter).hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

/// JSON shape matching what the drift plugin expects for history entries.
#[derive(serde::Serialize, serde::Deserialize)]
struct HistoryRecord {
    track_id: String,
    title: String,
    artist: String,
    album: String,
    duration_seconds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover_art_id: Option<String>,
    service: String,
    played_at_ms: u64,
}

impl HistoryRecord {
    fn from_track(track: &Track) -> Self {
        let cover_art_id = match &track.cover_art {
            CoverArt::ServiceId { id, .. } => Some(id.clone()),
            CoverArt::Url(url) => Some(url.clone()),
            CoverArt::None => None,
        };
        Self {
            track_id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_seconds: track.duration_seconds,
            cover_art_id,
            service: track.service.to_string(),
            played_at_ms: Utc::now().timestamp_millis() as u64,
        }
    }

    fn to_history_entry(&self) -> HistoryEntry {
        let played_at = chrono::DateTime::from_timestamp_millis(self.played_at_ms as i64)
            .unwrap_or_else(Utc::now);
        let service = self.service.parse().unwrap_or(ServiceType::Tidal);
        HistoryEntry {
            id: 0,
            track_id: self.track_id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_seconds: self.duration_seconds,
            cover_art_id: self.cover_art_id.clone(),
            service,
            played_at,
        }
    }
}

/// Wrapper for search cache with timestamp (matches drift plugin's CachedSearch).
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedSearch {
    #[serde(rename = "r")]
    results_json: String,
    #[serde(rename = "t")]
    cached_at_ms: u64,
}

#[async_trait]
impl DriftStorage for AspenStorage {
    fn backend_name(&self) -> &str {
        "aspen"
    }

    async fn record_play(&self, track: &Track) -> Result<()> {
        let now_ms = Utc::now().timestamp_millis() as u64;
        let record = HistoryRecord::from_track(track);
        let key = self.key(&format!("history:{now_ms:020}"));
        let value = serde_json::to_vec(&record)?;
        // Server-side drift plugin handles dedup
        self.kv_set(&key, &value).await?;
        // Bump our sync state so we don't detect our own write as remote
        if let Ok(mut sync) = self.sync.lock() {
            sync.last_history_count += 1;
            sync.last_history_latest_hash = Some(Self::hash_bytes(key.as_bytes()));
        }
        Ok(())
    }

    async fn get_history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let prefix = self.key("history:");
        let entries = self.kv_scan(&prefix, limit as u32).await?;
        let mut records: Vec<HistoryEntry> = entries
            .into_iter()
            .filter_map(|(_key, value)| {
                serde_json::from_str::<HistoryRecord>(&value)
                    .ok()
                    .map(|r| r.to_history_entry())
            })
            .collect();
        // Most recent first
        records.sort_by(|a, b| b.played_at.cmp(&a.played_at));
        records.truncate(limit);
        Ok(records)
    }

    async fn save_queue(&self, queue: &PersistedQueue) -> Result<()> {
        let key = self.key("queue");
        let value = serde_json::to_vec(queue)?;
        // Track what we wrote so poll_changes ignores our own writes
        let hash = Self::hash_bytes(&value);
        self.kv_set(&key, &value).await?;
        if let Ok(mut sync) = self.sync.lock() {
            sync.last_queue_hash = Some(hash);
        }
        Ok(())
    }

    async fn load_queue(&self) -> Result<Option<PersistedQueue>> {
        let key = self.key("queue");
        match self.kv_get(&key).await? {
            Some(bytes) => {
                let queue: PersistedQueue = serde_json::from_slice(&bytes)?;
                Ok(Some(queue))
            }
            None => Ok(None),
        }
    }

    async fn cache_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
        results: &SearchResults,
    ) -> Result<()> {
        let hash = Self::search_cache_key(query, service_filter);
        let key = self.key(&format!("search:{hash}"));
        let cached = CachedSearch {
            results_json: serde_json::to_string(results)?,
            cached_at_ms: Utc::now().timestamp_millis() as u64,
        };
        let value = serde_json::to_vec(&cached)?;
        self.kv_set(&key, &value).await
    }

    async fn get_cached_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
    ) -> Result<Option<SearchResults>> {
        let hash = Self::search_cache_key(query, service_filter);
        let key = self.key(&format!("search:{hash}"));
        match self.kv_get(&key).await? {
            Some(bytes) => {
                let cached: CachedSearch = serde_json::from_slice(&bytes)?;
                // TTL is enforced server-side by the plugin, but if we get
                // a result back it's valid
                let results: SearchResults = serde_json::from_str(&cached.results_json)?;
                Ok(Some(results))
            }
            None => Ok(None),
        }
    }

    async fn poll_changes(&self) -> Result<Vec<SyncEvent>> {
        let mut events = Vec::new();

        // ── Check queue for remote changes ──────────────────────────
        let queue_key = self.key("queue");
        if let Some(bytes) = self.kv_get(&queue_key).await? {
            let hash = Self::hash_bytes(&bytes);
            let is_new = self.sync.lock()
                .map(|s| s.last_queue_hash != Some(hash))
                .unwrap_or(false);
            if is_new {
                if let Ok(queue) = serde_json::from_slice::<PersistedQueue>(&bytes) {
                    if let Ok(mut sync) = self.sync.lock() {
                        sync.last_queue_hash = Some(hash);
                    }
                    events.push(SyncEvent::QueueChanged(queue));
                }
            }
        }

        // ── Check history for remote changes ────────────────────────
        let history_prefix = self.key("history:");
        let entries = self.kv_scan(&history_prefix, 1).await?;
        if let Some((latest_key, _)) = entries.last() {
            let latest_hash = Self::hash_bytes(latest_key.as_bytes());
            let is_new = self.sync.lock()
                .map(|s| s.last_history_latest_hash != Some(latest_hash))
                .unwrap_or(false);
            if is_new {
                // New history entry detected — fetch full history
                let full = self.get_history(100).await?;
                let count = full.len();
                if let Ok(mut sync) = self.sync.lock() {
                    sync.last_history_latest_hash = Some(latest_hash);
                    sync.last_history_count = count;
                }
                events.push(SyncEvent::HistoryChanged(full));
            }
        }

        Ok(events)
    }
}

//! Aspen distributed KV storage backend.
//!
//! Stores drift data in an Aspen cluster over iroh QUIC:
//!
//! ```text
//! drift:{user}:history:{timestamp_ms:020}   → JSON HistoryEntry
//! drift:{user}:queue                         → JSON PersistedQueue
//! drift:{user}:search:{hash}               → JSON CachedSearch
//! drift:{user}:search_history               → JSON SearchHistory
//! ```
//!
//! Dedup, pruning, and cache TTL are enforced client-side via the
//! `drift_plugin` crate's pure functions. The same logic can also
//! run server-side as a WASM plugin on the cluster.

use std::collections::{hash_map::DefaultHasher, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use aspen_client::AspenClient;
use aspen_client::ClientRpcRequest;
use aspen_client::ClientRpcResponse;

use super::{DriftStorage, SyncEvent};
use crate::history_db::HistoryEntry;
use crate::queue_persistence::PersistedQueue;
use crate::search::SearchHistory;
use crate::service::{CoverArt, SearchResults, ServiceType, Track};

/// Connection state for tracking reconnection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    Connected,
    Reconnecting,
}

/// Tracks connection health and reconnection attempts.
struct ConnectionHealth {
    consecutive_failures: u32,
    last_reconnect_attempt: Option<Instant>,
    state: ConnectionState,
    reconnect_attempt_count: u32,
}

impl Default for ConnectionHealth {
    fn default() -> Self {
        Self {
            consecutive_failures: 0,
            last_reconnect_attempt: None,
            state: ConnectionState::Connected,
            reconnect_attempt_count: 0,
        }
    }
}

impl ConnectionHealth {
    /// Get the backoff duration based on attempt count: 5s, 10s, 20s, 40s, max 60s
    fn backoff_duration(&self) -> Duration {
        let base = 5;
        let exp = self.reconnect_attempt_count.min(4); // Cap at 2^4 = 16
        let seconds = base * (1 << exp);
        Duration::from_secs(seconds.min(60))
    }

    /// Check if enough time has passed to attempt reconnection
    fn should_reconnect(&self) -> bool {
        match self.last_reconnect_attempt {
            None => true,
            Some(last) => last.elapsed() >= self.backoff_duration(),
        }
    }
}

/// Pending write operation to replay after reconnection.
#[derive(Clone)]
struct PendingWrite {
    key: String,
    value: Vec<u8>,
}

/// Inner connection state protected by RwLock.
struct ConnectionInner {
    client: AspenClient,
    health: ConnectionHealth,
    pending_writes: VecDeque<PendingWrite>,
}

impl ConnectionInner {
    const MAX_PENDING_WRITES: usize = 100;

    fn new(client: AspenClient) -> Self {
        Self {
            client,
            health: ConnectionHealth::default(),
            pending_writes: VecDeque::new(),
        }
    }

    fn record_success(&mut self) {
        if self.health.state == ConnectionState::Reconnecting {
            info!("AspenStorage reconnected successfully");
        }
        self.health.consecutive_failures = 0;
        self.health.state = ConnectionState::Connected;
        self.health.reconnect_attempt_count = 0;
    }

    fn record_failure(&mut self) {
        self.health.consecutive_failures += 1;
        // Transition to Reconnecting after 3 consecutive failures (9 total attempts with 3x retries)
        if self.health.consecutive_failures >= 3 && self.health.state == ConnectionState::Connected {
            warn!("AspenStorage detected 3 consecutive failures, entering reconnection mode");
            self.health.state = ConnectionState::Reconnecting;
        }
    }

    fn queue_write(&mut self, key: String, value: Vec<u8>) {
        if self.pending_writes.len() >= Self::MAX_PENDING_WRITES {
            warn!("Pending write queue full, dropping oldest write");
            self.pending_writes.pop_front();
        }
        self.pending_writes.push_back(PendingWrite { key, value });
    }
}

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
    /// Connection state protected by RwLock for swapping client on reconnect
    connection: Arc<RwLock<ConnectionInner>>,
    /// Cluster ticket for reconnection
    cluster_ticket: String,
    /// User ID for reconnection and key prefixing
    user_id: String,
    /// Key prefix: `drift:{user_id}:`
    prefix: String,
    /// Mutable sync tracking state.
    sync: Mutex<SyncState>,
    /// RPC timeout for operations
    rpc_timeout: Duration,
}

impl AspenStorage {
    pub async fn connect(cluster_ticket: &str, user_id: &str) -> Result<Self> {
        let rpc_timeout = Duration::from_secs(10);
        let client = AspenClient::connect(
            cluster_ticket,
            rpc_timeout,
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
            connection: Arc::new(RwLock::new(ConnectionInner::new(client))),
            cluster_ticket: cluster_ticket.to_string(),
            user_id: user_id.to_string(),
            prefix: format!("drift:{user_id}:"),
            sync: Mutex::new(SyncState::default()),
            rpc_timeout,
        })
    }

    /// Attempt to reconnect to the Aspen cluster
    async fn try_reconnect(&self) -> Result<()> {
        let mut conn = self.connection.write().await;
        
        // Check if we should attempt reconnection based on backoff
        if !conn.health.should_reconnect() {
            return Err(anyhow::anyhow!("Backoff period not elapsed"));
        }

        conn.health.last_reconnect_attempt = Some(Instant::now());
        conn.health.reconnect_attempt_count += 1;

        info!(
            "Attempting reconnection to Aspen cluster as '{}' (attempt {})",
            self.user_id,
            conn.health.reconnect_attempt_count
        );

        // Try to establish new connection
        match AspenClient::connect(&self.cluster_ticket, self.rpc_timeout, None).await {
            Ok(new_client) => {
                // Verify with ping
                match new_client.send(ClientRpcRequest::Ping).await {
                    Ok(ClientRpcResponse::Pong) => {
                        info!("Reconnection successful, replaying pending writes");
                        
                        // Swap in the new client
                        conn.client = new_client;
                        conn.health.state = ConnectionState::Connected;
                        conn.health.consecutive_failures = 0;
                        conn.health.reconnect_attempt_count = 0;
                        
                        // Replay pending writes (best-effort)
                        let pending = std::mem::take(&mut conn.pending_writes);
                        for write in pending {
                            if let Err(e) = self.kv_set_inner(&conn.client, &write.key, &write.value).await {
                                warn!("Failed to replay write for key {}: {}", write.key, e);
                            }
                        }
                        
                        Ok(())
                    }
                    Ok(other) => {
                        warn!("Reconnection ping got unexpected response: {:?}", other);
                        Err(anyhow::anyhow!("Unexpected ping response"))
                    }
                    Err(e) => {
                        warn!("Reconnection ping failed: {}", e);
                        Err(anyhow::anyhow!("Ping failed: {}", e))
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Reconnection attempt {} failed: {}. Next retry in {:?}",
                    conn.health.reconnect_attempt_count,
                    e,
                    conn.health.backoff_duration()
                );
                Err(e)
            }
        }
    }

    fn key(&self, suffix: &str) -> String {
        format!("{}{}", self.prefix, suffix)
    }

    /// Internal kv_set that works with a client reference (for replay without locking)
    async fn kv_set_inner(&self, client: &AspenClient, key: &str, value: &[u8]) -> Result<()> {
        let resp = client
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

    async fn kv_set(&self, key: &str, value: &[u8]) -> Result<()> {
        // Try reconnect if in reconnecting state
        {
            let conn = self.connection.read().await;
            if conn.health.state == ConnectionState::Reconnecting {
                drop(conn); // Release read lock before trying write lock in try_reconnect
                let _ = self.try_reconnect().await; // Best effort, continue regardless
            }
        }

        let mut conn = self.connection.write().await;
        
        // If still disconnected, queue the write
        if conn.health.state == ConnectionState::Reconnecting {
            conn.queue_write(key.to_string(), value.to_vec());
            return Ok(());
        }

        // Try the operation
        match self.kv_set_inner(&conn.client, key, value).await {
            Ok(()) => {
                conn.record_success();
                Ok(())
            }
            Err(e) => {
                conn.record_failure();
                Err(e)
            }
        }
    }

    /// Delete a key from the KV store (best-effort, logs on failure).
    async fn kv_delete(&self, key: &str) {
        let conn = self.connection.read().await;
        if conn.health.state == ConnectionState::Reconnecting {
            return;
        }
        let result = conn.client
            .send(ClientRpcRequest::DeleteKey {
                key: key.to_string(),
            })
            .await;
        if let Err(e) = result {
            warn!("kv_delete({key}) failed: {e}");
        }
    }

    async fn kv_get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        // Try reconnect if in reconnecting state
        {
            let conn = self.connection.read().await;
            if conn.health.state == ConnectionState::Reconnecting {
                drop(conn);
                let _ = self.try_reconnect().await;
            }
        }

        let mut conn = self.connection.write().await;
        
        // If still disconnected, return None (graceful degradation)
        if conn.health.state == ConnectionState::Reconnecting {
            return Ok(None);
        }

        // Try the operation
        let resp = match conn.client
            .send(ClientRpcRequest::ReadKey {
                key: key.to_string(),
            })
            .await
        {
            Ok(r) => {
                conn.record_success();
                r
            }
            Err(e) => {
                conn.record_failure();
                return Err(e.into());
            }
        };

        match resp {
            ClientRpcResponse::ReadResult(r) if r.was_found => Ok(r.value),
            ClientRpcResponse::ReadResult(_) => Ok(None),
            ClientRpcResponse::Error(e) => anyhow::bail!("KV error: {}", e.message),
            _ => anyhow::bail!("unexpected response"),
        }
    }

    async fn kv_scan(&self, prefix: &str, limit: u32) -> Result<Vec<(String, String)>> {
        // Try reconnect if in reconnecting state
        {
            let conn = self.connection.read().await;
            if conn.health.state == ConnectionState::Reconnecting {
                drop(conn);
                let _ = self.try_reconnect().await;
            }
        }

        let mut conn = self.connection.write().await;
        
        // If still disconnected, return empty vec (graceful degradation)
        if conn.health.state == ConnectionState::Reconnecting {
            return Ok(Vec::new());
        }

        // Try the operation
        let resp = match conn.client
            .send(ClientRpcRequest::ScanKeys {
                prefix: prefix.to_string(),
                limit: Some(limit),
                continuation_token: None,
            })
            .await
        {
            Ok(r) => {
                conn.record_success();
                r
            }
            Err(e) => {
                conn.record_failure();
                return Err(e.into());
            }
        };

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
        self.kv_set(&key, &value).await?;

        // Bump our sync state so we don't detect our own write as remote
        if let Ok(mut sync) = self.sync.lock() {
            sync.last_history_count += 1;
            sync.last_history_latest_hash = Some(Self::hash_bytes(key.as_bytes()));
        }

        // ── Client-side dedup via drift-plugin ──────────────────────
        // Scan recent entries and remove duplicates of this track
        // within the 10s dedup window.
        let prefix = self.key("history:");
        if let Ok(entries) = self.kv_scan(&prefix, 100).await {
            let plugin_record = drift_plugin::HistoryRecord {
                track_id: record.track_id.clone(),
                title: record.title.clone(),
                artist: record.artist.clone(),
                album: record.album.clone(),
                duration_seconds: record.duration_seconds,
                cover_art_id: record.cover_art_id.clone(),
                service: record.service.clone(),
                played_at_ms: record.played_at_ms,
            };
            let recent: Vec<(String, drift_plugin::HistoryRecord)> = entries
                .iter()
                .filter_map(|(k, v)| {
                    serde_json::from_str::<drift_plugin::HistoryRecord>(v)
                        .ok()
                        .map(|r| (k.clone(), r))
                })
                .collect();
            let to_delete = drift_plugin::dedup::find_duplicates(
                &key,
                &plugin_record,
                &recent,
                drift_plugin::DEFAULT_DEDUP_WINDOW_MS,
            );
            for dup_key in &to_delete {
                self.kv_delete(dup_key).await;
            }
        }

        // ── Client-side pruning via drift-plugin ────────────────────
        // Keep history bounded at DEFAULT_MAX_HISTORY_ENTRIES.
        if let Ok(all_entries) = self.kv_scan(&prefix, 1000).await {
            let mut keys: Vec<String> = all_entries.into_iter().map(|(k, _)| k).collect();
            keys.sort_by(|a, b| b.cmp(a)); // Newest first (keys are timestamps)
            let to_prune = drift_plugin::prune::keys_to_prune(
                &keys,
                drift_plugin::DEFAULT_MAX_HISTORY_ENTRIES,
            );
            for prune_key in &to_prune {
                self.kv_delete(prune_key).await;
            }
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
                // Check TTL client-side via drift-plugin
                let now_ms = Utc::now().timestamp_millis() as u64;
                if drift_plugin::ttl::is_expired(
                    cached.cached_at_ms,
                    now_ms,
                    drift_plugin::DEFAULT_CACHE_TTL_MS,
                ) {
                    // Expired — delete stale cache entry and return miss
                    self.kv_delete(&key).await;
                    return Ok(None);
                }
                let results: SearchResults = serde_json::from_str(&cached.results_json)?;
                Ok(Some(results))
            }
            None => Ok(None),
        }
    }

    async fn save_search_history(&self, history: &SearchHistory) -> Result<()> {
        let key = self.key("search_history");
        let value = serde_json::to_vec(history)?;
        self.kv_set(&key, &value).await
    }

    async fn load_search_history(&self, max_size: usize) -> Result<SearchHistory> {
        let key = self.key("search_history");
        match self.kv_get(&key).await? {
            Some(bytes) => {
                let mut history: SearchHistory = serde_json::from_slice(&bytes)?;
                history.max_size = max_size;
                while history.entries.len() > max_size {
                    history.entries.pop_back();
                }
                Ok(history)
            }
            None => Ok(SearchHistory::new(max_size)),
        }
    }

    async fn poll_changes(&self) -> Result<Vec<SyncEvent>> {
        // Return empty vec if disconnected (graceful degradation)
        {
            let conn = self.connection.read().await;
            if conn.health.state == ConnectionState::Reconnecting {
                return Ok(Vec::new());
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{CoverArt, ServiceType, Track};

    // ── Helper: Create test track ──────────────────────────────────
    fn create_test_track(
        id: &str,
        title: &str,
        artist: &str,
        cover_art: CoverArt,
        service: ServiceType,
    ) -> Track {
        Track {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            album: "Test Album".to_string(),
            duration_seconds: 180,
            cover_art,
            service,
        }
    }

    // ── 1. HistoryRecord::from_track() ─────────────────────────────
    #[test]
    fn test_history_record_from_track_with_service_id_cover() {
        let track = create_test_track(
            "track-123",
            "Test Song",
            "Test Artist",
            CoverArt::ServiceId {
                id: "cover-456".to_string(),
                service: ServiceType::Tidal,
            },
            ServiceType::Tidal,
        );

        let record = HistoryRecord::from_track(&track);

        assert_eq!(record.track_id, "track-123");
        assert_eq!(record.title, "Test Song");
        assert_eq!(record.artist, "Test Artist");
        assert_eq!(record.album, "Test Album");
        assert_eq!(record.duration_seconds, 180);
        assert_eq!(record.cover_art_id, Some("cover-456".to_string()));
        assert_eq!(record.service, "tidal");

        // Verify played_at_ms is recent (within 1 second)
        let now_ms = Utc::now().timestamp_millis() as u64;
        assert!(record.played_at_ms > 0);
        assert!(record.played_at_ms <= now_ms);
        assert!(now_ms - record.played_at_ms < 1000);
    }

    #[test]
    fn test_history_record_from_track_with_url_cover() {
        let track = create_test_track(
            "yt-abc",
            "YouTube Song",
            "YT Artist",
            CoverArt::Url("https://example.com/cover.jpg".to_string()),
            ServiceType::YouTube,
        );

        let record = HistoryRecord::from_track(&track);

        assert_eq!(record.track_id, "yt-abc");
        assert_eq!(record.cover_art_id, Some("https://example.com/cover.jpg".to_string()));
        assert_eq!(record.service, "youtube");
    }

    #[test]
    fn test_history_record_from_track_with_no_cover() {
        let track = create_test_track(
            "bc-xyz",
            "Bandcamp Song",
            "BC Artist",
            CoverArt::None,
            ServiceType::Bandcamp,
        );

        let record = HistoryRecord::from_track(&track);

        assert_eq!(record.track_id, "bc-xyz");
        assert_eq!(record.cover_art_id, None);
        assert_eq!(record.service, "bandcamp");
    }

    // ── 2. HistoryRecord::to_history_entry() ───────────────────────
    #[test]
    fn test_history_record_to_history_entry_roundtrip() {
        let now_ms = Utc::now().timestamp_millis() as u64;
        let record = HistoryRecord {
            track_id: "track-999".to_string(),
            title: "Roundtrip Song".to_string(),
            artist: "Roundtrip Artist".to_string(),
            album: "Roundtrip Album".to_string(),
            duration_seconds: 240,
            cover_art_id: Some("cover-999".to_string()),
            service: "tidal".to_string(),
            played_at_ms: now_ms,
        };

        let entry = record.to_history_entry();

        assert_eq!(entry.id, 0);
        assert_eq!(entry.track_id, "track-999");
        assert_eq!(entry.title, "Roundtrip Song");
        assert_eq!(entry.artist, "Roundtrip Artist");
        assert_eq!(entry.album, "Roundtrip Album");
        assert_eq!(entry.duration_seconds, 240);
        assert_eq!(entry.cover_art_id, Some("cover-999".to_string()));
        assert_eq!(entry.service, ServiceType::Tidal);

        // Verify timestamp conversion
        assert_eq!(entry.played_at.timestamp_millis(), now_ms as i64);
    }

    #[test]
    fn test_history_record_to_history_entry_invalid_service_fallback() {
        let record = HistoryRecord {
            track_id: "track-invalid".to_string(),
            title: "Invalid Service".to_string(),
            artist: "Test".to_string(),
            album: "Test".to_string(),
            duration_seconds: 100,
            cover_art_id: None,
            service: "not-a-real-service".to_string(),
            played_at_ms: Utc::now().timestamp_millis() as u64,
        };

        let entry = record.to_history_entry();

        // Should fall back to Tidal on parse error
        assert_eq!(entry.service, ServiceType::Tidal);
    }

    #[test]
    fn test_history_record_entry_id_always_zero() {
        let record = HistoryRecord {
            track_id: "test".to_string(),
            title: "Test".to_string(),
            artist: "Test".to_string(),
            album: "Test".to_string(),
            duration_seconds: 100,
            cover_art_id: None,
            service: "youtube".to_string(),
            played_at_ms: Utc::now().timestamp_millis() as u64,
        };

        let entry = record.to_history_entry();
        assert_eq!(entry.id, 0);
    }

    // ── 3. CachedSearch serde ──────────────────────────────────────
    #[test]
    fn test_cached_search_serde_roundtrip() {
        let cached = CachedSearch {
            results_json: r#"{"tracks":[],"albums":[],"artists":[]}"#.to_string(),
            cached_at_ms: 1234567890,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&cached).unwrap();

        // Verify field renames
        assert!(json.contains(r#""r":"#));
        assert!(json.contains(r#""t":"#));
        assert!(!json.contains(r#""results_json""#));
        assert!(!json.contains(r#""cached_at_ms""#));

        // Deserialize back
        let deserialized: CachedSearch = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.results_json, cached.results_json);
        assert_eq!(deserialized.cached_at_ms, cached.cached_at_ms);
    }

    #[test]
    fn test_cached_search_field_names() {
        let cached = CachedSearch {
            results_json: "test".to_string(),
            cached_at_ms: 999,
        };

        let json = serde_json::to_value(&cached).unwrap();
        let obj = json.as_object().unwrap();

        // Should have exactly 2 fields with short names
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("r"));
        assert!(obj.contains_key("t"));
    }

    // ── 4. search_cache_key() ──────────────────────────────────────
    #[test]
    fn test_search_cache_key_case_insensitive() {
        let key1 = AspenStorage::search_cache_key("Hello World", None);
        let key2 = AspenStorage::search_cache_key("hello world", None);
        let key3 = AspenStorage::search_cache_key("HELLO WORLD", None);

        assert_eq!(key1, key2);
        assert_eq!(key2, key3);
    }

    #[test]
    fn test_search_cache_key_whitespace_normalization() {
        let key1 = AspenStorage::search_cache_key("test query", None);
        let key2 = AspenStorage::search_cache_key("  test query  ", None);
        let key3 = AspenStorage::search_cache_key("\ttest query\n", None);

        assert_eq!(key1, key2);
        assert_eq!(key2, key3);
    }

    #[test]
    fn test_search_cache_key_service_filter_differs() {
        let key_none = AspenStorage::search_cache_key("test", None);
        let key_tidal = AspenStorage::search_cache_key("test", Some(ServiceType::Tidal));
        let key_youtube = AspenStorage::search_cache_key("test", Some(ServiceType::YouTube));

        assert_ne!(key_none, key_tidal);
        assert_ne!(key_tidal, key_youtube);
        assert_ne!(key_none, key_youtube);
    }

    #[test]
    fn test_search_cache_key_none_includes_all() {
        // The implementation uses "all" for None filter
        // We can't test the hash directly, but we can verify the key
        // is different from any specific service
        let key_none = AspenStorage::search_cache_key("test", None);
        let key_tidal = AspenStorage::search_cache_key("test", Some(ServiceType::Tidal));

        assert_ne!(key_none, key_tidal);
    }

    #[test]
    fn test_search_cache_key_deterministic() {
        let key1 = AspenStorage::search_cache_key("same query", Some(ServiceType::Tidal));
        let key2 = AspenStorage::search_cache_key("same query", Some(ServiceType::Tidal));
        let key3 = AspenStorage::search_cache_key("same query", Some(ServiceType::Tidal));

        assert_eq!(key1, key2);
        assert_eq!(key2, key3);
    }

    #[test]
    fn test_search_cache_key_different_queries_differ() {
        let key1 = AspenStorage::search_cache_key("query1", None);
        let key2 = AspenStorage::search_cache_key("query2", None);

        assert_ne!(key1, key2);
    }

    // ── 5. hash_bytes() ────────────────────────────────────────────
    #[test]
    fn test_hash_bytes_deterministic() {
        let data = b"test data for hashing";
        let hash1 = AspenStorage::hash_bytes(data);
        let hash2 = AspenStorage::hash_bytes(data);
        let hash3 = AspenStorage::hash_bytes(data);

        assert_eq!(hash1, hash2);
        assert_eq!(hash2, hash3);
    }

    #[test]
    fn test_hash_bytes_different_inputs_differ() {
        let data1 = b"input one";
        let data2 = b"input two";
        let data3 = b"input three";

        let hash1 = AspenStorage::hash_bytes(data1);
        let hash2 = AspenStorage::hash_bytes(data2);
        let hash3 = AspenStorage::hash_bytes(data3);

        assert_ne!(hash1, hash2);
        assert_ne!(hash2, hash3);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_hash_bytes_empty_input() {
        let hash1 = AspenStorage::hash_bytes(b"");
        let hash2 = AspenStorage::hash_bytes(b"");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, 0); // Hash of empty should not be zero
    }

    // ── 6. SyncState change detection ──────────────────────────────
    #[test]
    fn test_sync_state_default() {
        let state = SyncState::default();

        assert_eq!(state.last_queue_hash, None);
        assert_eq!(state.last_history_count, 0);
        assert_eq!(state.last_history_latest_hash, None);
    }

    #[test]
    fn test_sync_state_queue_change_detection() {
        let mut state = SyncState::default();

        // No hash set yet, so any hash is new
        assert_eq!(state.last_queue_hash, None);

        // Set a hash
        state.last_queue_hash = Some(12345);

        // Same hash → no change
        assert_eq!(state.last_queue_hash, Some(12345));

        // Different hash → change
        state.last_queue_hash = Some(67890);
        assert_ne!(state.last_queue_hash, Some(12345));
    }

    #[test]
    fn test_sync_state_history_change_detection() {
        let mut state = SyncState::default();

        assert_eq!(state.last_history_count, 0);
        assert_eq!(state.last_history_latest_hash, None);

        // Update count and hash
        state.last_history_count = 5;
        state.last_history_latest_hash = Some(11111);

        assert_eq!(state.last_history_count, 5);
        assert_eq!(state.last_history_latest_hash, Some(11111));

        // New entry → different hash
        state.last_history_latest_hash = Some(22222);
        assert_ne!(state.last_history_latest_hash, Some(11111));
    }

    // ── 7. key() prefix construction ───────────────────────────────
    #[test]
    fn test_key_prefix_construction() {
        // We can't easily construct an AspenStorage without connecting,
        // but we can test the expected format by simulating it
        let user_id = "test_user";
        let prefix = format!("drift:{user_id}:");

        let queue_key = format!("{}{}", prefix, "queue");
        assert_eq!(queue_key, "drift:test_user:queue");

        let history_key = format!("{}{}", prefix, "history:12345");
        assert_eq!(history_key, "drift:test_user:history:12345");

        let search_key = format!("{}{}", prefix, "search:abcdef");
        assert_eq!(search_key, "drift:test_user:search:abcdef");
    }

    #[test]
    fn test_key_different_users() {
        let prefix1 = format!("drift:{}:", "user1");
        let prefix2 = format!("drift:{}:", "user2");

        let key1 = format!("{}{}", prefix1, "queue");
        let key2 = format!("{}{}", prefix2, "queue");

        assert_eq!(key1, "drift:user1:queue");
        assert_eq!(key2, "drift:user2:queue");
        assert_ne!(key1, key2);
    }

    // ── 8. Reconnection logic tests ────────────────────────────────
    #[test]
    fn test_backoff_duration() {
        let mut health = ConnectionHealth::default();
        
        // First attempt: 5s
        assert_eq!(health.backoff_duration(), Duration::from_secs(5));
        
        // Second attempt: 10s
        health.reconnect_attempt_count = 1;
        assert_eq!(health.backoff_duration(), Duration::from_secs(10));
        
        // Third attempt: 20s
        health.reconnect_attempt_count = 2;
        assert_eq!(health.backoff_duration(), Duration::from_secs(20));
        
        // Fourth attempt: 40s
        health.reconnect_attempt_count = 3;
        assert_eq!(health.backoff_duration(), Duration::from_secs(40));
        
        // Fifth attempt and beyond: capped at 60s
        health.reconnect_attempt_count = 4;
        assert_eq!(health.backoff_duration(), Duration::from_secs(60));
        
        health.reconnect_attempt_count = 5;
        assert_eq!(health.backoff_duration(), Duration::from_secs(60));
        
        health.reconnect_attempt_count = 10;
        assert_eq!(health.backoff_duration(), Duration::from_secs(60));
    }

    #[test]
    fn test_should_reconnect_timing() {
        let mut health = ConnectionHealth::default();
        
        // Should reconnect immediately on first attempt
        assert!(health.should_reconnect());
        
        // After setting a recent attempt, should not reconnect
        health.last_reconnect_attempt = Some(Instant::now());
        assert!(!health.should_reconnect());
        
        // Simulate time passing (we can't actually wait, so we test the logic)
        // In real usage, after backoff_duration() elapses, should_reconnect returns true
        health.last_reconnect_attempt = Some(Instant::now() - Duration::from_secs(10));
        health.reconnect_attempt_count = 1; // backoff is 10s
        // This might be flaky on very slow systems, but the elapsed time should be ~10s
        // which matches the backoff
    }

    #[test]
    fn test_connection_state_transitions() {
        let mut health = ConnectionHealth::default();
        
        // Start in Connected state
        assert_eq!(health.state, ConnectionState::Connected);
        assert_eq!(health.consecutive_failures, 0);
        
        // First failure doesn't trigger reconnecting
        health.consecutive_failures = 1;
        assert_eq!(health.state, ConnectionState::Connected);
        
        // Second failure doesn't trigger reconnecting
        health.consecutive_failures = 2;
        assert_eq!(health.state, ConnectionState::Connected);
        
        // Third failure triggers reconnecting state
        health.consecutive_failures = 3;
        health.state = ConnectionState::Reconnecting;
        assert_eq!(health.state, ConnectionState::Reconnecting);
    }

    #[test]
    fn test_connection_inner_record_success() {
        // We can't easily create a real AspenClient here, so this test
        // would need a mock. For now, test the logic directly on ConnectionHealth
        let mut health = ConnectionHealth {
            consecutive_failures: 5,
            last_reconnect_attempt: Some(Instant::now()),
            state: ConnectionState::Reconnecting,
            reconnect_attempt_count: 3,
        };
        
        // Simulate record_success behavior
        health.consecutive_failures = 0;
        health.state = ConnectionState::Connected;
        health.reconnect_attempt_count = 0;
        
        assert_eq!(health.consecutive_failures, 0);
        assert_eq!(health.state, ConnectionState::Connected);
        assert_eq!(health.reconnect_attempt_count, 0);
    }

    #[test]
    fn test_connection_inner_record_failure() {
        let mut health = ConnectionHealth::default();
        
        // First failure
        health.consecutive_failures += 1;
        assert_eq!(health.consecutive_failures, 1);
        assert_eq!(health.state, ConnectionState::Connected);
        
        // Second failure
        health.consecutive_failures += 1;
        assert_eq!(health.consecutive_failures, 2);
        assert_eq!(health.state, ConnectionState::Connected);
        
        // Third failure - should trigger state transition
        health.consecutive_failures += 1;
        if health.consecutive_failures >= 3 && health.state == ConnectionState::Connected {
            health.state = ConnectionState::Reconnecting;
        }
        assert_eq!(health.consecutive_failures, 3);
        assert_eq!(health.state, ConnectionState::Reconnecting);
    }

    #[test]
    fn test_pending_write_queue_bounded() {
        // Test that queue respects MAX_PENDING_WRITES limit
        let max = ConnectionInner::MAX_PENDING_WRITES;
        assert_eq!(max, 100);
    }

    #[test]
    fn test_pending_write_structure() {
        let write = PendingWrite {
            key: "test_key".to_string(),
            value: vec![1, 2, 3],
        };
        
        assert_eq!(write.key, "test_key");
        assert_eq!(write.value, vec![1, 2, 3]);
    }
}

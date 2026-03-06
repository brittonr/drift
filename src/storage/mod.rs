//! Storage abstraction for Drift.
//!
//! **Architecture: local-first, multiplayer second.**
//!
//! - [`LocalStorage`]: redb + TOML + JSON — fast, always-available local persistence
//! - [`LocalFirstStorage`]: wraps LocalStorage + optional Aspen replication via WAL
//! - [`MetadataCache`]: redb-backed cache for service API responses (playlists, favorites)
//! - [`AspenStorage`]: Aspen distributed KV over iroh QUIC (used by replication task)
//!
//! All reads come from local storage. All writes go to local first, then queue
//! for background replication. Remote changes are merged using CRDT semantics.

pub mod local;
pub mod local_first;
pub mod merge;
pub mod metadata_cache;
pub mod wal;

#[cfg(feature = "aspen")]
pub mod aspen;

#[cfg(feature = "aspen")]
pub mod peers;

use anyhow::Result;
use async_trait::async_trait;

use crate::history_db::HistoryEntry;
use crate::queue_persistence::PersistedQueue;
use crate::search::SearchHistory;
use crate::service::{SearchResults, ServiceType, Track};

// Re-export playlist types from drift-plugin for convenience
pub use drift_plugin::playlist::{
    PlaylistIndex, PlaylistIndexEntry, PlaylistVisibility, SyncedPlaylist, SyncedTrackRef,
};

/// Reference to a blob in the distributed store.
#[derive(Debug, Clone)]
pub struct BlobRef {
    /// BLAKE3 hash (hex-encoded).
    pub hash: String,
    /// Size in bytes.
    pub size: u64,
    /// File format (e.g., "flac", "mp3").
    pub format: String,
}

/// A remote change detected by `poll_changes`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used only with aspen feature
pub enum SyncEvent {
    /// Queue was updated by another device.
    QueueChanged(PersistedQueue),
    /// History was updated by another device.
    HistoryChanged(Vec<HistoryEntry>),
    /// A playlist was created or updated (own or peer).
    PlaylistChanged { playlist_id: String },
    /// A playlist was deleted.
    PlaylistDeleted { playlist_id: String },
}

/// Core storage trait for all persistent drift data.
///
/// All methods are async to support both local (trivially wrapped) and
/// remote (Aspen RPC) backends.
#[async_trait]
pub trait DriftStorage: Send + Sync {
    /// Human-readable backend name (e.g., "local", "aspen").
    fn backend_name(&self) -> &str;

    // ── History ──────────────────────────────────────────────────────

    /// Record a track play. Implementations should dedup within ~10s.
    async fn record_play(&self, track: &Track) -> Result<()>;

    /// Get recent history entries, most-recent first.
    async fn get_history(&self, limit: usize) -> Result<Vec<HistoryEntry>>;

    // ── Queue ────────────────────────────────────────────────────────

    /// Save the current playback queue.
    async fn save_queue(&self, queue: &PersistedQueue) -> Result<()>;

    /// Load the saved queue. Returns None if nothing saved.
    async fn load_queue(&self) -> Result<Option<PersistedQueue>>;

    // ── Search Cache ────────────────────────────────────────────────

    /// Cache search results for a query.
    async fn cache_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
        results: &SearchResults,
    ) -> Result<()>;

    /// Retrieve cached search results. Returns None on miss/expiry.
    async fn get_cached_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
    ) -> Result<Option<SearchResults>>;

    // ── Search History ────────────────────────────────────────────

    /// Save search history.
    async fn save_search_history(&self, history: &SearchHistory) -> Result<()>;

    /// Load search history. Returns empty history on miss.
    async fn load_search_history(&self, max_size: usize) -> Result<SearchHistory>;

    // ── Blob Storage ────────────────────────────────────────────────

    /// Upload a downloaded file to the distributed blob store.
    ///
    /// Returns the BLAKE3 hash of the stored blob, or None if blob storage
    /// is unavailable (local backend, disconnected, etc.).
    async fn upload_blob(&self, _track_id: &str, _file_path: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Check if a track's audio file is available in the distributed blob store.
    ///
    /// Returns the BLAKE3 hash and file size if the track has been uploaded
    /// by any device in the cluster.
    async fn has_blob(&self, _track_id: &str) -> Result<Option<BlobRef>> {
        Ok(None)
    }

    /// Download a track's audio file from the distributed blob store.
    ///
    /// Returns the raw bytes of the file if found in the cluster.
    async fn fetch_blob(&self, _track_id: &str) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    // ── Playlists ─────────────────────────────────────────────────

    /// Save a synced playlist.
    async fn save_playlist(&self, _playlist: &SyncedPlaylist) -> Result<()> {
        Ok(())
    }

    /// Load a single playlist by ID.
    async fn load_playlist(&self, _playlist_id: &str) -> Result<Option<SyncedPlaylist>> {
        Ok(None)
    }

    /// List all own playlists (from the playlist index).
    async fn list_playlists(&self) -> Result<Vec<PlaylistIndexEntry>> {
        Ok(Vec::new())
    }

    /// Delete a playlist by ID.
    async fn delete_playlist(&self, _playlist_id: &str) -> Result<()> {
        Ok(())
    }

    // ── Peer Clusters ──────────────────────────────────────────

    /// List connected peer clusters.
    async fn list_peers(&self) -> Result<Vec<PeerInfo>> {
        Ok(Vec::new())
    }

    /// Get playlists from a specific peer (shared playlists only).
    async fn get_peer_playlists(&self, _peer_name: &str) -> Result<Vec<PlaylistIndexEntry>> {
        Ok(Vec::new())
    }

    /// Get a specific playlist from a peer.
    async fn get_peer_playlist(
        &self,
        _peer_name: &str,
        _playlist_id: &str,
    ) -> Result<Option<SyncedPlaylist>> {
        Ok(None)
    }

    // ── Sync ────────────────────────────────────────────────────────

    /// Poll for remote changes since last check.
    ///
    /// Called from the main event loop (~1s interval). Returns sync events
    /// for data changed by other devices. Local-only backends return empty.
    async fn poll_changes(&self) -> Result<Vec<SyncEvent>> {
        Ok(Vec::new())
    }
}

/// Info about a peer cluster.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub name: String,
    pub cluster_id: String,
    pub enabled: bool,
    pub sync_status: PeerSyncStatus,
}

/// Sync status of a peer cluster subscription.
#[derive(Debug, Clone)]
pub enum PeerSyncStatus {
    Synced,
    Syncing,
    Error(String),
    Disabled,
}

//! Storage abstraction for Drift.
//!
//! - [`LocalStorage`]: SQLite + TOML + JSON (default, fully offline)
//! - [`AspenStorage`]: Aspen distributed KV over iroh QUIC (multi-device sync)
//!
//! The `App` holds a `Box<dyn DriftStorage>` and all persistence goes through it.

pub mod local;

#[cfg(feature = "aspen")]
pub mod aspen;

use anyhow::Result;
use async_trait::async_trait;

use crate::history_db::HistoryEntry;
use crate::queue_persistence::PersistedQueue;
use crate::service::{SearchResults, ServiceType, Track};

/// A remote change detected by `poll_changes`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used only with aspen feature
pub enum SyncEvent {
    /// Queue was updated by another device.
    QueueChanged(PersistedQueue),
    /// History was updated by another device.
    HistoryChanged(Vec<HistoryEntry>),
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

    // ── Sync ────────────────────────────────────────────────────────

    /// Poll for remote changes since last check.
    ///
    /// Called from the main event loop (~1s interval). Returns sync events
    /// for data changed by other devices. Local-only backends return empty.
    async fn poll_changes(&self) -> Result<Vec<SyncEvent>> {
        Ok(Vec::new())
    }
}

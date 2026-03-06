//! Local storage backend — wraps redb, TOML, and JSON files.
//!
//! Preserves drift's original behavior with zero changes to the underlying
//! storage format. The async trait methods just lock and call through.

use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{DriftStorage, PlaylistIndex, PlaylistIndexEntry, SyncedPlaylist};
use crate::history_db::{HistoryDb, HistoryEntry};
use crate::queue_persistence::{self, PersistedQueue};
use crate::search::SearchHistory;
use crate::search_cache::SearchCache;
use crate::service::{SearchResults, ServiceType, Track};

use redb::{Database, ReadableTable, TableDefinition};

/// Redb table for synced playlists: key = playlist_id, value = JSON bytes.
const PLAYLISTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("playlists");
/// Redb table for playlist index: key = "index", value = JSON bytes.
const PLAYLIST_INDEX_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("playlist_index");

pub struct LocalStorage {
    history: Option<Mutex<HistoryDb>>,
    search_cache: Mutex<SearchCache>,
    /// Override for queue file path (None = default ~/.config/drift/queue.toml).
    queue_path: Option<std::path::PathBuf>,
    /// Playlist storage (redb).
    playlist_db: Option<Database>,
}

impl LocalStorage {
    pub fn new(cache_ttl_seconds: u64) -> Result<Self> {
        let history = match HistoryDb::new() {
            Ok(db) => Some(Mutex::new(db)),
            Err(e) => {
                tracing::warn!("Could not initialize history DB: {}", e);
                None
            }
        };
        let search_cache = SearchCache::new(cache_ttl_seconds)?;
        let playlist_db = match Self::open_playlist_db() {
            Ok(db) => Some(db),
            Err(e) => {
                tracing::warn!("Could not initialize playlist DB: {}", e);
                None
            }
        };
        Ok(Self {
            history,
            search_cache: Mutex::new(search_cache),
            queue_path: None,
            playlist_db,
        })
    }

    /// Create a LocalStorage backed by temp directories (for integration tests).
    ///
    /// Uses in-memory HistoryDb and temp dirs for search cache and queue,
    /// isolating tests from user data and from each other.
    #[doc(hidden)]
    pub fn new_for_test(cache_ttl_seconds: u64) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);

        let test_dir = std::env::temp_dir().join(format!(
            "drift-test-{}-{}",
            std::process::id(),
            n
        ));
        let history = HistoryDb::new_in_memory()?;
        let search_cache = SearchCache::new_in_dir(test_dir.join("search-cache"), cache_ttl_seconds)?;
        let playlist_db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .context("Failed to create in-memory playlist DB")?;
        Ok(Self {
            history: Some(Mutex::new(history)),
            search_cache: Mutex::new(search_cache),
            queue_path: Some(test_dir.join("queue.toml")),
            playlist_db: Some(playlist_db),
        })
    }

    /// Open the playlist redb database.
    fn open_playlist_db() -> Result<Database> {
        let data_dir = dirs::data_dir()
            .context("Failed to get data directory")?
            .join("drift");
        std::fs::create_dir_all(&data_dir)?;
        let db = Database::create(data_dir.join("playlists.redb"))
            .context("Failed to open playlist database")?;
        Ok(db)
    }

    /// Save a playlist to the local redb store and update the index.
    fn save_playlist_local(&self, playlist: &SyncedPlaylist) -> Result<()> {
        let db = self.playlist_db.as_ref()
            .context("Playlist DB not available")?;
        let json = serde_json::to_vec(playlist)?;

        let txn = db.begin_write()?;
        {
            let mut table = txn.open_table(PLAYLISTS_TABLE)?;
            table.insert(playlist.id.as_str(), json.as_slice())?;
        }
        txn.commit()?;

        // Update the index
        self.update_playlist_index(playlist)?;
        Ok(())
    }

    /// Load a playlist by ID from local redb.
    fn load_playlist_local(&self, playlist_id: &str) -> Result<Option<SyncedPlaylist>> {
        let db = self.playlist_db.as_ref()
            .context("Playlist DB not available")?;

        let txn = db.begin_read()?;
        let table = match txn.open_table(PLAYLISTS_TABLE) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        match table.get(playlist_id)? {
            Some(val) => {
                let playlist: SyncedPlaylist = serde_json::from_slice(val.value())?;
                Ok(Some(playlist))
            }
            None => Ok(None),
        }
    }

    /// List all playlists from the index.
    fn list_playlists_local(&self) -> Result<Vec<PlaylistIndexEntry>> {
        let db = self.playlist_db.as_ref()
            .context("Playlist DB not available")?;

        let txn = db.begin_read()?;
        let table = match txn.open_table(PLAYLIST_INDEX_TABLE) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        match table.get("index")? {
            Some(val) => {
                let index: PlaylistIndex = serde_json::from_slice(val.value())?;
                Ok(index.playlists)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Delete a playlist from local redb and update the index.
    fn delete_playlist_local(&self, playlist_id: &str) -> Result<()> {
        let db = self.playlist_db.as_ref()
            .context("Playlist DB not available")?;

        let txn = db.begin_write()?;
        {
            let mut table = txn.open_table(PLAYLISTS_TABLE)?;
            table.remove(playlist_id)?;
        }
        txn.commit()?;

        // Remove from index
        self.remove_from_playlist_index(playlist_id)?;
        Ok(())
    }

    /// Update the playlist index after a playlist save.
    fn update_playlist_index(&self, playlist: &SyncedPlaylist) -> Result<()> {
        let db = self.playlist_db.as_ref()
            .context("Playlist DB not available")?;

        let empty_index = || PlaylistIndex {
            playlists: Vec::new(),
            updated_at_ms: 0,
            lamport_clock: 0,
            device_id: String::new(),
        };

        let txn = db.begin_write()?;
        {
            let mut table = txn.open_table(PLAYLIST_INDEX_TABLE)?;

            // Read existing index, drop the guard before writing back
            let mut index: PlaylistIndex = table
                .get("index")?
                .map(|val| serde_json::from_slice(val.value()).unwrap_or_else(|_| empty_index()))
                .unwrap_or_else(empty_index);

            // Upsert entry
            let entry = PlaylistIndexEntry {
                id: playlist.id.clone(),
                title: playlist.title.clone(),
                track_count: playlist.tracks.len(),
                updated_at_ms: playlist.updated_at_ms,
                visibility: playlist.visibility,
            };

            if let Some(existing) = index.playlists.iter_mut().find(|e| e.id == playlist.id) {
                *existing = entry;
            } else {
                index.playlists.push(entry);
            }

            index.updated_at_ms = playlist.updated_at_ms;

            let json = serde_json::to_vec(&index)?;
            table.insert("index", json.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Remove a playlist from the index.
    fn remove_from_playlist_index(&self, playlist_id: &str) -> Result<()> {
        let db = self.playlist_db.as_ref()
            .context("Playlist DB not available")?;

        let txn = db.begin_write()?;
        {
            let mut table = txn.open_table(PLAYLIST_INDEX_TABLE)?;

            // Read the existing index, drop the guard, then write back
            let existing: Option<PlaylistIndex> = table
                .get("index")?
                .map(|val| serde_json::from_slice(val.value()))
                .transpose()?;

            if let Some(mut index) = existing {
                index.playlists.retain(|e| e.id != playlist_id);
                let json = serde_json::to_vec(&index)?;
                table.insert("index", json.as_slice())?;
            }
        }
        txn.commit()?;
        Ok(())
    }
}

#[async_trait]
impl DriftStorage for LocalStorage {
    fn backend_name(&self) -> &str {
        "local"
    }

    async fn record_play(&self, track: &Track) -> Result<()> {
        if let Some(ref h) = self.history {
            let db = h.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
            db.record_play(track)?;
        }
        Ok(())
    }

    async fn get_history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        if let Some(ref h) = self.history {
            let db = h.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
            Ok(db.get_recent(limit)?)
        } else {
            Ok(Vec::new())
        }
    }

    async fn save_queue(&self, queue: &PersistedQueue) -> Result<()> {
        match &self.queue_path {
            Some(path) => queue_persistence::save_queue_to(queue, path),
            None => queue_persistence::save_queue(queue),
        }
    }

    async fn load_queue(&self) -> Result<Option<PersistedQueue>> {
        match &self.queue_path {
            Some(path) => queue_persistence::load_queue_from(path),
            None => queue_persistence::load_queue(),
        }
    }

    async fn cache_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
        results: &SearchResults,
    ) -> Result<()> {
        let mut cache = self.search_cache.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        cache.insert(query, service_filter, results.clone());
        Ok(())
    }

    async fn get_cached_search(
        &self,
        query: &str,
        service_filter: Option<ServiceType>,
    ) -> Result<Option<SearchResults>> {
        let mut cache = self.search_cache.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        Ok(cache.get(query, service_filter))
    }

    async fn save_search_history(&self, history: &SearchHistory) -> Result<()> {
        history.save()
    }

    async fn load_search_history(&self, max_size: usize) -> Result<SearchHistory> {
        Ok(SearchHistory::load(max_size))
    }

    async fn save_playlist(&self, playlist: &SyncedPlaylist) -> Result<()> {
        self.save_playlist_local(playlist)
    }

    async fn load_playlist(&self, playlist_id: &str) -> Result<Option<SyncedPlaylist>> {
        self.load_playlist_local(playlist_id)
    }

    async fn list_playlists(&self) -> Result<Vec<PlaylistIndexEntry>> {
        self.list_playlists_local()
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.delete_playlist_local(playlist_id)
    }
}

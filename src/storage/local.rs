//! Local storage backend — wraps redb, TOML, and JSON files.
//!
//! Preserves drift's original behavior with zero changes to the underlying
//! storage format. The async trait methods just lock and call through.

use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;

use super::DriftStorage;
use crate::history_db::{HistoryDb, HistoryEntry};
use crate::queue_persistence::{self, PersistedQueue};
use crate::search::SearchHistory;
use crate::search_cache::SearchCache;
use crate::service::{SearchResults, ServiceType, Track};

pub struct LocalStorage {
    history: Option<Mutex<HistoryDb>>,
    search_cache: Mutex<SearchCache>,
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
        Ok(Self {
            history,
            search_cache: Mutex::new(search_cache),
        })
    }

    /// Create a LocalStorage backed by temp directories (for integration tests).
    ///
    /// Uses in-memory HistoryDb and a temp dir for search cache,
    /// isolating tests from user data.
    #[doc(hidden)]
    pub fn new_for_test(cache_ttl_seconds: u64) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);

        let history = HistoryDb::new_in_memory()?;
        let cache_dir = std::env::temp_dir().join(format!(
            "drift-search-cache-test-{}-{}",
            std::process::id(),
            n
        ));
        let search_cache = SearchCache::new_in_dir(cache_dir, cache_ttl_seconds)?;
        Ok(Self {
            history: Some(Mutex::new(history)),
            search_cache: Mutex::new(search_cache),
        })
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
        queue_persistence::save_queue(queue)
    }

    async fn load_queue(&self) -> Result<Option<PersistedQueue>> {
        queue_persistence::load_queue()
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
}

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::service::{SearchResults, ServiceType};

#[derive(Debug, Serialize, Deserialize)]
struct CachedSearchEntry {
    results: SearchResults,
    #[serde(with = "chrono::serde::ts_seconds")]
    cached_at: DateTime<Utc>,
}

pub struct SearchCache {
    cache_dir: PathBuf,
    entries: HashMap<String, CachedSearchEntry>,
    ttl_seconds: u64,
}

impl SearchCache {
    pub fn new(ttl_seconds: u64) -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .context("Failed to get cache directory")?
            .join("drift")
            .join("search");

        std::fs::create_dir_all(&cache_dir)
            .context("Failed to create search cache directory")?;

        Ok(Self {
            cache_dir,
            entries: HashMap::new(),
            ttl_seconds,
        })
    }

    pub fn get(
        &mut self,
        query: &str,
        service_filter: Option<ServiceType>,
    ) -> Option<SearchResults> {
        let key = Self::cache_key(query, service_filter);

        // Check memory cache first
        if let Some(entry) = self.entries.get(&key) {
            if self.is_valid(entry) {
                return Some(entry.results.clone());
            } else {
                // Expired - remove from memory
                self.entries.remove(&key);
            }
        }

        // Try loading from disk
        if let Some(entry) = self.load_from_disk(&key) {
            if self.is_valid(&entry) {
                let results = entry.results.clone();
                self.entries.insert(key, entry);
                return Some(results);
            } else {
                // Expired - remove from disk
                let _ = self.remove_from_disk(&key);
            }
        }

        None
    }

    pub fn insert(
        &mut self,
        query: &str,
        service_filter: Option<ServiceType>,
        results: SearchResults,
    ) {
        let key = Self::cache_key(query, service_filter);
        let entry = CachedSearchEntry {
            results,
            cached_at: Utc::now(),
        };

        // Save to disk (ignore errors - cache is best-effort)
        let _ = self.save_to_disk(&key, &entry);

        // Store in memory
        self.entries.insert(key, entry);
    }

    pub fn clear_expired(&mut self) {
        let expired_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| !self.is_valid(entry))
            .map(|(key, _)| key.clone())
            .collect();

        for key in expired_keys {
            self.entries.remove(&key);
            let _ = self.remove_from_disk(&key);
        }
    }

    fn is_valid(&self, entry: &CachedSearchEntry) -> bool {
        let age = Utc::now()
            .signed_duration_since(entry.cached_at)
            .num_seconds();
        age >= 0 && (age as u64) < self.ttl_seconds
    }

    fn cache_key(query: &str, service_filter: Option<ServiceType>) -> String {
        let normalized_query = query.trim().to_lowercase();
        let filter_str = service_filter
            .map(|s| s.to_string())
            .unwrap_or_else(|| "all".to_string());
        format!("{}_{}", normalized_query, filter_str)
    }

    fn disk_path(&self, key: &str) -> PathBuf {
        let hash = Self::hash_key(key);
        self.cache_dir.join(format!("{}.json", hash))
    }

    fn hash_key(key: &str) -> String {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    fn load_from_disk(&self, key: &str) -> Option<CachedSearchEntry> {
        let path = self.disk_path(key);
        if !path.exists() {
            return None;
        }

        let contents = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    fn save_to_disk(&self, key: &str, entry: &CachedSearchEntry) -> Result<()> {
        let path = self.disk_path(key);
        let contents = serde_json::to_string(entry)?;
        std::fs::write(&path, contents)?;
        Ok(())
    }

    fn remove_from_disk(&self, key: &str) -> Result<()> {
        let path = self.disk_path(key);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

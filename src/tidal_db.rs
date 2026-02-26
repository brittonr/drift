//! Content-addressed download history backed by redb.
//!
//! Shared library used by both the TUI (direct reads) and the tidal-dl
//! co-process binary (reads + writes). Two indices:
//!   tracks:       track_id    → {hash, path, artist, title}
//!   hashes:       blake3_hash → track_id
//!   albums:       album_id    → "complete"
//!   unavailable:  track_id    → unix timestamp (when we gave up)
//!
//! The TUI only needs read access — it checks whether a track has already
//! been downloaded by tidal-dl and where the file lives on disk.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};

pub const TRACKS: TableDefinition<&str, &[u8]> = TableDefinition::new("tracks");
#[allow(dead_code)]
pub const HASHES: TableDefinition<&str, &str> = TableDefinition::new("hashes");
#[allow(dead_code)]
pub const ALBUMS: TableDefinition<&str, &str> = TableDefinition::new("albums");
#[allow(dead_code)]
pub const UNAVAILABLE: TableDefinition<&str, u64> = TableDefinition::new("unavailable");

/// How long to cache an "unavailable" verdict before retrying (7 days).
pub const UNAVAILABLE_TTL_SECS: u64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRecord {
    pub hash: String,
    pub path: String,
    pub artist: String,
    pub title: String,
}

/// Handle to the tidal-dl redb download history.
/// The TUI opens read-only; the tidal-db binary uses create + write methods.
pub struct TidalDb {
    db: Database,
}

#[allow(dead_code)]
impl TidalDb {
    /// Open an existing redb database. Returns None if the file doesn't exist
    /// (tidal-dl hasn't been run yet).
    pub fn open(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let db = Database::open(path)
            .with_context(|| format!("failed to open tidal-dl redb at {}", path.display()))?;
        Ok(Some(Self { db }))
    }

    /// Open or create a redb database (for the tidal-db binary).
    pub fn create(path: &Path) -> Result<Self> {
        let db = Database::create(path)
            .with_context(|| format!("failed to create redb at {}", path.display()))?;
        // Ensure tables exist
        {
            let txn = db.begin_write()?;
            txn.open_table(TRACKS)?;
            txn.open_table(HASHES)?;
            txn.open_table(ALBUMS)?;
            txn.open_table(UNAVAILABLE)?;
            txn.commit()?;
        }
        Ok(Self { db })
    }

    /// Default redb path: ~/Music/Tidal/.tidal-dl.redb
    pub fn default_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join("Music").join("Tidal").join(".tidal-dl.redb"))
    }

    /// Check if a track is downloaded and its file still exists on disk.
    pub fn check(&self, track_id: &str) -> Result<Option<TrackRecord>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(TRACKS)?;
        match table.get(track_id)? {
            Some(data) => {
                let rec: TrackRecord = serde_json::from_slice(data.value())
                    .context("corrupt track record in redb")?;
                if Path::new(&rec.path).exists() {
                    Ok(Some(rec))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Batch check multiple track IDs. Returns the set of IDs that exist
    /// with valid files on disk.
    pub fn check_batch(&self, track_ids: &[&str]) -> Result<std::collections::HashSet<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(TRACKS)?;
        let mut found = std::collections::HashSet::new();
        for &tid in track_ids {
            if let Some(data) = table.get(tid)? {
                let rec: TrackRecord = serde_json::from_slice(data.value())?;
                if Path::new(&rec.path).exists() {
                    found.insert(tid.to_string());
                }
            }
        }
        Ok(found)
    }

    /// Get the local file path for a downloaded track, if it exists on disk.
    pub fn get_local_path(&self, track_id: &str) -> Result<Option<String>> {
        Ok(self.check(track_id)?.map(|r| r.path))
    }

    /// Check if a BLAKE3 content hash exists.
    pub fn check_hash(&self, hash: &str) -> Result<Option<(String, String)>> {
        let txn = self.db.begin_read()?;
        let hashes = txn.open_table(HASHES)?;
        match hashes.get(hash)? {
            Some(tid_guard) => {
                let track_id = tid_guard.value().to_string();
                let tracks = txn.open_table(TRACKS)?;
                match tracks.get(track_id.as_str())? {
                    Some(data) => {
                        let rec: TrackRecord = serde_json::from_slice(data.value())?;
                        if Path::new(&rec.path).exists() {
                            Ok(Some((track_id, rec.path)))
                        } else {
                            Ok(None)
                        }
                    }
                    None => Ok(None),
                }
            }
            None => Ok(None),
        }
    }

    /// Record a downloaded track.
    pub fn put(
        &self,
        track_id: &str,
        hash: &str,
        path: &str,
        artist: &str,
        title: &str,
    ) -> Result<()> {
        let rec = TrackRecord {
            hash: hash.to_string(),
            path: path.to_string(),
            artist: artist.to_string(),
            title: title.to_string(),
        };
        let data = serde_json::to_vec(&rec)?;
        let txn = self.db.begin_write()?;
        {
            let mut tracks = txn.open_table(TRACKS)?;
            tracks.insert(track_id, data.as_slice())?;
            let mut hashes = txn.open_table(HASHES)?;
            hashes.insert(hash, track_id)?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Check if an album is marked as fully downloaded.
    pub fn check_album(&self, album_id: &str) -> Result<bool> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(ALBUMS)?;
        Ok(table.get(album_id)?.is_some())
    }

    /// Mark an album as fully downloaded.
    pub fn mark_album(&self, album_id: &str) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(ALBUMS)?;
            table.insert(album_id, "complete")?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Check if a track is marked unavailable (and the TTL hasn't expired).
    pub fn is_unavailable(&self, track_id: &str) -> Result<bool> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(UNAVAILABLE)?;
        match table.get(track_id)? {
            Some(ts) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                Ok(now - ts.value() < UNAVAILABLE_TTL_SECS)
            }
            None => Ok(false),
        }
    }

    /// Batch check unavailable track IDs. Returns the set that are still
    /// within the TTL window.
    pub fn check_unavailable_batch(
        &self,
        track_ids: &[&str],
    ) -> Result<std::collections::HashSet<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(UNAVAILABLE)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut unavail = std::collections::HashSet::new();
        for &tid in track_ids {
            if let Some(ts) = table.get(tid)? {
                if now - ts.value() < UNAVAILABLE_TTL_SECS {
                    unavail.insert(tid.to_string());
                }
            }
        }
        Ok(unavail)
    }

    /// Mark a track as unavailable (403 after retries).
    pub fn mark_unavailable(&self, track_id: &str) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(UNAVAILABLE)?;
            table.insert(track_id, now)?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Clear a track's unavailable status (e.g., if it becomes available again).
    pub fn clear_unavailable(&self, track_id: &str) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(UNAVAILABLE)?;
            let _ = table.remove(track_id);
        }
        txn.commit()?;
        Ok(())
    }

    /// Count of tracks currently marked unavailable (within TTL).
    pub fn unavailable_count(&self) -> Result<u64> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(UNAVAILABLE)?;
        Ok(table.len()?)
    }

    /// Return total track count.
    pub fn track_count(&self) -> Result<u64> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(TRACKS)?;
        Ok(table.len()?)
    }

    /// Import from old JSON history file.
    pub fn import_json(&self, json_path: &str) -> Result<usize> {
        let content = std::fs::read_to_string(json_path)?;
        let history: std::collections::HashMap<String, serde_json::Value> =
            serde_json::from_str(&content)?;

        let txn = self.db.begin_write()?;
        let mut imported = 0usize;
        {
            let mut tracks = txn.open_table(TRACKS)?;
            let mut hashes = txn.open_table(HASHES)?;

            for (hash, info) in &history {
                let track_id = info["track_id"].as_str().unwrap_or("");
                let path = info["path"].as_str().unwrap_or("");
                let artist = info["artist"].as_str().unwrap_or("");
                let title = info["title"].as_str().unwrap_or("");

                if track_id.is_empty() || path.is_empty() || !Path::new(path).exists() {
                    continue;
                }

                let rec = TrackRecord {
                    hash: hash.clone(),
                    path: path.to_string(),
                    artist: artist.to_string(),
                    title: title.to_string(),
                };
                let data = serde_json::to_vec(&rec)?;
                tracks.insert(track_id, data.as_slice())?;
                hashes.insert(hash.as_str(), track_id)?;
                imported += 1;
            }
        }
        txn.commit()?;
        Ok(imported)
    }

    /// Remove entries where the file no longer exists on disk.
    pub fn prune(&self) -> Result<usize> {
        let stale: Vec<(String, String)> = {
            let txn = self.db.begin_read()?;
            let table = txn.open_table(TRACKS)?;
            let mut stale = Vec::new();
            for entry in table.iter()? {
                let (k, v) = entry?;
                let rec: TrackRecord = serde_json::from_slice(v.value())?;
                if !Path::new(&rec.path).exists() {
                    stale.push((k.value().to_string(), rec.hash));
                }
            }
            stale
        };

        let pruned = stale.len();
        if pruned > 0 {
            let txn = self.db.begin_write()?;
            {
                let mut tracks = txn.open_table(TRACKS)?;
                let mut hashes = txn.open_table(HASHES)?;
                for (id, hash) in &stale {
                    let _ = tracks.remove(id.as_str());
                    let _ = hashes.remove(hash.as_str());
                }
            }
            txn.commit()?;
        }
        Ok(pruned)
    }
}
